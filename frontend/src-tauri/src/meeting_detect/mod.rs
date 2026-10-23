// meeting_detect: one notification per call app (Zoom, Teams) when a call
// starts. A background task ticks every 2 s, gathers each app's signals
// (probe.rs), feeds the pure tracker (rules.rs) and turns its events into the
// `meeting-detected` / `meeting-detected-cleared` events and the native banner
// (notify.rs).

pub mod notify;
pub mod probe;
pub mod rules;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::audio::call_apps::CallAppId;
use rules::{Event, Rules, Tick};

const TICK: Duration = Duration::from_secs(2);

/// Tracker plus the clock its whole-second times come from.
pub struct DetectState {
    inner: Mutex<Rules>,
}

impl DetectState {
    /// Wall-clock seconds, so a 30-minute dismissal keeps counting while the Mac sleeps
    /// (a monotonic Instant stops during sleep on macOS).
    fn now(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
    }
}

#[derive(Serialize, Clone)]
pub struct DetectedPayload {
    app: &'static str,
    label: &'static str,
    title: Option<String>,
}

#[derive(Serialize, Clone)]
struct ClearedPayload {
    app: &'static str,
}

fn payload(app: CallAppId, title: Option<String>) -> DetectedPayload {
    DetectedPayload { app: app.as_str(), label: app.label(), title }
}

/// Starts detection. macOS only: elsewhere there is nothing to watch.
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    let state = Arc::new(DetectState { inner: Mutex::new(Rules::new()) });
    app.manage(state.clone());
    #[cfg(target_os = "macos")]
    {
        let handle = app.clone();
        notify::init(Box::new({
            let handle = handle.clone();
            let state = state.clone();
            move |key, response| on_banner(&handle, &state, key, response)
        }));
        tauri::async_runtime::spawn(run(handle, state));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = state;
}

/// What the user did with a banner.
#[cfg(target_os = "macos")]
fn on_banner<R: Runtime>(app: &AppHandle<R>, state: &Arc<DetectState>, key: &str, response: notify::Response) {
    let Some(id) = CallAppId::parse(key) else { return };
    match response {
        notify::Response::Dismiss => dismiss_app(app, state, id),
        notify::Response::Open => crate::tray::focus_main_window(app),
        notify::Response::Record => {
            // A stale banner must not start a recording: Record only counts while that app is
            // prompted right now and nothing is recording. Otherwise it just opens the window.
            let (app, state) = (app.clone(), state.clone());
            tauri::async_runtime::spawn(async move {
                let recording = crate::is_recording().await;
                let prompted = state.inner.lock().ok().map(|r| r.prompted()).unwrap_or_default();
                let title = prompted.into_iter().find(|(a, _)| *a == id).map(|(_, t)| t);
                crate::tray::focus_main_window(&app);
                if let (Some(title), false) = (title, recording) {
                    if let Err(e) = app.emit("meeting-detect-record", payload(id, title)) {
                        log::error!("meeting_detect: cannot emit meeting-detect-record: {}", e);
                    }
                }
            });
        }
    }
}

fn dismiss_app<R: Runtime>(app: &AppHandle<R>, state: &DetectState, id: CallAppId) {
    let event = match state.inner.lock() {
        Ok(mut r) => r.dismiss(id, state.now()),
        Err(_) => None,
    };
    if let Some(e) = event {
        apply(app, e);
    }
    notify::withdraw(id.as_str());
}

fn apply<R: Runtime>(app: &AppHandle<R>, event: Event) {
    match event {
        Event::Prompted { app: id, title } => {
            log::info!("meeting_detect: {} meeting started", id.label());
            let body = title.clone().unwrap_or_else(|| "Record it with meetily-fork".to_string());
            notify::show(id.as_str(), &format!("{} meeting started", id.label()), &body);
            if let Err(e) = app.emit("meeting-detected", payload(id, title)) {
                log::error!("meeting_detect: cannot emit meeting-detected: {}", e);
            }
        }
        Event::Cleared { app: id } => {
            log::info!("meeting_detect: {} meeting cleared", id.label());
            notify::withdraw(id.as_str());
            if let Err(e) = app.emit("meeting-detected-cleared", ClearedPayload { app: id.as_str() }) {
                log::error!("meeting_detect: cannot emit meeting-detected-cleared: {}", e);
            }
        }
    }
}

