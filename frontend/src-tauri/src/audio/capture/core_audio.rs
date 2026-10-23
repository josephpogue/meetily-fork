// Core Audio implementation for macOS system audio capture

#[cfg(target_os = "macos")]
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use anyhow::Result;
use futures_util::Stream;
use ringbuf::{
    traits::{Consumer, Producer, Split},
    HeapCons, HeapProd, HeapRb,
};
use log::{error, info, warn};
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use crate::audio::call_apps::{self, CallAppId};

#[cfg(target_os = "macos")]
use cidre::{arc, av, cat, cf, core_audio as ca, os};

/// Waker state for async polling
struct WakerState {
    waker: Option<Waker>,
    has_data: bool,
}

/// Core Audio speaker input using aggregate device + tap
#[cfg(target_os = "macos")]
pub struct CoreAudioCapture {
    tap: Arc<Mutex<ca::TapGuard>>,
    agg_desc: arc::Retained<cf::DictionaryOf<cf::String, cf::Type>>,
    /// Set for a one-app tap: keeps the tap's process list current while recording.
    watcher: Option<TapWatcher>,
}

/// Core Audio stream that produces audio samples
#[cfg(target_os = "macos")]
pub struct CoreAudioStream {
    consumer: HeapCons<f32>,
    _device: ca::hardware::StartedDevice<ca::AggregateDevice>,
    _ctx: Box<AudioContext>,
    // Declared before `_tap`: the watcher thread stops before the tap is destroyed.
    _watcher: Option<TapWatcher>,
    _tap: Arc<Mutex<ca::TapGuard>>,
    waker_state: Arc<Mutex<WakerState>>,
    current_sample_rate: Arc<AtomicU32>,
}

/// Audio processing context
#[cfg(target_os = "macos")]
struct AudioContext {
    format: arc::R<av::AudioFormat>,
    producer: HeapProd<f32>,
    waker_state: Arc<Mutex<WakerState>>,
    current_sample_rate: Arc<AtomicU32>,
    consecutive_drops: Arc<AtomicU32>,
    should_terminate: Arc<AtomicBool>,
}

#[cfg(target_os = "macos")]
impl CoreAudioCapture {
    /// Create a new Core Audio capture for system audio
    pub fn new() -> Result<Self> {
        info!("🎙️ CoreAudio: Starting Core Audio capture initialization...");
        // Mono global tap, excluding no processes.
        // Note: Mono tap is more reliable for system audio capture on macOS
        info!("🎙️ CoreAudio: Creating process tap (global mono tap)...");
        let tap_desc = ca::TapDesc::with_mono_global_tap_excluding_processes(&cidre::ns::Array::new());
        Self::from_tap_desc(&tap_desc)
    }

    /// Tap only the audio processes of one call app (mono mixdown). An app with no Core Audio
    /// process yet gives a silent tap that picks its processes up when they appear.
    pub fn new_for_app(app: CallAppId) -> Result<Self> {
        let ids = call_apps::owned_object_ids(app)?;
        info!("System audio: {} only ({} processes)", app.label(), ids.len());
        let resolve = move || call_apps::owned_object_ids(app);
        Self::new_for_processes(&ids, Some(Box::new(resolve)))
    }

    /// Tap exactly these process objects. With a resolver, a background thread re-resolves the
    /// set about every 2 s and updates the running tap when it changes.
    pub(crate) fn new_for_processes(
        object_ids: &[u32],
        resolver: Option<Box<dyn Fn() -> Result<Vec<u32>> + Send>>,
    ) -> Result<Self> {
        let tap_desc = ca::TapDesc::with_mono_mixdown_of_processes(&process_array(object_ids));
        let mut capture = Self::from_tap_desc(&tap_desc)?;
        if let Some(resolver) = resolver {
            capture.watcher = Some(TapWatcher::start(
                capture.tap.clone(),
                object_ids.to_vec(),
                Duration::from_secs(2),
                resolver,
            ));
        }
        Ok(capture)
    }

    fn from_tap_desc(tap_desc: &ca::TapDesc) -> Result<Self> {

        // Note: Audio Capture permission (NSAudioCaptureUsageDescription) is required for macOS 14.4+
        // The permission dialog is automatically triggered when creating the Core Audio tap.
        // If permission is denied, the tap will return silence (all zeros).

        // Get default output device
        info!("🎙️ CoreAudio: Getting default output device...");
        let output_device = ca::System::default_output_device()
            .map_err(|e| {
                error!("❌ CoreAudio: Failed to get default output device: {:?}", e);
                anyhow::anyhow!("Failed to get default output device: {:?}", e)
            })?;

        info!("✅ CoreAudio: Got default output device");

        let output_uid = output_device.uid()
            .map_err(|e| {
                error!("❌ CoreAudio: Failed to get device UID: {:?}", e);
                anyhow::anyhow!("Failed to get device UID: {:?}", e)
            })?;

        // Get device name for better debugging
        let device_name = output_device.name().unwrap_or_else(|_| cf::String::from_str("Unknown"));
        info!("✅ CoreAudio: Default output device: '{}' (UID: {:?})", device_name, output_uid);

        // IMPORTANT: We do NOT create a sub_device dictionary here
        // When using a tap, the tap provides all the audio we need
        // Including both the tap AND the device creates duplicate audio (echo issue)

        let tap = tap_desc.create_process_tap()
            .map_err(|e| {
                error!("❌ CoreAudio: Failed to create process tap: {:?}", e);
                anyhow::anyhow!("Failed to create process tap: {:?}", e)
            })?;

        // Get tap information
        let tap_uid = tap.uid().unwrap_or_else(|_| cf::Uuid::new().to_cf_string());
        let tap_asbd = tap.asbd();

        match tap_asbd {
            Ok(asbd) => {
                info!("✅ CoreAudio: Process tap created - UID: {:?}", tap_uid);
                info!("📊 CoreAudio: Tap format - sample_rate: {} Hz, channels: {}",
                      asbd.sample_rate, asbd.channels_per_frame);
            }
            Err(e) => {
                warn!("⚠️ CoreAudio: Tap created but couldn't get format info: {:?}", e);
            }
        }

        // Create sub-tap dictionary
        let sub_tap = cf::DictionaryOf::with_keys_values(
            &[ca::sub_device_keys::uid()],
            &[tap.uid().unwrap().as_type_ref()],
        );

        // Create aggregate device descriptor
        // CRITICAL FIX: Use ONLY the tap, NOT the output device + tap
        // Previous configuration included both sub_device_list (output device) and tap_list (tap of same device)
        // This caused duplicate audio capture, resulting in echo (YouTube audio appeared twice)
        // The tap alone provides all the system audio we need
        let agg_desc = cf::DictionaryOf::with_keys_values(
            &[
                ca::aggregate_device_keys::is_private(),
                ca::aggregate_device_keys::is_stacked(),
                ca::aggregate_device_keys::tap_auto_start(),
                ca::aggregate_device_keys::name(),
                ca::aggregate_device_keys::main_sub_device(),
                ca::aggregate_device_keys::uid(),
                // REMOVED: sub_device_list (was causing duplicate audio)
                ca::aggregate_device_keys::tap_list(),
            ],
            &[
                cf::Boolean::value_true().as_type_ref(),
                cf::Boolean::value_false(),
                cf::Boolean::value_true(),
                cf::str!(c"meetily-audio-tap").as_type_ref(),
                &output_uid,
                &cf::Uuid::new().to_cf_string(),
                // REMOVED: sub_device array (was causing echo)
                &cf::ArrayOf::from_slice(&[sub_tap.as_ref()]),
            ],
        );

        info!("✅ CoreAudio: Aggregate device descriptor created");
        info!("✅ CoreAudio: Core Audio capture initialized successfully!");

        Ok(Self { tap: Arc::new(Mutex::new(tap)), agg_desc, watcher: None })
    }