#[cfg(target_os = "macos")]
async fn run<R: Runtime>(app: AppHandle<R>, state: Arc<DetectState>) {
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Each failure is logged once until the next success.
    let (audio_logged, window_logged) = (AtomicBool::new(false), AtomicBool::new(false));
    let mut last_seen = String::new();
    loop {
        interval.tick().await;
        let recording = crate::is_recording().await;
        let view = match tokio::task::spawn_blocking(probe::audio_view).await {
            Ok(Ok(v)) => {
                audio_logged.store(false, Ordering::Relaxed);
                Some(v)
            }
            Ok(Err(e)) => {
                log_once(&audio_logged, &format!("meeting_detect: Core Audio snapshot failed: {}", e));
                None
            }
            Err(_) => None,
        };
        let mic = |id: CallAppId| view.as_ref().is_some_and(|v| v.mic(id));
        let (zoom_mic, teams_mic) = (mic(CallAppId::Zoom), mic(CallAppId::Teams));
        let needs = state.inner.lock().map(|r| r.needs_windows(zoom_mic, teams_mic)).unwrap_or(false);
        let mut front = None;
        let mut windows = None;
        if needs {
            front = probe::frontmost_bundle_id();
            match crate::assistant::vision::capture::list_windows().await {
                Ok(w) => {
                    window_logged.store(false, Ordering::Relaxed);
                    windows = Some(w);
                }
                Err(e) => log_once(&window_logged, &format!("meeting_detect: no window listing, so no prompts: {}", e)),
            }
        }
        let w = windows.as_deref();
        // The listing can take seconds: read the recording state again so a banner is never raised
        // just after a recording started.
        let recording = crate::is_recording().await;
        // A failed read is not a quiet tick: hold the last known state until the next good read.
        // It still applies a recording start, so a prompt never outlives one.
        if !readings_complete(view.is_some(), needs, windows.is_some()) {
            if recording {
                let events = state.inner.lock().map(|mut r| r.recording_started()).unwrap_or_default();
                for e in events {
                    apply(&app, e);
                }
            }
            continue;
        }
        let tick = Tick {
            now: state.now(),
            recording,
            zoom: probe::signals(CallAppId::Zoom, zoom_mic, front.as_deref(), w),
            teams: probe::signals(CallAppId::Teams, teams_mic, front.as_deref(), w),
        };
        // One line whenever what the probe sees changes, so a gap in a call can be read back from the log.
        let seen = format!(
            "recording={} windows={} zoom[mic={} win={} front={} procs={}] teams[mic={} win={} plain={} front={} procs={}]",
            recording,
            windows.as_ref().map_or("none".to_string(), |w| w.len().to_string()),
            tick.zoom.mic, tick.zoom.meeting_window, tick.zoom.frontmost,
            view.as_ref().map_or(0, |v| v.owned[0].len()),
            tick.teams.mic, tick.teams.meeting_window, tick.teams.plain_window, tick.teams.frontmost,
            view.as_ref().map_or(0, |v| v.owned[1].len()),
        );
        if last_seen != seen {
            log::info!("meeting_detect: signals {}", seen);
            last_seen = seen;
        }
        let events = {
            let Ok(mut rules) = state.inner.lock() else { continue };
            rules.tick(&tick)
        };
        for e in events {
            apply(&app, e);
        }
    }
}

/// Whether this tick's reads all succeeded: the audio snapshot always, the window list when it was asked for.
/// A tick with a failed read is skipped, so a join error, a Core Audio error or a window-listing timeout
/// cannot count as the quiet ticks that end a call.
fn readings_complete(audio_ok: bool, windows_needed: bool, windows_ok: bool) -> bool {
    audio_ok && (!windows_needed || windows_ok)
}

#[cfg(target_os = "macos")]
fn log_once(flag: &AtomicBool, msg: &str) {
    if !flag.swap(true, Ordering::Relaxed) {
        log::warn!("{}", msg);
    }
}

/// The in-app Dismiss: silences `app` for 30 minutes and clears its card.
#[tauri::command]
pub fn meeting_detect_dismiss(
    app: String,
    handle: AppHandle,
    state: State<'_, Arc<DetectState>>,
) -> Result<(), String> {
    let id = CallAppId::parse(&app).ok_or_else(|| format!("unknown call app: {}", app))?;
    dismiss_app(&handle, &state, id);
    Ok(())
}

/// Calls prompted right now, so a window that loads late still shows them.
#[tauri::command]
pub fn meeting_detect_current(state: State<'_, Arc<DetectState>>) -> Vec<DetectedPayload> {
    state
        .inner
        .lock()
        .map(|r| r.prompted().into_iter().map(|(a, t)| payload(a, t)).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::readings_complete;

    #[test]
    fn a_failed_read_skips_the_tick() {
        assert!(readings_complete(true, false, false), "no window list asked for, none missing");
        assert!(readings_complete(true, true, true));
        assert!(!readings_complete(false, false, false), "audio snapshot failed");
        assert!(!readings_complete(true, true, false), "window listing failed or timed out");
    }
}