    /// Start the audio device and create a stream
    fn start_device(
        &self,
        ctx: &mut Box<AudioContext>,
    ) -> Result<ca::hardware::StartedDevice<ca::AggregateDevice>> {
        extern "C" fn audio_proc(
            device: ca::Device,
            _now: &cat::AudioTimeStamp,
            input_data: &cat::AudioBufList<1>,
            _input_time: &cat::AudioTimeStamp,
            _output_data: &mut cat::AudioBufList<1>,
            _output_time: &cat::AudioTimeStamp,
            ctx: Option<&mut AudioContext>,
        ) -> os::Status {
            let ctx = ctx.unwrap();

            // Check for sample rate changes
            let after = device
                .nominal_sample_rate()
                .unwrap_or(ctx.format.absd().sample_rate) as u32;
            let before = ctx.current_sample_rate.load(Ordering::Acquire);

            if before != after {
                ctx.current_sample_rate.store(after, Ordering::Release);
            }

            // Try to get audio data from the buffer list
            if let Some(view) =
                av::AudioPcmBuf::with_buf_list_no_copy(&ctx.format, input_data, None)
            {
                if let Some(data) = view.data_f32_at(0) {
                    process_audio_data(ctx, data);
                }
            } else if ctx.format.common_format() == av::audio::CommonFormat::PcmF32 {
                // Fallback: manual extraction if AudioPcmBuf fails
                let first_buffer = &input_data.buffers[0];
                let byte_count = first_buffer.data_bytes_size as usize;
                let float_count = byte_count / std::mem::size_of::<f32>();

                if float_count > 0 && first_buffer.data != std::ptr::null_mut() {
                    let data = unsafe {
                        std::slice::from_raw_parts(first_buffer.data as *const f32, float_count)
                    };
                    process_audio_data(ctx, data);
                }
            }

            os::Status::NO_ERR
        }

        // Create aggregate device
        info!("🎙️ CoreAudio: Creating aggregate device...");
        let agg_device = ca::AggregateDevice::with_desc(&self.agg_desc)
            .map_err(|e| {
                error!("❌ CoreAudio: Failed to create aggregate device: {:?}", e);
                anyhow::anyhow!("Failed to create aggregate device: {:?}", e)
            })?;

        info!("✅ CoreAudio: Aggregate device created");

        // Create IO proc ID for audio processing
        info!("🎙️ CoreAudio: Creating IO proc...");
        let proc_id = agg_device.create_io_proc_id(audio_proc, Some(ctx))
            .map_err(|e| {
                error!("❌ CoreAudio: Failed to create IO proc: {:?}", e);
                anyhow::anyhow!("Failed to create IO proc: {:?}", e)
            })?;

        info!("✅ CoreAudio: IO proc created with ID: {:?}", proc_id);

        // Start the device
        info!("🎙️ CoreAudio: Starting audio device...");
        let started_device = ca::device_start(agg_device, Some(proc_id))
            .map_err(|e| {
                error!("❌ CoreAudio: Failed to start device: {:?}", e);
                anyhow::anyhow!("Failed to start device: {:?}", e)
            })?;

        info!("✅ CoreAudio: Audio device started successfully!");

        // Get device sample rate
        let device_ref = started_device.as_ref();
        let sample_rate = device_ref.nominal_sample_rate().unwrap_or(0.0);
        info!("📊 CoreAudio: Aggregate device sample_rate: {} Hz", sample_rate);

        Ok(started_device)
    }

    /// Create a stream from this capture
    pub fn stream(mut self) -> Result<CoreAudioStream> {
        info!("🎙️ CoreAudio: Creating CoreAudioStream...");

        // Get tap audio format
        let asbd = self.tap.lock().unwrap().asbd()
            .map_err(|e| {
                error!("❌ CoreAudio: Failed to get tap ASBD: {:?}", e);
                anyhow::anyhow!("Failed to get tap ASBD: {:?}", e)
            })?;

        let format = av::AudioFormat::with_asbd(&asbd)
            .ok_or_else(|| {
                error!("❌ CoreAudio: Failed to create audio format");
                anyhow::anyhow!("Failed to create audio format")
            })?;

        info!("✅ CoreAudio: Tap audio format: {} Hz, {} channels", asbd.sample_rate, asbd.channels_per_frame);

        // Create ring buffer for lock-free audio transfer
        let buffer_size = 1024 * 128; // 128KB buffer
        let rb = HeapRb::<f32>::new(buffer_size);
        let (producer, consumer) = rb.split();

        let waker_state = Arc::new(Mutex::new(WakerState {
            waker: None,
            has_data: false,
        }));

        let current_sample_rate = Arc::new(AtomicU32::new(asbd.sample_rate as u32));
        info!("✅ CoreAudio: Initial sample rate: {} Hz", asbd.sample_rate);

        let mut ctx = Box::new(AudioContext {
            format,
            producer,
            waker_state: waker_state.clone(),
            current_sample_rate: current_sample_rate.clone(),
            consecutive_drops: Arc::new(AtomicU32::new(0)),
            should_terminate: Arc::new(AtomicBool::new(false)),
        });

        info!("🎙️ CoreAudio: Starting audio device...");
        let device = self.start_device(&mut ctx)?;

        info!("✅ CoreAudio: CoreAudioStream created successfully!");

        Ok(CoreAudioStream {
            consumer,
            _device: device,
            _ctx: ctx,
            _watcher: self.watcher.take(),
            _tap: self.tap,
            waker_state,
            current_sample_rate,
        })
    }
}

/// Array of process object ids as `TapDesc` wants them.
#[cfg(target_os = "macos")]
fn process_array(object_ids: &[u32]) -> arc::R<cidre::ns::Array<cidre::ns::Number>> {
    let numbers: Vec<arc::R<cidre::ns::Number>> =
        object_ids.iter().map(|&id| cidre::ns::Number::with_u32(id)).collect();
    cidre::ns::Array::from_slice_retained(&numbers)
}

/// Keeps a one-app tap pointed at the app's current processes: when the app spawns a new audio
/// helper mid-call, the tap description is updated in place (kAudioTapPropertyDescription), so the
/// stream and the recording stay alive and the new process is heard within one interval.
#[cfg(target_os = "macos")]
pub(crate) struct TapWatcher {
    stop: std::sync::mpsc::Sender<()>,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(target_os = "macos")]
impl TapWatcher {
    pub(crate) fn start(
        tap: Arc<Mutex<ca::TapGuard>>,
        initial: Vec<u32>,
        interval: Duration,
        resolver: Box<dyn Fn() -> Result<Vec<u32>> + Send>,
    ) -> Self {
        let (stop, stop_rx) = std::sync::mpsc::channel::<()>();
        let handle = std::thread::Builder::new()
            .name("call-app-tap-watcher".into())
            .spawn(move || {
                let mut current = initial;
                // A message or a closed channel both mean stop; only a timeout is a tick.
                while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                    stop_rx.recv_timeout(interval)
                {
                    let latest = match resolver() {
                        Ok(ids) => ids,
                        Err(e) => {
                            warn!("CoreAudio: could not re-resolve app processes: {}", e);
                            continue;
                        }
                    };
                    if latest == current {
                        continue;
                    }
                    match update_tap_processes(&tap, &latest) {
                        Ok(()) => {
                            info!(
                                "CoreAudio: app tap now covers {} processes (was {})",
                                latest.len(),
                                current.len()
                            );
                            current = latest;
                        }
                        // Keep `current` so the next tick tries again.
                        Err(e) => warn!("CoreAudio: could not update app tap: {}", e),
                    }
                }
            })
            .expect("spawn call-app-tap-watcher");
        Self { stop, handle: Some(handle) }
    }
}

#[cfg(target_os = "macos")]
impl Drop for TapWatcher {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Point a running tap at a new set of process objects, keeping its uuid and stream alive.
#[cfg(target_os = "macos")]
pub(crate) fn update_tap_processes(tap: &Mutex<ca::TapGuard>, object_ids: &[u32]) -> Result<()> {
    let mut tap = tap.lock().unwrap();
    let mut desc = tap
        .desc()
        .map_err(|e| anyhow::anyhow!("Failed to read tap description: {:?}", e))?;
    desc.set_processes(&process_array(object_ids));
    tap.set_desc(desc)
        .map_err(|e| anyhow::anyhow!("Failed to set tap description: {:?}", e))
}

/// Process audio data from the IO proc callback
#[cfg(target_os = "macos")]
fn process_audio_data(ctx: &mut AudioContext, data: &[f32]) {
    // Push raw samples directly to ring buffer
    // Let the pipeline handle all gain adjustments (post-mix 3x gain + mic normalization)
    let buffer_size = data.len();
    let pushed = ctx.producer.push_slice(data);

    if pushed < buffer_size {
        let consecutive = ctx.consecutive_drops.fetch_add(1, Ordering::AcqRel) + 1;

        if consecutive > 10 {
            ctx.should_terminate.store(true, Ordering::Release);
            return;
        }
    } else {
        ctx.consecutive_drops.store(0, Ordering::Release);
    }

    if pushed > 0 {
        let should_wake = {
            let mut waker_state = ctx.waker_state.lock().unwrap();
            if !waker_state.has_data {
                waker_state.has_data = true;
                waker_state.waker.take()
            } else {
                None
            }
        };

        if let Some(waker) = should_wake {
            waker.wake();
        }
    }
}

#[cfg(target_os = "macos")]
impl CoreAudioStream {
    /// Get current sample rate
    pub fn sample_rate(&self) -> u32 {
        self.current_sample_rate.load(Ordering::Acquire)
    }
}

#[cfg(target_os = "macos")]
impl Stream for CoreAudioStream {
    type Item = f32;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        // Try to pop a sample from the ring buffer
        if let Some(sample) = self.consumer.try_pop() {
            return Poll::Ready(Some(sample));
        }

        // Check if we should terminate
        if self._ctx.should_terminate.load(Ordering::Acquire) {
            warn!("Stream terminating due to buffer pressure");
            return match self.consumer.try_pop() {
                Some(sample) => Poll::Ready(Some(sample)),
                None => Poll::Ready(None),
            };
        }

        // No data available, register waker and return pending
        {
            let mut state = self.waker_state.lock().unwrap();
            state.has_data = false;
            state.waker = Some(cx.waker().clone());
        }

        Poll::Pending
    }
}

#[cfg(target_os = "macos")]
impl Drop for CoreAudioStream {
    fn drop(&mut self) {
        info!("CoreAudioStream dropped, signaling termination");
        self._ctx.should_terminate.store(true, Ordering::Release);
    }
}

// Stub implementations for non-macOS platforms
#[cfg(not(target_os = "macos"))]
pub struct CoreAudioCapture;

#[cfg(not(target_os = "macos"))]
pub struct CoreAudioStream;

#[cfg(not(target_os = "macos"))]
impl CoreAudioCapture {
    pub fn new() -> Result<Self> {
        Err(anyhow::anyhow!("Core Audio is only supported on macOS"))
    }

    pub fn new_for_app(_app: crate::audio::call_apps::CallAppId) -> Result<Self> {
        Err(anyhow::anyhow!("Core Audio is only supported on macOS"))
    }

    pub fn stream(self) -> Result<CoreAudioStream> {
        Err(anyhow::anyhow!("Core Audio is only supported on macOS"))
    }
}

#[cfg(not(target_os = "macos"))]
impl CoreAudioStream {
    pub fn sample_rate(&self) -> u32 {
        0
    }
}

#[cfg(not(target_os = "macos"))]
impl Stream for CoreAudioStream {
    type Item = f32;

    fn poll_next(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        Poll::Ready(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[cfg(target_os = "macos")]
    #[ignore] // Only run manually as it requires audio hardware
    async fn test_core_audio_capture() {
        use futures_util::StreamExt;

        let capture = CoreAudioCapture::new().expect("Failed to create capture");
        let mut stream = capture.stream().expect("Failed to create stream");

        info!("Stream sample rate: {} Hz", stream.sample_rate());

        // Collect some samples
        let mut sample_count = 0;
        while sample_count < 48000 { // 1 second at 48kHz
            if let Some(_sample) = stream.next().await {
                sample_count += 1;
            }
        }

        info!("Collected {} samples", sample_count);
        assert!(sample_count >= 48000);
    }

    // ---- Live checks for the one-app tap (macOS, real audio hardware, quiet playback) ----

    #[cfg(target_os = "macos")]
    mod live {
        use super::super::*;
        use futures_util::StreamExt;
        use std::process::{Child, Command, Stdio};

        const DIR: &str = "/tmp/meeting-detect-app-audio";

        fn make_clip(name: &str, text: &str) -> String {
            std::fs::create_dir_all(DIR).unwrap();
            let aiff = format!("{DIR}/{name}.aiff");
            let wav = format!("{DIR}/{name}.wav");
            assert!(Command::new("say").args(["-o", &aiff, text]).status().unwrap().success());
            assert!(Command::new("afconvert")
                .args(["-f", "WAVE", "-d", "LEI16", &aiff, &wav])
                .status()
                .unwrap()
                .success());
            wav
        }

        /// Start a very quiet afplay and wait until Core Audio lists it as a process object.
        fn play(wav: &str) -> (Child, u32) {
            let child = Command::new("afplay")
                .args(["-v", "0.05", wav])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let pid = child.id() as i32;
            for _ in 0..60 {
                if let Ok(p) = ca::Process::with_pid(pid) {
                    if p.0 .0 != 0 {
                        return (child, p.0 .0);
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            panic!("afplay pid {pid} never appeared as a Core Audio process");
        }

        fn signal(child: &Child, sig: &str) {
            Command::new("kill").args([sig, &child.id().to_string()]).status().unwrap();
        }

        fn rms(samples: &[f32]) -> f32 {
            if samples.is_empty() {
                return 0.0;
            }
            (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
        }

        /// Poll the tap stream into a shared buffer until aborted.
        fn collect(mut stream: CoreAudioStream) -> (Arc<Mutex<Vec<f32>>>, tokio::task::JoinHandle<()>) {
            let buf = Arc::new(Mutex::new(Vec::new()));
            let out = buf.clone();
            let task = tokio::spawn(async move {
                while let Some(s) = stream.next().await {
                    out.lock().unwrap().push(s);
                }
            });
            (buf, task)
        }

        async fn pause(ms: u64) {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }

        #[tokio::test]
        #[ignore] // Live: plays two very quiet clips through the speakers
        async fn live_app_tap_hears_only_its_processes() {
            let text = "This is a steady test sentence for the process tap check. ".repeat(6);
            let (a, b) = (make_clip("a", &text), make_clip("b", &text));
            let (mut pa, id_a) = play(&a);
            signal(&pa, "-STOP");
            let (mut pb, id_b) = play(&b);
            signal(&pb, "-STOP");
            println!("afplay A object {id_a}, afplay B object {id_b}");

            // Tap only A's process object, through the constructor new_for_app uses.
            let cap = CoreAudioCapture::new_for_processes(&[id_a], None).unwrap();
            let stream = cap.stream().unwrap();
            let rate = stream.sample_rate() as usize;
            let (buf, task) = collect(stream);

            pause(500).await;
            signal(&pa, "-CONT"); // only A plays
            pause(3500).await;
            let a_end = buf.lock().unwrap().len();
            signal(&pa, "-STOP");
            pause(700).await; // let A's buffered tail drain
            let b_start = buf.lock().unwrap().len();
            signal(&pb, "-CONT"); // only B plays
            pause(3500).await;
            let b_end = buf.lock().unwrap().len();
            task.abort();
            let _ = (pa.kill(), pb.kill(), pa.wait(), pb.wait());

            let samples = buf.lock().unwrap().clone();
            let a_rms = rms(&samples[rate / 2..a_end]);
            let b_rms = rms(&samples[b_start..b_end]);
            println!("RMS while only A plays (tapped): {a_rms:.6}");
            println!("RMS while only B plays (not tapped): {b_rms:.6}");
            assert!(a_rms > 0.0005, "tapped app should be heard, rms {a_rms}");
            assert!(b_rms < a_rms / 20.0, "other app must not be heard: {b_rms} vs {a_rms}");
        }

        #[tokio::test]
        #[ignore] // Live: plays one very quiet clip through the speakers
        async fn live_app_tap_hears_process_added_after_start() {
            let text = "This is a steady test sentence for the late process check. ".repeat(8);
            let clip = make_clip("late", &text);
            let (mut p, id) = play(&clip);
            println!("afplay object {id}");

            // The tap starts with NO processes (the app had none yet); the resolver gains the
            // process later, as when Teams spawns an audio helper mid-call.
            let owned = Arc::new(Mutex::new(Vec::<u32>::new()));
            let owned_for_resolver = owned.clone();
            let cap = CoreAudioCapture::new_for_processes(
                &[],
                Some(Box::new(move || Ok(owned_for_resolver.lock().unwrap().clone()))),
            )
            .unwrap();
            let stream = cap.stream().unwrap();
            let rate = stream.sample_rate() as usize;
            let (buf, task) = collect(stream);

            pause(1500).await; // before any tick: the process is playing but not in the tap
            let before_end = buf.lock().unwrap().len();
            *owned.lock().unwrap() = vec![id];
            let added_at = std::time::Instant::now();
            pause(2500).await; // one 2 s tick plus settle
            let after_start = buf.lock().unwrap().len();
            pause(2500).await;
            let after_end = buf.lock().unwrap().len();
            task.abort();
            let _ = (p.kill(), p.wait());

            let samples = buf.lock().unwrap().clone();
            // An empty tap may deliver no samples at all; that counts as silence.
            println!("samples before the add: {before_end}, total: {}", samples.len());
            let before = if before_end > rate / 4 { rms(&samples[rate / 4..before_end]) } else { 0.0 };
            let after = rms(&samples[after_start..after_end]);
            println!("RMS before the process was added: {before:.6}");
            println!("RMS after the tap update ({:.1}s later+): {after:.6}", added_at.elapsed().as_secs_f32() - 2.5);
            assert!(after > 0.0005, "added process should be heard, rms {after}");
            assert!(before < after / 20.0, "process must not be heard before it is added: {before} vs {after}");
        }
    }
}