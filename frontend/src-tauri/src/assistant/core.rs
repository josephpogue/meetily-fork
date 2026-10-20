// assistant/core.rs
//
// The assistant's live session: transcript log, trigger engine, answer
// lanes, voice ask, note state. Recording and transcription never wait on,
// or fail because of, any of this: every entry point here locks its own
// state, does its own work, and reports failure through `last_error` or a
// note's `error` field rather than propagating into the audio path.
//
// Everything below `install` is a plain async function taking `&AssistantHandle`
// rather than a Tauri `State`, so it is callable directly from a unit test
// with no running app. The `#[tauri::command]` wrappers in `commands.rs` are
// thin one-line adapters over these.

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Listener, Manager, Wry};

use crate::audio::TranscriptUpdate;

use super::ask_lane::{AskConfig, AskHooks, AskLane, AsksOut, ChatExchange, ChatLog};
use super::lanes::{claude_runner, AnswerLanes, CardKind, CardOut, FrameFeed, LaneConfig, Runner};
use super::note::{self, NoteDraft, NOTE_PROMPT};
use super::settings::{self, AssistantSettings};
use super::transcript::{AssemblerOut, Speaker, TranscriptLog};
use super::trigger::{TriggerEngine, TriggerMode};
use super::vision::capture::{self, CaptureConfig, CaptureHandle};
use super::vision::FrameStore;
use super::voice_ask::{VoiceAsk, VoiceState};
use super::AssistantHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteState {
    Idle,
    Drafting,
    Ready,
    Saved,
    Failed,
}

impl NoteState {
    fn label(self) -> &'static str {
        match self {
            NoteState::Idle => "idle",
            NoteState::Drafting => "drafting",
            NoteState::Ready => "ready",
            NoteState::Saved => "saved",
            NoteState::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusOut {
    pub enabled: bool,
    pub session_open: bool,
    pub lanes_ready: bool,
    pub mode: String,
    pub listening: bool,
    pub claude_ok: bool,
    pub last_error: Option<String>,
    /// Bumped every time `last_error` is set to a new `Some(...)`, never on
    /// a clear or on an unrelated `emit_status()`. The panel keys its
    /// transient error display on this instead of the string value, so a
    /// second error with identical text (or a stale value re-forwarded by
    /// some other status change) is still distinguishable from silence.
    pub last_error_seq: u64,
}

#[derive(Debug, Clone, Serialize)]
struct VoiceOut {
    state: &'static str,
    heard: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NoteOut {
    state: &'static str,
    markdown: String,
    error: Option<String>,
}

pub struct AssistantCore {
    settings: AssistantSettings,
    transcript: TranscriptLog,
    trigger: TriggerEngine,
    lanes: AnswerLanes,
    /// Ask Next. Replaced with a lane on the real runner when a session opens.
    ask_lane: AskLane,
    /// Built once in `wire`, handed to every session's ask lane.
    ask_hooks: AskHooks,
    /// The brief as it stands in an open session. Edits land here and in
    /// both lane sets; `pending_brief` is only for before a session opens.
    brief: String,
    /// The startup context every lane's turn carries; read by the wrapped
    /// runner at call time. `startup` keeps the fixed files for re-resolving
    /// the project page when the brief changes.
    startup_ctx: super::startup_context::ContextHandle,
    startup: Option<super::startup_context::Startup>,
    voice: VoiceAsk,
    /// The open You-utterance's text at the moment voice-ask's hotkey was
    /// pressed, if any. Stripped as a prefix from transcript text fed to
    /// `voice` so speech from before the press does not bleed into the
    /// captured question.
    voice_baseline: String,

    app: Option<AppHandle<Wry>>,
    /// Built once in `install`; every settled card reaches the panel and,
    /// for an answer or a direct ask, appends to `qa_log` for the note.
    card_emit: super::lanes::EmitFn,
    qa_log: Arc<StdMutex<Vec<String>>>,
    /// What the assistant showed this session, kept for the meeting save.
    /// Reset when a session opens.
    history: super::history::HistoryHandle,
    /// Ask cards Joseph had with the assistant this session, for the ask
    /// lane's sub-lanes to hear once. Shared with every session's ask lane.
    chat_log: ChatLog,

    enabled: bool,
    session_open: bool,
    lanes_ready: bool,
    claude_ok: bool,
    last_error: Option<String>,
    last_error_seq: u64,
    mode: TriggerMode,
    listening: bool,

    /// Typed in the panel while no session is open; seeds the next session
    /// and is cleared once used. A closing session puts its brief back here,
    /// because the panel's box still shows it.
    pending_brief: Option<String>,
    /// Set by a `set_brief` made while no session is open, cleared when an
    /// open takes the brief. `open_lanes_with` reads it to tell "cleared in
    /// the gap" (`pending_brief` is None) from "no edit in the gap".
    brief_edited: bool,

    note_state: NoteState,
    note_draft: Option<NoteDraft>,
    note_markdown: String,
    note_error: Option<String>,
    session_transcript: String,

    /// Edge-triggered so `on_transcript_update`'s drop condition logs once
    /// per state change instead of once per transcript chunk.
    transcript_ingest_ok: bool,

    /// Frames of the Zoom window for this recording. Kept after the
    /// recording stops so `draft_note` can send them all; replaced at the
    /// next open, emptied and dropped when the assistant is turned off.
    frames: Option<FrameStore>,
    /// The running capture task, stopped with the recording.
    capture: Option<CaptureHandle>,
}

impl Default for AssistantCore {
    fn default() -> Self {
        Self {
            settings: AssistantSettings::default(),
            transcript: TranscriptLog::default(),
            trigger: TriggerEngine::new(|_, _| {}),
            // Replaced with a real binary-backed runner once a session opens;
            // an empty path just fails fast if ever invoked before that.
            lanes: AnswerLanes::new(claude_runner(PathBuf::new())),
            ask_lane: AskLane::new(claude_runner(PathBuf::new()), AskHooks::default()),
            ask_hooks: AskHooks::default(),
            brief: String::new(),
            startup_ctx: super::startup_context::new_handle(),
            startup: None,
            voice: VoiceAsk::new(|_| {}, |_| {}),
            voice_baseline: String::new(),
            app: None,
            card_emit: Arc::new(|_| {}),
            qa_log: Arc::new(StdMutex::new(Vec::new())),
            history: super::history::new_handle(),
            chat_log: Arc::new(StdMutex::new(Vec::new())),
            enabled: true,
            session_open: false,
            lanes_ready: false,
            claude_ok: false,
            last_error: None,
            last_error_seq: 0,
            mode: TriggerMode::Gated,
            listening: true,
            pending_brief: None,
            brief_edited: false,
            note_state: NoteState::Idle,
            note_draft: None,
            note_markdown: String::new(),
            note_error: None,
            session_transcript: String::new(),
            transcript_ingest_ok: false,
            frames: None,
            capture: None,
        }
    }
}

impl AssistantCore {
    /// Stops the capture task, if one runs. The frame store stays.
    fn stop_capture(&mut self) {
        if let Some(handle) = self.capture.take() {
            handle.stop();
        }
    }

    fn status_snapshot(&self) -> StatusOut {
        StatusOut {
            enabled: self.enabled,
            session_open: self.session_open,
            lanes_ready: self.lanes_ready,
            mode: mode_label(self.mode).to_string(),
            listening: self.listening,
            claude_ok: self.claude_ok,
            last_error: self.last_error.clone(),
            last_error_seq: self.last_error_seq,
        }
    }

    /// The single place `last_error` gets assigned. Bumps `last_error_seq`
    /// on a new `Some(...)`, leaves it alone on a clear -- callers just set
    /// the value they mean, without having to remember the counter exists.
    fn set_last_error(&mut self, err: Option<String>) {
        if err.is_some() {
            self.last_error_seq += 1;
        }
        self.last_error = err;
    }

    /// A fresh Ask lane, keeping only the brief it showed, and its empty
    /// snapshot sent to the panel. Called wherever the session transcript is
    /// cleared, so a meeting never inherits the last one's questions.
    /// The collector the meeting save reads.
    pub fn history_handle(&self) -> super::history::HistoryHandle {
        self.history.clone()
    }

    fn reset_ask_lane(&mut self) {
        let brief = self.ask_lane.snapshot().brief;
        self.ask_lane.close();
        self.ask_lane = AskLane::new(claude_runner(PathBuf::new()), self.ask_hooks.clone());
        if !brief.is_empty() {
            self.ask_lane.set_brief(brief);
        }
        (self.ask_hooks.emit)(self.ask_lane.snapshot());
    }

    fn emit_status(&self) {
        let Some(app) = &self.app else { return };
        if let Err(e) = app.emit("assistant-status", &self.status_snapshot()) {
            log::warn!("assistant: failed to emit assistant-status: {}", e);
        }
    }

    fn emit_note(&self) {
        let Some(app) = &self.app else { return };
        let out = NoteOut {
            state: self.note_state.label(),
            markdown: self.note_markdown.clone(),
            error: self.note_error.clone(),
        };
        if let Err(e) = app.emit("assistant-note", &out) {
            log::warn!("assistant: failed to emit assistant-note: {}", e);
        }
    }
}

fn mode_label(mode: TriggerMode) -> &'static str {
    match mode {
        TriggerMode::Manual => "manual",
        TriggerMode::Gated => "gated",
        TriggerMode::Continuous => "continuous",
    }
}

fn parse_mode(raw: &str) -> Result<TriggerMode, String> {
    match raw {
        "manual" => Ok(TriggerMode::Manual),
        "gated" => Ok(TriggerMode::Gated),
        "continuous" => Ok(TriggerMode::Continuous),
        other => Err(format!("unknown assistant mode: {}", other)),
    }
}

fn split_names(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Empty means `~/brain/wiki`, resolved here rather than at save time so a
/// missing home directory fails the same way settings validation would.
fn resolve_vault_root(raw: &str) -> PathBuf {
    if raw.trim().is_empty() {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join("brain")
            .join("wiki")
    } else {
        PathBuf::from(raw.trim())
    }
}

/// Empty means `[~/brain]`, matching `AssistantSettings::deep_read_dirs`'s
/// doc comment.
fn resolve_deep_read_dirs(raw: &str) -> Vec<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        return vec![home.join("brain")];
    }
    trimmed
        .split(',')
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

/// Builds the shared card emitter once, at `install` time. A settled answer
/// or ask card also appends to `qa_log` so the end-of-meeting note has a
/// record of what was asked and answered; a drafting card, explain, or
/// catch-up does not. `forward` then passes the card on (to the panel).
fn make_card_emit(
    handle: AssistantHandle,
    qa_log: Arc<StdMutex<Vec<String>>>,
    chat_log: ChatLog,
    history: super::history::HistoryHandle,
    forward: super::lanes::EmitFn,
) -> super::lanes::EmitFn {
    Arc::new(move |card: CardOut| {
        record_card(&card, &qa_log, &chat_log);
        super::history::record_card(&history, &card);
        if is_settled_ask(&card) {
            // A question worked out in Ask may have answered one on the
            // list: do not wait for the next utterance. Spawned, because the
            // card can be emitted while the core lock is held.
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                let core = handle.0.lock().await;
                if core.session_open && core.enabled {
                    core.ask_lane.request(&core.transcript, false);
                }
            });
        }
        forward(card);
    })
}

fn is_settled_ask(card: &CardOut) -> bool {
    matches!(card.kind, CardKind::Ask) && (card.phase == "checked" || card.phase == "corrected")
}

/// What a settled card leaves behind. An answer or an Ask card goes to the
/// note's Q&A log; an Ask card (Joseph's own question) also goes to the chat
/// log, so Ask next can hear it and mark a list question answered.
fn record_card(card: &CardOut, qa_log: &StdMutex<Vec<String>>, chat_log: &StdMutex<Vec<ChatExchange>>) {
    let settled = card.phase == "checked" || card.phase == "corrected";
    if !settled {
        return;
    }
    if matches!(card.kind, CardKind::Answer | CardKind::Ask) {
        qa_log
            .lock()
            .unwrap()
            .push(format!("Q: {}\nA: {}", card.question, card.lead));
    }
    if matches!(card.kind, CardKind::Ask) {
        let mut answer = card.lead.clone();
        for bullet in &card.bullets {
            answer.push_str("\n- ");
            answer.push_str(bullet);
        }
        chat_log.lock().unwrap().push(ChatExchange { question: card.question.clone(), answer });
    }
}

fn make_voice_publish(app: AppHandle<Wry>) -> Arc<dyn Fn(VoiceState) + Send + Sync> {
    Arc::new(move |state: VoiceState| {
        let (label, heard) = match state {
            VoiceState::Off => ("off", String::new()),
            VoiceState::Listening { heard } => ("listening", heard),
            VoiceState::Submitting { question } => ("submitting", question),
        };
        if let Err(e) = app.emit("assistant-voice", &VoiceOut { state: label, heard }) {
            log::warn!("assistant: failed to emit assistant-voice: {}", e);
        }
    })
}

// ---------------------------------------------------------------------
// Entry points. Each locks AssistantCore, does its work, and returns; the
// real claude turns run on tasks spawned inside AnswerLanes/TriggerEngine/
// VoiceAsk themselves.
// ---------------------------------------------------------------------

/// Ask lane hooks: every list change goes out as `assistant-asks`; a
/// coalesced follow-up and a failure both need core, so they hop back in
/// through a spawned task.
fn make_ask_hooks(app: AppHandle<Wry>, handle: AssistantHandle, history: super::history::HistoryHandle) -> AskHooks {
    let rerun_handle = handle.clone();
    let error_handle = handle;
    AskHooks {
        emit: Arc::new(move |out: AsksOut| {
            super::history::record_asks(&history, &out);
            if let Err(e) = app.emit("assistant-asks", &out) {
                log::warn!("assistant: failed to emit assistant-asks: {}", e);
            }
        }),
        rerun: Arc::new(move || {
            let handle = rerun_handle.clone();
            tauri::async_runtime::spawn(async move {
                rerun_asks(&handle).await;
            });
        }),
        on_error: Arc::new(move |msg: String| {
            let handle = error_handle.clone();
            tauri::async_runtime::spawn(async move {
                let mut core = handle.0.lock().await;
                core.set_last_error(Some(msg));
                core.emit_status();
            });
        }),
    }
}

pub async fn get_state(handle: &AssistantHandle) -> StatusOut {
    handle.0.lock().await.status_snapshot()
}

pub async fn set_enabled(handle: &AssistantHandle, enabled: bool) {
    let (already_open, app) = {
        let mut core = handle.0.lock().await;
        core.enabled = enabled;
        if !enabled {
            core.lanes.close();
            core.ask_lane.close();
            // Switch off means no capture task and an empty store.
            core.stop_capture();
            if let Some(store) = core.frames.take() {
                store.clear();
            }
            core.lanes.set_frames(None);
            core.ask_lane.set_frames(None);
            if core.session_open {
                core.pending_brief = Some(core.brief.clone()).filter(|b| !b.trim().is_empty());
            }
            core.voice.cancel();
            // Otherwise every other path (`ask`/`explain`/`catchup`,
            // the panel's own `actionsDisabled`) disagrees with reality:
            // lanes are closed but session_open still claims a live
            // session, and explain/catchup have no `enabled` guard of
            // their own to catch it.
            core.session_open = false;
        }
        core.emit_status();
        (core.session_open, core.app.clone())
    };

    // The panel switch is the user's explicit intent, not a transient
    // runtime toggle -- persist it so the next recording's `open_session`
    // (which reloads settings fresh from the DB) doesn't read a stale row
    // and silently revert the choice just made.
    if let Some(state) = app.as_ref().and_then(|a| a.try_state::<crate::state::AppState>()) {
        if let Err(e) = settings::AssistantSettings::persist_enabled(state.db_manager.pool(), enabled).await {
            log::warn!("assistant: failed to persist enabled={}: {}", enabled, e);
        }
    }

    if !enabled || already_open {
        return;
    }
    // The session only opens on `recording-started`. If a recording was
    // already running when the assistant got turned on here, that event
    // already fired and will not fire again -- without this, the
    // assistant stays closed for the rest of the recording no matter how
    // many times the switch is flipped.
    if crate::audio::recording_commands::is_recording().await {
        log::info!("assistant: enabled while a recording is already running, opening the session now");
        open_session(handle, Some(true)).await;
    }
}

/// Shared by `set_mode` (explicit, from the settings UI) and `cycle_mode`
/// (the Option-M hotkey): applies a mode, rebuilds the trigger engine's
/// config from it, and emits status. Caller already holds the lock.
fn apply_mode(core: &mut AssistantCore, mode: TriggerMode) {
    core.mode = mode;
    let names = split_names(&core.settings.names);
    let quiet_gap = core.settings.quiet_gap_secs;
    let listening = core.listening;
    core.trigger.update(mode, quiet_gap, listening, names);
    core.emit_status();
}

pub async fn set_mode(handle: &AssistantHandle, mode: &str) -> Result<(), String> {
    let parsed = parse_mode(mode)?;
    let mut core = handle.0.lock().await;
    apply_mode(&mut core, parsed);
    Ok(())
}

/// Option-M: manual -> gated -> continuous -> manual.
pub async fn cycle_mode(handle: &AssistantHandle) {
    let mut core = handle.0.lock().await;
    let next = match core.mode {
        TriggerMode::Manual => TriggerMode::Gated,
        TriggerMode::Gated => TriggerMode::Continuous,
        TriggerMode::Continuous => TriggerMode::Manual,
    };
    apply_mode(&mut core, next);
}

pub async fn set_listening(handle: &AssistantHandle, listening: bool) {
    let mut core = handle.0.lock().await;
    core.listening = listening;
    let names = split_names(&core.settings.names);
    let mode = core.mode;
    let quiet_gap = core.settings.quiet_gap_secs;
    core.trigger.update(mode, quiet_gap, listening, names);
    core.emit_status();
}

/// Before a session: held for the next one. During a session: replaces the
/// brief everywhere at once and asks for a fresh list.
pub async fn set_brief(handle: &AssistantHandle, text: String) {
    let mut core = handle.0.lock().await;
    if !core.session_open {
        // Keep the panel's copy current before and after a recording too.
        core.ask_lane.set_brief(text.clone());
        core.pending_brief = if text.trim().is_empty() { None } else { Some(text) };
        core.brief_edited = true;
        return;
    }
    if core.brief == text {
        return;
    }
    core.brief = text.clone();
    core.lanes.set_brief(text.clone());
    core.ask_lane.set_brief(text.clone());
    if core.enabled {
        core.ask_lane.request(&core.transcript, false);
    }
    drop(core);
    refresh_startup(handle, text).await;
}

/// Re-resolves the project page for `brief` and updates the startup context
/// handle. The page is read on a blocking thread with no lock held; if the
/// brief changed again meanwhile, the later call wins and this one drops its
/// result.
async fn refresh_startup(handle: &AssistantHandle, brief: String) {
    let startup = handle.0.lock().await.startup.clone();
    let Some(startup) = startup else { return };
    let b = brief.clone();
    let Ok((ctx, slug)) = tokio::task::spawn_blocking(move || startup.render(&b)).await else {
        return;
    };
    let core = handle.0.lock().await;
    if core.brief != brief {
        return;
    }
    super::startup_context::log_built(&ctx, slug.as_deref(), "re-resolved on brief change");
    super::startup_context::set_context(&core.startup_ctx, &ctx);
}

/// The Refresh button and Option-Q. Runs even when nothing changed and
/// even while triggers are paused, because Joseph asked for it.
pub async fn refresh_asks(handle: &AssistantHandle) {
    let mut core = handle.0.lock().await;
    if !core.session_open {
        drop_closed_session(&mut core, "refresh asks");
        return;
    }
    if !core.enabled {
        drop_disabled(&mut core, "refresh asks");
        return;
    }
    core.ask_lane.request(&core.transcript, true);
}

/// The follow-up an ask turn fires when requests landed while it ran.
async fn rerun_asks(handle: &AssistantHandle) {
    let core = handle.0.lock().await;
    if !core.session_open || !core.enabled {
        return;
    }
    // Fires only the sub-lanes that owe a follow-up. Their dirty flag
    // includes manual refreshes even with no new transcript.
    core.ask_lane.rerun(&core.transcript);
}

pub async fn dismiss_ask(handle: &AssistantHandle, id: String) {
    let core = handle.0.lock().await;
    if !core.ask_lane.dismiss(&id) {
        return;
    }
    if core.session_open && core.enabled {
        core.ask_lane.request(&core.transcript, false);
    }
}

/// Joseph typed the answer to a list question. The item moves to the
/// answered list; the brief is the frontend's to update. False for an
/// unknown id or an empty answer.
pub async fn answer_ask(handle: &AssistantHandle, id: String, answer: String) -> bool {
    let core = handle.0.lock().await;
    if !core.ask_lane.answer(&id, &answer) {
        return false;
    }
    if core.session_open && core.enabled {
        core.ask_lane.request(&core.transcript, false);
    }
    true
}

/// Surfaces "the session is not open" as a visible error rather than a
/// silent no-op. The panel renders `lastError`, so a button press that gets
/// dropped needs to reach it the same way a lane failure does -- otherwise
/// pressing Explain, Catch up, or Ask while closed just looks like nothing
/// happened.
fn drop_closed_session(core: &mut AssistantCore, action: &str) {
    log::info!("assistant: {} dropped, no open session", action);
    core.set_last_error(Some("Assistant session is not open yet.".to_string()));
    core.emit_status();
}

/// Same shape as `drop_closed_session`, for the other reason a press can be
/// dropped: the session is open but the assistant itself is off. Every
/// entry point checks `session_open` before `enabled` -- a closed session
/// gets the closed-session message even if the assistant also happens to
/// be off, since that is the more specific and more actionable of the two.
fn drop_disabled(core: &mut AssistantCore, action: &str) {
    log::debug!("assistant: {} dropped, assistant disabled", action);
    core.set_last_error(Some("Assistant is turned off.".to_string()));
    core.emit_status();
}

/// Fires on a trigger, a typed ask, and a submitted voice ask alike. `extra`
/// reaches the model's prompt only, never the card's displayed question.
pub async fn ask(handle: &AssistantHandle, question: String, kind: CardKind, extra: Option<&str>) {
    let mut core = handle.0.lock().await;
    if !core.session_open {
        drop_closed_session(&mut core, "ask");
        return;
    }
    if !core.enabled {
        drop_disabled(&mut core, "ask");
        return;
    }
    let emit = core.card_emit.clone();
    let AssistantCore { lanes, transcript, .. } = &mut *core;
    lanes.answer_with_note(question, extra, kind, transcript, emit);
}

pub async fn explain(handle: &AssistantHandle) {
    let mut core = handle.0.lock().await;
    if !core.session_open {
        drop_closed_session(&mut core, "explain");
        return;
    }
    if !core.enabled {
        drop_disabled(&mut core, "explain");
        return;
    }
    let emit = core.card_emit.clone();
    let AssistantCore { lanes, transcript, .. } = &mut *core;
    let result = lanes.explain(transcript, emit);
    if let Err(e) = result {
        core.set_last_error(Some(e));
        core.emit_status();
    }
}

pub async fn catchup(handle: &AssistantHandle) {
    let mut core = handle.0.lock().await;
    if !core.session_open {
        drop_closed_session(&mut core, "catch-up");
        return;
    }
    if !core.enabled {
        drop_disabled(&mut core, "catch-up");
        return;
    }
    let emit = core.card_emit.clone();
    let AssistantCore { lanes, transcript, .. } = &mut *core;
    lanes.catchup(transcript, emit);
}

pub async fn voice_start(handle: &AssistantHandle) {
    let mut core = handle.0.lock().await;
    core.voice_baseline = core.transcript.open_you_preview();
    core.voice.start();
}

pub async fn voice_finish(handle: &AssistantHandle) {
    handle.0.lock().await.voice.finish();
}

pub async fn voice_cancel(handle: &AssistantHandle) {
    handle.0.lock().await.voice.cancel();
}

/// Option-A: start if idle, finish if capturing. Checked and acted on under
/// one lock so two presses close together can't both see "idle".
pub async fn voice_toggle(handle: &AssistantHandle) {
    let mut core = handle.0.lock().await;
    if core.voice.is_capturing() {
        core.voice.finish();
    } else {
        core.voice_baseline = core.transcript.open_you_preview();
        core.voice.start();
    }
}

pub async fn draft_note(handle: &AssistantHandle) -> Result<(), String> {
    let (transcript_text, qa_log_text) = {
        let mut core = handle.0.lock().await;
        if core.session_open {
            return Err("stop the recording before drafting a note".to_string());
        }
        core.note_state = NoteState::Drafting;
        core.note_error = None;
        core.emit_note();
        let transcript_text = core.transcript.all();
        let qa_log_text = core.qa_log.lock().unwrap().join("\n\n");
        let open_asks = core.ask_lane.open_asks();
        let mut draft_input = qa_log_text;
        let answered = core.ask_lane.answered_items();
        if !answered.is_empty() {
            let list = answered
                .iter()
                .map(|a| format!("- {} Answer: {}", a.question, a.answer))
                .collect::<Vec<_>>()
                .join("\n");
            draft_input = format!("{}\n\nQUESTIONS ANSWERED DURING THE CALL:\n{}", draft_input, list);
        }
        if !open_asks.is_empty() {
            let list = open_asks
                .iter()
                .map(|q| format!("- {}", q))
                .collect::<Vec<_>>()
                .join("\n");
            draft_input = format!("{}\n\nSUGGESTED QUESTIONS STILL OPEN AT THE END:\n{}", draft_input, list);
        }
        core.session_transcript = transcript_text.clone();
        (transcript_text, draft_input)
    };

    let result = {
        let mut core = handle.0.lock().await;
        core.lanes.draft_note(transcript_text, qa_log_text).await
    };

    let mut core = handle.0.lock().await;
    match result {
        Ok(raw) => match note::parse_note(&raw) {
            Ok(mut draft) => {
                // Built in code, not by the model: the answered list as it
                // stood, in the preview and in the saved file alike.
                draft.markdown = note::with_answered(&draft.markdown, &core.ask_lane.answered_items());
                core.note_markdown = draft.markdown.clone();
                core.note_draft = Some(draft);
                core.note_state = NoteState::Ready;
                core.note_error = None;
            }
            Err(e) => {
                core.note_state = NoteState::Failed;
                core.note_error = Some(e);
            }
        },
        Err(e) => {
            core.note_state = NoteState::Failed;
            core.note_error = Some(e);
        }
    }
    core.emit_note();
    Ok(())
}

pub async fn save_note(handle: &AssistantHandle) {
    let mut core = handle.0.lock().await;
    let Some(draft) = core.note_draft.clone() else {
        core.note_state = NoteState::Failed;
        core.note_error = Some("no note draft to save".to_string());
        core.emit_note();
        return;
    };
    let vault_root = resolve_vault_root(&core.settings.vault_root);
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let dry_run = std::env::var("ASSISTANT_DRY_RUN").ok().as_deref() == Some("1");
    let qa_log_text = core.qa_log.lock().unwrap().join("\n\n");
    let transcript_text = core.session_transcript.clone();

    match note::save_note(&vault_root, &date, &draft, &transcript_text, &qa_log_text, dry_run) {
        Ok(paths) => {
            if dry_run {
                log::info!("assistant: dry-run, not written: {:?}", paths);
            } else {
                log::info!("assistant: note saved: {:?}", paths);
            }
            core.note_state = NoteState::Saved;
            core.note_error = None;
        }
        Err(e) => {
            core.note_state = NoteState::Failed;
            core.note_error = Some(e);
        }
    }
    core.emit_note();
}

pub async fn discard_note(handle: &AssistantHandle) {
    let mut core = handle.0.lock().await;
    core.note_state = NoteState::Idle;
    core.note_draft = None;
    core.note_markdown.clear();
    core.note_error = None;
    core.emit_note();
}

/// Removes a previously-captured baseline prefix (the open utterance's text
/// as it stood when voice-ask's hotkey was pressed) from freshly-ingested
/// transcript text. A mismatched or empty baseline passes `text` through
/// unchanged, which is what a fresh utterance started after the press
/// should do.
fn strip_voice_baseline(baseline: &str, text: &str) -> String {
    if baseline.is_empty() {
        return text.to_string();
    }
    match text.strip_prefix(baseline) {
        Some(rest) => rest.trim_start().to_string(),
        None => text.to_string(),
    }
}

async fn on_transcript_update(handle: &AssistantHandle, update: TranscriptUpdate) {
    let mut core = handle.0.lock().await;
    if !core.enabled || !core.session_open {
        if core.transcript_ingest_ok {
            core.transcript_ingest_ok = false;
            log::info!(
                "assistant: transcript updates now dropping, enabled={} session_open={}",
                core.enabled, core.session_open
            );
        }
        return;
    }
    if !core.transcript_ingest_ok {
        core.transcript_ingest_ok = true;
        log::info!("assistant: transcript updates now reaching the assistant log");
    }
    let outs = core.transcript.ingest(&update);
    for out in outs {
        match out {
            AssemblerOut::Running { speaker, text } => {
                if speaker == Speaker::You && core.voice.is_capturing() {
                    let voice_text = strip_voice_baseline(&core.voice_baseline, &text);
                    core.voice.note_running(&voice_text);
                }
                core.trigger.consume_running(speaker, &text);
            }
            AssemblerOut::Utterance { speaker, text } => {
                log::info!(
                    "assistant: transcript utterance ingested, speaker={:?} len={}",
                    speaker, text.len()
                );
                if speaker == Speaker::You && core.voice.is_capturing() {
                    let voice_text = strip_voice_baseline(&core.voice_baseline, &text);
                    core.voice.note_utterance(&voice_text);
                    // The utterance the baseline was measured against just
                    // closed; anything after this is a fresh utterance the
                    // baseline should not touch.
                    core.voice_baseline.clear();
                }
                core.trigger.consume_utterance(speaker, &text);
                // Someone finished speaking: refresh Ask Next, unless
                // Joseph paused the assistant from the header dot.
                if core.listening {
                    core.ask_lane.request(&core.transcript, false);
                }
            }
        }
    }
}

/// The claude-turn part of opening a session, isolated so a test can inject
/// a fake `Runner` and check the failure path without a real binary, a real
/// DB, or a running Tauri app.
async fn open_lanes_with(
    handle: &AssistantHandle,
    cfg: LaneConfig,
    seed: String,
    brief: String,
    runner: Runner,
) {
    let mut core = handle.0.lock().await;
    core.lanes = AnswerLanes::new(runner.clone());
    let feed = core.frames.clone().map(FrameFeed::new);
    core.lanes.set_frames(feed.clone());
    let emit = core.card_emit.clone();
    core.lanes.open(seed, &cfg, emit).await;
    core.lanes_ready = core.lanes.is_ready();

    // The ask lane forks the same trunk once the answer lanes are up. Its
    // failure leaves the answer lanes alone; it reports "unavailable".
    let hooks = core.ask_hooks.clone();
    core.ask_lane = AskLane::new(runner, hooks);
    core.ask_lane.set_chat_log(core.chat_log.clone());
    core.ask_lane.set_frames(feed);
    core.brief = brief.clone();
    let trunk = core.lanes.trunk_id();
    let ask_cfg = AskConfig {
        cwd: cfg.cwd.clone(),
        model: cfg.fast_model.clone(),
        deep_model: cfg.deep_model.clone(),
        deep_effort: cfg.deep_effort.clone(),
        deep_read_dirs: cfg.deep_read_dirs.clone(),
    };
    core.ask_lane.open(trunk, ask_cfg, brief).await;

    // A brief set while the lanes were opening went to `pending_brief` (the
    // session was not open yet). It is newer than `brief`: the session uses it.
    // An edit that cleared the brief counts too: it wins over `brief`.
    let edited = std::mem::take(&mut core.brief_edited);
    let pending = core.pending_brief.take();
    let newer = if edited {
        Some(pending.unwrap_or_default()).filter(|b| *b != core.brief)
    } else {
        pending.filter(|b| *b != core.brief)
    };
    if let Some(newer) = &newer {
        core.brief = newer.clone();
        core.lanes.set_brief(newer.clone());
        core.ask_lane.set_brief(newer.clone());
    }
    core.session_open = true;
    let lanes_ready = core.lanes_ready;
    core.set_last_error(if lanes_ready {
        None
    } else {
        Some("assistant lanes failed to open".to_string())
    });
    log::info!(
        "assistant: session opened, session_open=true lanes_ready={}",
        core.lanes_ready
    );
    core.emit_status();
    drop(core);
    if let Some(newer) = newer {
        refresh_startup(handle, newer).await;
    }
}

/// Bound to `recording-started`. Reloads settings fresh (so a save made
/// before this meeting takes effect), resets the session, and opens both
/// lanes if the assistant is enabled and claude resolves.
///
/// `force_enabled`, when `Some`, wins over the persisted `settings.enabled`
/// for the open/don't-open decision. `set_enabled` passes this when the
/// assistant gets flipped on through the panel switch while a recording is
/// already running: that explicit press should open the session right
/// away, not defer to a possibly-stale settings row that would otherwise
/// leave the session closed until the next stop/start cycle.
async fn open_session(handle: &AssistantHandle, force_enabled: Option<bool>) {
    open_session_in(handle, force_enabled, &std::env::temp_dir()).await;
}

/// `open_session` with the folder that holds session dirs passed in, so a
/// test can make the dir step fail without touching the process environment.
async fn open_session_in(handle: &AssistantHandle, force_enabled: Option<bool>, session_root: &std::path::Path) {
    log::info!("assistant: open_session entered");
    let app = { handle.0.lock().await.app.clone() };

    let settings = match &app {
        Some(app) => match app.try_state::<crate::state::AppState>() {
            Some(state) => AssistantSettings::load(state.db_manager.pool())
                .await
                .unwrap_or_else(|e| {
                    log::warn!("assistant: could not load settings, using defaults: {}", e);
                    AssistantSettings::default()
                }),
            None => AssistantSettings::default(),
        },
        None => AssistantSettings::default(),
    };

    let meeting_name = crate::audio::recording_commands::get_recording_meeting_name()
        .await
        .ok()
        .flatten();

    let cfg_ready = {
        let mut core = handle.0.lock().await;
        core.settings = settings.clone();
        core.transcript = TranscriptLog::default();
        core.trigger.reset();
        core.qa_log.lock().unwrap().clear();
        core.chat_log.lock().unwrap().clear();
        core.history.lock().unwrap().reset(chrono::Utc::now().timestamp_millis());
        core.note_state = NoteState::Idle;
        core.note_draft = None;
        core.note_markdown.clear();
        core.note_error = None;
        core.session_transcript.clear();
        // The previous meeting's answered and open questions go with its
        // transcript, however far this attempt gets.
        core.reset_ask_lane();
        core.session_open = false;
        core.lanes_ready = false;
        // A new recording starts with no frames and no capture task.
        core.stop_capture();
        core.frames = None;

        core.mode = parse_mode(&settings.trigger_mode).unwrap_or(TriggerMode::Gated);
        let names = split_names(&settings.names);
        let mode = core.mode;
        let listening = core.listening;
        core.trigger.update(mode, settings.quiet_gap_secs, listening, names);

        if !force_enabled.unwrap_or(settings.enabled) {
            core.enabled = false;
            core.emit_status();
            false
        } else {
            core.enabled = true;
            true
        }
    };
    if !cfg_ready {
        log::info!("assistant: open_session stopped, the assistant is disabled in settings");
        return;
    }

    let probe = settings::probe_claude(&settings).await;
    let bin = settings::resolve_claude_binary(&settings).filter(|_| probe.ok);
    {
        let mut core = handle.0.lock().await;
        core.claude_ok = probe.ok;
        if bin.is_none() {
            core.set_last_error(
                probe
                    .error
                    .clone()
                    .or_else(|| Some("claude binary not found".to_string())),
            );
            core.emit_status();
            return;
        }
    }
    let bin = bin.expect("checked above");

    let session_dir = session_root
        .join("meetily-assistant")
        .join(uuid::Uuid::new_v4().to_string());
    if let Err(e) = std::fs::create_dir_all(&session_dir) {
        let mut core = handle.0.lock().await;
        core.set_last_error(Some(format!("could not create the assistant session dir: {}", e)));
        core.emit_status();
        return;
    }

    // Take the brief only now that the session is sure to open: an early
    // return above leaves it pending, so the next attempt still has it.
    let brief = {
        let mut core = handle.0.lock().await;
        core.brief_edited = false;
        core.pending_brief.take()
    };

    let cfg = LaneConfig {
        cwd: session_dir,
        fast_model: settings.fast_model.clone(),
        fast_effort: settings.fast_effort.clone(),
        deep_model: settings.deep_model.clone(),
        deep_effort: settings.deep_effort.clone(),
        deep_read_dirs: resolve_deep_read_dirs(&settings.deep_read_dirs),
        note_prompt: NOTE_PROMPT.to_string(),
    };

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut seed = format!(
        "Meeting: {}\nDate: {}",
        meeting_name.as_deref().unwrap_or("Untitled meeting"),
        date
    );
    if let Some(brief) = brief.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        seed.push_str("\n\nBrief:\n");
        seed.push_str(brief);
    }

    if settings.see_zoom_window {
        start_capture(handle, app.as_ref()).await;
    }

    // The fixed files are read once here; a later brief change only
    // re-resolves the project page.
    let brief = brief.unwrap_or_default();
    let b = brief.clone();
    let (startup, rendered) = tokio::task::spawn_blocking(move || {
        let startup = super::startup_context::Startup::load_from_home();
        let rendered = startup.as_ref().map(|s| s.render(&b));
        (startup, rendered)
    })
    .await
    .unwrap_or((None, None));
    let ctx_handle = {
        let mut core = handle.0.lock().await;
        let ctx = match &rendered {
            Some((ctx, slug)) => {
                super::startup_context::log_built(ctx, slug.as_deref(), "built at open");
                ctx.as_str()
            }
            None => "",
        };
        super::startup_context::set_context(&core.startup_ctx, ctx);
        core.startup = startup;
        core.startup_ctx.clone()
    };
    let runner = super::startup_context::with_context(claude_runner(bin), ctx_handle);
    open_lanes_with(handle, cfg, seed, brief, runner).await;
}

/// The Settings switch "see the Zoom window", applied to a running call.
/// Off: stop the capture, empty and drop the store, take the frames away
/// from both lanes. On, with a session open and no capture running: start
/// one and hand its feed to both lanes. Without a session it only records
/// the choice; the next open reads the saved row.
pub async fn apply_see_zoom_window(handle: &AssistantHandle, on: bool) {
    let dir = if on {
        let (app, wanted) = {
            let core = handle.0.lock().await;
            (core.app.clone(), core.session_open && core.capture.is_none())
        };
        if wanted {
            zoom_frames_dir(app.as_ref()).await
        } else {
            None
        }
    } else {
        None
    };
    apply_see_zoom_window_in(handle, on, dir).await;
}

/// `apply_see_zoom_window` with the frames folder already resolved.
async fn apply_see_zoom_window_in(handle: &AssistantHandle, on: bool, dir: Option<PathBuf>) {
    let mut core = handle.0.lock().await;
    core.settings.see_zoom_window = on;
    if !on {
        core.stop_capture();
        if let Some(store) = core.frames.take() {
            store.clear();
        }
        core.lanes.set_frames(None);
        core.ask_lane.set_frames(None);
        return;
    }
    if !core.session_open || core.capture.is_some() {
        return;
    }
    let Some(dir) = dir else {
        log::warn!("assistant: no folder for Zoom frames, running without sight");
        return;
    };
    let store = FrameStore::new();
    core.capture = Some(capture::start(CaptureConfig::new(dir), store.clone()));
    core.frames = Some(store.clone());
    let feed = FrameFeed::new(store);
    core.lanes.set_frames(Some(feed.clone()));
    core.ask_lane.set_frames(Some(feed));
}

/// Where this recording's kept Zoom frames are saved: `zoom-frames` inside
/// the meeting folder, or when the folder is not known yet, `zoom-frames/
/// <date_time>` under the app data dir. Never the temp session dir.
async fn zoom_frames_dir(app: Option<&AppHandle<Wry>>) -> Option<PathBuf> {
    if let Ok(Some(folder)) = crate::audio::recording_commands::get_meeting_folder_path().await {
        return Some(PathBuf::from(folder).join("zoom-frames"));
    }
    let base = app?.path().app_data_dir().ok()?;
    Some(
        base.join("zoom-frames")
            .join(chrono::Local::now().format("%Y-%m-%d_%H%M%S").to_string()),
    )
}

/// Makes this recording's frame store and starts the capture task (it
/// returns at once). A missing folder only means no frames: the assistant
/// carries on without sight.
async fn start_capture(handle: &AssistantHandle, app: Option<&AppHandle<Wry>>) {
    let Some(dir) = zoom_frames_dir(app).await else {
        log::warn!("assistant: no folder for Zoom frames, running without sight");
        return;
    };
    let store = FrameStore::new();
    let capture = capture::start(CaptureConfig::new(dir), store.clone());
    let mut core = handle.0.lock().await;
    core.frames = Some(store);
    core.capture = Some(capture);
}

/// Bound to `recording-stopped`. Leaves the transcript and Q&A log in place
/// for `draft_note`; aborts any turn still in flight.
async fn close_session(handle: &AssistantHandle) {
    let mut core = handle.0.lock().await;
    core.stop_capture();
    if !core.session_open {
        return;
    }
    core.session_open = false;
    core.lanes.close();
    core.ask_lane.close();
    core.pending_brief = Some(core.brief.clone()).filter(|b| !b.trim().is_empty());
    core.emit_status();
}

/// Wires the callback fields that need a live `AppHandle` (`trigger.on_fire`,
/// `voice.on_submit`, `voice.publish`), loads settings once for an accurate
/// initial status, and registers the transcript-update / recording-started /
/// recording-stopped listeners. Runs once, from the tauri `setup` hook.
pub fn install(app: AppHandle<Wry>) {
    let init_app = app.clone();
    tauri::async_runtime::spawn(async move {
        wire(init_app).await;
    });
}

async fn wire(app: AppHandle<Wry>) {
    let handle = app.state::<AssistantHandle>().inner().clone();
    let (qa_log, chat_log, history) = {
        let core = handle.0.lock().await;
        (core.qa_log.clone(), core.chat_log.clone(), core.history.clone())
    };
    let forward_app = app.clone();
    let forward: super::lanes::EmitFn = Arc::new(move |card: CardOut| {
        if let Err(e) = forward_app.emit("assistant-card", &card) {
            log::warn!("assistant: failed to emit assistant-card: {}", e);
        }
    });
    let card_emit = make_card_emit(handle.clone(), qa_log, chat_log, history.clone(), forward);

    {
        let mut core = handle.0.lock().await;
        core.app = Some(app.clone());
        core.card_emit = card_emit;
        core.ask_hooks = make_ask_hooks(app.clone(), handle.clone(), history.clone());
        core.ask_lane = AskLane::new(claude_runner(PathBuf::new()), core.ask_hooks.clone());

        let fire_handle = handle.clone();
        core.trigger.on_fire = Arc::new(move |question, _reason| {
            let handle = fire_handle.clone();
            tauri::async_runtime::spawn(async move {
                ask(&handle, question, CardKind::Answer, None).await;
            });
        });

        let submit_handle = handle.clone();
        core.voice.on_submit = Arc::new(move |question: String| {
            let handle = submit_handle.clone();
            tauri::async_runtime::spawn(async move {
                ask(
                    &handle,
                    question,
                    CardKind::Ask,
                    Some("This question was asked by Joseph directly. Do not reply SKIP."),
                )
                .await;
            });
        });
        core.voice.publish = make_voice_publish(app.clone());

        if let Some(state) = app.try_state::<crate::state::AppState>() {
            match AssistantSettings::load(state.db_manager.pool()).await {
                Ok(s) => core.settings = s,
                Err(e) => log::warn!("assistant: could not load settings at startup: {}", e),
            }
        }
        core.claude_ok = settings::probe_claude(&core.settings).await.ok;
        core.mode = parse_mode(&core.settings.trigger_mode).unwrap_or(TriggerMode::Gated);
        let names = split_names(&core.settings.names);
        let mode = core.mode;
        let listening = core.listening;
        let quiet_gap = core.settings.quiet_gap_secs;
        core.trigger.update(mode, quiet_gap, listening, names);
        core.emit_status();
    }

    register_listeners(app, handle);
    log::info!("assistant: listeners registered, wire() complete");
}

fn register_listeners(app: AppHandle<Wry>, handle: AssistantHandle) {
    {
        let handle = handle.clone();
        app.listen("transcript-update", move |event| {
            let Ok(update) = serde_json::from_str::<TranscriptUpdate>(event.payload()) else {
                return;
            };
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                on_transcript_update(&handle, update).await;
            });
        });
    }
    {
        let handle = handle.clone();
        app.listen("recording-started", move |_event| {
            log::info!("assistant: recording-started event received");
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                open_session(&handle, None).await;
            });
        });
    }
    {
        app.listen("recording-stopped", move |_event| {
            log::info!("assistant: recording-stopped event received");
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                close_session(&handle).await;
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::claude_cli::ClaudeOutcome;

    fn tu(source: &str, text: &str, start: f64, end: f64) -> TranscriptUpdate {
        TranscriptUpdate {
            text: text.to_string(),
            timestamp: "00:00:00".to_string(),
            source: source.to_string(),
            sequence_id: 1,
            chunk_start_time: start,
            is_partial: false,
            confidence: 1.0,
            audio_start_time: start,
            audio_end_time: end,
            duration: end - start,
        }
    }

    #[tokio::test]
    async fn set_mode_round_trips_through_get_state() {
        let handle = AssistantHandle::new();
        set_mode(&handle, "continuous").await.unwrap();
        let state = get_state(&handle).await;
        assert_eq!(state.mode, "continuous");
    }

    #[tokio::test]
    async fn set_mode_rejects_an_unknown_mode() {
        let handle = AssistantHandle::new();
        assert!(set_mode(&handle, "sleepwalking").await.is_err());
    }

    #[tokio::test]
    async fn disabled_ignores_transcript_events() {
        let handle = AssistantHandle::new();
        {
            let mut core = handle.0.lock().await;
            core.enabled = false;
            core.session_open = true;
        }

        on_transcript_update(&handle, tu("mic", "hello", 0.0, 1.0)).await;
        // A speaker flip would close "hello" into finals if it had been
        // ingested; while disabled, nothing should have happened at all.
        on_transcript_update(&handle, tu("system", "hi there", 3.0, 4.0)).await;

        let core = handle.0.lock().await;
        let (text, cursor) = core.transcript.delta_since(0);
        assert_eq!(text, "");
        assert_eq!(cursor, 0);
    }

    /// Reproduces the leading-text bleed: speech happens, merges into an
    /// open You utterance, then the hotkey is pressed mid-utterance. What
    /// voice-ask hears must start at the press, not at the utterance's
    /// start.
    #[tokio::test]
    async fn voice_ask_excludes_speech_from_before_the_hotkey() {
        let handle = AssistantHandle::new();
        let published = Arc::new(StdMutex::new(Vec::new()));
        let published_clone = published.clone();
        {
            let mut core = handle.0.lock().await;
            core.enabled = true;
            core.session_open = true;
            core.voice = VoiceAsk::new(
                |_q: String| {},
                move |s: VoiceState| published_clone.lock().unwrap().push(s),
            );
        }

        // Spoken before the hotkey; merges into the open You utterance.
        on_transcript_update(&handle, tu("mic", "Okay.", 0.0, 0.5)).await;

        voice_start(&handle).await;

        // Same utterance continues (same speaker, well inside the 1.2s gap).
        on_transcript_update(&handle, tu("mic", "let's talk about the roadmap", 0.6, 2.0)).await;

        let heard = match published.lock().unwrap().last().cloned() {
            Some(VoiceState::Listening { heard }) => heard,
            other => panic!("expected Listening, got {:?}", other),
        };
        assert_eq!(heard, "let's talk about the roadmap");
    }

    /// A dropped button press must be visible, not silent: the panel only
    /// has `lastError` to show, so pressing Ask, Explain, or Catch up while
    /// closed has to reach it or it just looks like nothing happened.
    #[tokio::test]
    async fn ask_explain_and_catchup_set_last_error_when_session_closed() {
        let handle = AssistantHandle::new();

        ask(&handle, "hello?".to_string(), CardKind::Ask, None).await;
        assert_eq!(
            get_state(&handle).await.last_error.as_deref(),
            Some("Assistant session is not open yet.")
        );

        explain(&handle).await;
        assert_eq!(
            get_state(&handle).await.last_error.as_deref(),
            Some("Assistant session is not open yet.")
        );

        catchup(&handle).await;
        assert_eq!(
            get_state(&handle).await.last_error.as_deref(),
            Some("Assistant session is not open yet.")
        );
    }

    /// Without an active recording, turning the assistant on must not open
    /// a session on its own -- only `recording-started`, or `set_enabled`
    /// catching an already-running recording, should do that.
    #[tokio::test]
    async fn set_enabled_true_does_not_open_a_session_without_a_recording() {
        let handle = AssistantHandle::new();
        set_enabled(&handle, true).await;
        assert!(!get_state(&handle).await.session_open);
    }

    /// A session can be open while the assistant itself is off (toggled off
    /// mid-recording, or disabled by default while a recording that predates
    /// it is still going). explain/catchup must not fire a real Claude call
    /// in that state -- they have to stop at the same `enabled` guard `ask`
    /// already had, not just at `session_open`.
    #[tokio::test]
    async fn explain_and_catchup_set_last_error_when_session_open_but_disabled() {
        let handle = AssistantHandle::new();
        {
            let mut core = handle.0.lock().await;
            core.session_open = true;
            core.enabled = false;
        }

        explain(&handle).await;
        assert_eq!(
            get_state(&handle).await.last_error.as_deref(),
            Some("Assistant is turned off.")
        );

        catchup(&handle).await;
        assert_eq!(
            get_state(&handle).await.last_error.as_deref(),
            Some("Assistant is turned off.")
        );

        // Neither call touched the lanes: still the untouched default,
        // never opened.
        let core = handle.0.lock().await;
        assert!(!core.lanes_ready);
    }

    /// `set_enabled(false)` must leave every piece of state agreeing the
    /// session is closed, not just the lanes. Otherwise `ask`/`explain`/
    /// `catchup` (which check `session_open` first) would sail past their
    /// guard into lanes that were just closed out from under them.
    #[tokio::test]
    async fn set_enabled_false_closes_the_session_too() {
        let handle = AssistantHandle::new();
        {
            let mut core = handle.0.lock().await;
            core.session_open = true;
            core.enabled = true;
        }

        set_enabled(&handle, false).await;

        assert!(!get_state(&handle).await.session_open);
    }

    #[tokio::test]
    async fn lane_open_failure_sets_last_error_and_lanes_ready_false() {
        let handle = AssistantHandle::new();
        let failing_runner: Runner = Arc::new(|_turn, _on_delta, _register| {
            Box::pin(async {
                ClaudeOutcome {
                    session_id: None,
                    text: String::new(),
                    error: Some("boom".to_string()),
                }
            })
        });
        let cfg = LaneConfig {
            cwd: std::env::temp_dir(),
            fast_model: "fast".to_string(),
            fast_effort: "low".to_string(),
            deep_model: "deep".to_string(),
            deep_effort: "medium".to_string(),
            deep_read_dirs: Vec::new(),
            note_prompt: "note".to_string(),
        };

        open_lanes_with(&handle, cfg, "seed".to_string(), String::new(), failing_runner).await;

        let core = handle.0.lock().await;
        assert!(!core.lanes_ready);
        assert_eq!(core.last_error.as_deref(), Some("assistant lanes failed to open"));
        assert_eq!(core.ask_lane.snapshot().state, "unavailable");
    }

    /// The panel keys its transient error display on `last_error_seq`
    /// because two errors with identical text (pressing Explain twice with
    /// nothing new to explain, say) still both need to show. Repeated
    /// identical text must still bump the counter each time, and a status
    /// emit that never touches `last_error` (set_listening, in this case)
    /// must never bump it -- that would falsely re-flash a stale error on
    /// an unrelated toggle.
    #[tokio::test]
    async fn last_error_seq_bumps_on_repeat_and_stays_put_on_unrelated_emits() {
        let handle = AssistantHandle::new();

        explain(&handle).await;
        let after_first = get_state(&handle).await.last_error_seq;
        assert!(after_first > 0);

        explain(&handle).await;
        let after_second = get_state(&handle).await;
        assert_eq!(
            after_second.last_error.as_deref(),
            Some("Assistant session is not open yet.")
        );
        assert_eq!(after_second.last_error_seq, after_first + 1);

        set_listening(&handle, false).await;
        let after_unrelated = get_state(&handle).await.last_error_seq;
        assert_eq!(after_unrelated, after_second.last_error_seq);
    }
    use crate::assistant::ask_lane::ASK_PROMPT;

    fn test_lane_cfg() -> LaneConfig {
        LaneConfig {
            cwd: std::env::temp_dir(),
            fast_model: "fast".to_string(),
            fast_effort: "low".to_string(),
            deep_model: "deep".to_string(),
            deep_effort: "medium".to_string(),
            deep_read_dirs: Vec::new(),
            note_prompt: "note".to_string(),
        }
    }

    /// Every turn gets a session id. Ask-lane turns (the ask system prompt,
    /// past the fork) get a one-item list; everything else gets READY.
    fn ask_runner(calls: Arc<StdMutex<Vec<crate::assistant::claude_cli::ClaudeTurn>>>) -> Runner {
        Arc::new(move |turn, _on_delta, _register| {
            // The deep sub-lane's prompt is ASK_PROMPT plus its own addendum.
            let is_ask_turn = turn.append_system_prompt.as_deref().is_some_and(|p| p.starts_with(ASK_PROMPT))
                && !turn.prompt.starts_with("Standing by");
            calls.lock().unwrap().push(turn);
            let text = if is_ask_turn {
                "ASK: What are your fees? | WHY: budget"
            } else {
                "READY"
            };
            Box::pin(async move {
                ClaudeOutcome {
                    session_id: Some(uuid::Uuid::new_v4().to_string()),
                    text: text.to_string(),
                    error: None,
                }
            })
        })
    }

    /// The quick sub-lane's ask turns. The deep sub-lane runs a turn beside
    /// each one, so counting both would double every number.
    fn ask_turns(calls: &Arc<StdMutex<Vec<crate::assistant::claude_cli::ClaudeTurn>>>) -> Vec<String> {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.prompt.contains("CURRENT LIST:") && t.model.as_deref() == Some("fast"))
            .map(|t| t.prompt.clone())
            .collect()
    }

    async fn settle() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn utterance_close_requests_asks_only_while_listening_and_refresh_always_runs() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls.clone())).await;

        on_transcript_update(&handle, tu("system", "what do you need from us", 0.0, 1.0)).await;
        on_transcript_update(&handle, tu("mic", "good question", 2.0, 3.0)).await;
        settle().await;
        assert_eq!(ask_turns(&calls).len(), 1);
        assert_eq!(handle.0.lock().await.ask_lane.snapshot().asks.len(), 1);

        set_listening(&handle, false).await;
        on_transcript_update(&handle, tu("system", "well it depends", 4.0, 5.0)).await;
        settle().await;
        assert_eq!(ask_turns(&calls).len(), 1, "paused: no automatic refresh");

        refresh_asks(&handle).await;
        settle().await;
        assert_eq!(ask_turns(&calls).len(), 2, "manual refresh runs while paused");
    }

    #[tokio::test]
    async fn set_brief_mid_call_reaches_the_ask_lane_and_requests_a_refresh() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), "old goal".to_string(), ask_runner(calls.clone())).await;
        assert_eq!(handle.0.lock().await.ask_lane.snapshot().brief, "old goal");

        set_brief(&handle, "Learn their fees".to_string()).await;
        settle().await;

        let turns = ask_turns(&calls);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].contains("Learn their fees"));
        let core = handle.0.lock().await;
        assert_eq!(core.brief, "Learn their fees");
        assert_eq!(core.ask_lane.snapshot().brief, "Learn their fees");
    }

    #[tokio::test]
    async fn frames_survive_the_stop_but_not_switching_the_assistant_off() {
        let handle = AssistantHandle::new();
        let store = FrameStore::new();
        store.push("/f/1".into(), 1);
        {
            let mut core = handle.0.lock().await;
            core.frames = Some(store.clone());
            core.session_open = true;
        }
        close_session(&handle).await;
        assert_eq!(store.latest_seq(), 1, "the note still needs the frames after the stop");
        assert!(handle.0.lock().await.frames.is_some());

        set_enabled(&handle, false).await;
        assert!(handle.0.lock().await.frames.is_none());
        assert_eq!(store.latest_seq(), 0, "switch off means an empty store");
    }

    #[tokio::test]
    async fn the_zoom_switch_reaches_a_running_call_in_both_directions() {
        let handle = AssistantHandle::new();
        {
            let mut core = handle.0.lock().await;
            core.session_open = true;
        }
        let dir = std::env::temp_dir().join(format!("meetily-zoom-switch-{}", uuid::Uuid::new_v4()));

        // On, session open, no capture: one starts and the lanes get the feed.
        apply_see_zoom_window_in(&handle, true, Some(dir.clone())).await;
        {
            let core = handle.0.lock().await;
            assert!(core.capture.is_some());
            assert!(core.frames.is_some());
            assert!(core.lanes.has_frames() && core.ask_lane.has_frames());
        }

        // On again: nothing restarts.
        apply_see_zoom_window_in(&handle, true, Some(dir.clone())).await;
        assert!(handle.0.lock().await.capture.is_some());

        // Off: capture stopped, store emptied and dropped, lanes blind.
        let store = handle.0.lock().await.frames.clone().unwrap();
        store.push("/f/1".into(), 1);
        apply_see_zoom_window_in(&handle, false, None).await;
        let core = handle.0.lock().await;
        assert!(core.capture.is_none() && core.frames.is_none());
        assert_eq!(store.latest_seq(), 0);
        assert!(!core.lanes.has_frames() && !core.ask_lane.has_frames());
        assert!(!core.settings.see_zoom_window);
    }

    #[tokio::test]
    async fn the_zoom_switch_does_nothing_to_capture_without_a_session() {
        let handle = AssistantHandle::new();
        apply_see_zoom_window_in(&handle, true, Some(std::env::temp_dir())).await;
        let core = handle.0.lock().await;
        assert!(core.capture.is_none() && core.frames.is_none());
    }

    #[tokio::test]
    async fn set_brief_before_a_session_stays_pending() {
        let handle = AssistantHandle::new();
        set_brief(&handle, "intro call".to_string()).await;
        assert_eq!(handle.0.lock().await.pending_brief.as_deref(), Some("intro call"));
    }

    #[tokio::test]
    async fn a_session_that_fails_to_open_keeps_the_saved_brief() {
        let handle = AssistantHandle::new();
        set_brief(&handle, "intro call".to_string()).await;

        // A file where the session root should be: whether claude is missing
        // (probe fails) or present (dir creation fails), the open stops early.
        let not_a_dir = std::env::temp_dir().join(format!("meetily-brief-test-{}", uuid::Uuid::new_v4()));
        std::fs::write(&not_a_dir, b"x").unwrap();
        open_session_in(&handle, Some(true), &not_a_dir).await;
        std::fs::remove_file(&not_a_dir).unwrap();

        let core = handle.0.lock().await;
        assert!(!core.session_open);
        assert_eq!(core.pending_brief.as_deref(), Some("intro call"));
        assert_eq!(core.ask_lane.snapshot().brief, "intro call");
    }

    #[tokio::test]
    async fn brief_edits_outside_a_session_update_the_visible_snapshot() {
        let handle = AssistantHandle::new();
        let emitted = Arc::new(StdMutex::new(Vec::new()));
        let recorded = emitted.clone();
        let hooks = AskHooks {
            emit: Arc::new(move |out| recorded.lock().unwrap().push(out)),
            ..Default::default()
        };
        let calls = Arc::new(StdMutex::new(Vec::new()));
        {
            let mut core = handle.0.lock().await;
            core.ask_hooks = hooks.clone();
            core.ask_lane = AskLane::new(ask_runner(calls.clone()), hooks);
        }

        set_brief(&handle, "intro call".into()).await;
        assert_eq!(handle.0.lock().await.ask_lane.snapshot().brief, "intro call");
        assert_eq!(emitted.lock().unwrap().last().unwrap().brief, "intro call");

        open_lanes_with(&handle, test_lane_cfg(), "seed".into(), "intro call".into(), ask_runner(calls.clone())).await;
        refresh_asks(&handle).await;
        settle().await;
        let id = handle.0.lock().await.ask_lane.snapshot().asks[0].id.clone();
        close_session(&handle).await;
        set_brief(&handle, "next call".into()).await;
        dismiss_ask(&handle, id).await;

        let core = handle.0.lock().await;
        assert_eq!(core.pending_brief.as_deref(), Some("next call"));
        assert_eq!(core.ask_lane.snapshot().brief, "next call");
        assert_eq!(emitted.lock().unwrap().last().unwrap().brief, "next call");
        assert_eq!(ask_turns(&calls).len(), 1, "closed sessions must not refresh");
    }

    /// The open path takes `pending_brief`, drops the lock, and opens the
    /// lanes later; a `set_brief` in that gap lands in `pending_brief`. The
    /// session must end up with it everywhere, and the context must name its
    /// project page.
    #[tokio::test]
    async fn a_brief_set_while_the_lanes_open_becomes_the_session_brief() {
        let home = std::env::temp_dir().join(format!("core-brief-race-{}", uuid::Uuid::new_v4()));
        let wiki = home.join("brain/wiki/Wiki");
        std::fs::create_dir_all(wiki.join("Projects")).unwrap();
        std::fs::write(wiki.join("index.md"), "[[Projects/fesco]]").unwrap();
        std::fs::write(wiki.join("Projects/fesco.md"), "FESCO PAGE TEXT").unwrap();

        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        {
            let mut core = handle.0.lock().await;
            core.startup = Some(crate::assistant::startup_context::Startup::load(&home));
        }
        // The open path already took "old" (pending is empty); the user then
        // edits the brief before open_lanes_with runs.
        set_brief(&handle, "FESCO kickoff".to_string()).await;
        open_lanes_with(&handle, test_lane_cfg(), "seed".into(), "old".into(), ask_runner(calls.clone())).await;

        let core = handle.0.lock().await;
        assert!(core.session_open);
        assert_eq!(core.brief, "FESCO kickoff");
        assert_eq!(core.pending_brief, None);
        assert_eq!(core.ask_lane.snapshot().brief, "FESCO kickoff");
        let ctx = core.startup_ctx.read().unwrap().clone();
        assert!(ctx.contains("FESCO PAGE TEXT"), "context was: {ctx}");
        drop(core);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn closing_the_session_carries_the_brief_into_the_next_one() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), "goal A".to_string(), ask_runner(calls)).await;
        close_session(&handle).await;
        assert_eq!(handle.0.lock().await.pending_brief.as_deref(), Some("goal A"));
    }

    /// A brief cleared while the lanes open wins over the one the open path
    /// took, and the next session does not inherit the old text.
    #[tokio::test]
    async fn a_brief_cleared_while_the_lanes_open_wins() {
        let home = std::env::temp_dir().join(format!("core-brief-clear-{}", uuid::Uuid::new_v4()));
        let wiki = home.join("brain/wiki/Wiki");
        std::fs::create_dir_all(wiki.join("Projects")).unwrap();
        std::fs::write(wiki.join("index.md"), "[[Projects/fesco]]").unwrap();
        std::fs::write(wiki.join("Projects/fesco.md"), "FESCO PAGE TEXT").unwrap();

        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        {
            let mut core = handle.0.lock().await;
            core.startup = Some(crate::assistant::startup_context::Startup::load(&home));
        }
        set_brief(&handle, String::new()).await;
        open_lanes_with(&handle, test_lane_cfg(), "seed".into(), "old".into(), ask_runner(calls.clone())).await;

        {
            let core = handle.0.lock().await;
            assert!(core.session_open);
            assert_eq!(core.brief, "");
            assert_eq!(core.ask_lane.snapshot().brief, "");
            let ctx = core.startup_ctx.read().unwrap().clone();
            assert!(!ctx.contains("FESCO PAGE TEXT"), "context was: {ctx}");
        }
        close_session(&handle).await;
        assert_eq!(handle.0.lock().await.pending_brief, None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn dismiss_ask_removes_the_item_and_tells_the_model() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls.clone())).await;
        refresh_asks(&handle).await;
        settle().await;
        let id = handle.0.lock().await.ask_lane.snapshot().asks[0].id.clone();

        dismiss_ask(&handle, id).await;
        settle().await;

        let turns = ask_turns(&calls);
        assert_eq!(turns.len(), 2);
        assert!(turns[1].contains("Joseph dismissed these"));
        assert!(handle.0.lock().await.ask_lane.snapshot().asks.is_empty());
    }

    #[tokio::test]
    async fn turning_the_assistant_off_closes_the_ask_lane() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls.clone())).await;
        set_enabled(&handle, false).await;
        {
            let core = handle.0.lock().await;
            core.ask_lane.request(&core.transcript, true);
        }
        settle().await;
        assert!(ask_turns(&calls).is_empty());
    }

    #[tokio::test]
    async fn refresh_asks_sets_last_error_when_session_closed() {
        let handle = AssistantHandle::new();
        refresh_asks(&handle).await;
        assert_eq!(
            get_state(&handle).await.last_error.as_deref(),
            Some("Assistant session is not open yet.")
        );
    }

    #[tokio::test]
    async fn manual_refresh_during_a_turn_runs_one_follow_up_without_new_transcript() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let base = ask_runner(calls.clone());
        let runner_gate = gate.clone();
        let runner: Runner = Arc::new(move |turn, delta, register| {
            // Gate the quick sub-lane only; the deep one runs straight through.
            let is_ask = turn.prompt.contains("CURRENT LIST:") && turn.model.as_deref() == Some("fast");
            let reply = base(turn, delta, register);
            let gate = runner_gate.clone();
            Box::pin(async move {
                if is_ask {
                    gate.acquire().await.unwrap().forget();
                }
                reply.await
            })
        });
        let weak_handle = Arc::downgrade(&handle.0);
        handle.0.lock().await.ask_hooks.rerun = Arc::new(move || {
            if let Some(core) = weak_handle.upgrade() {
                tokio::spawn(async move {
                    rerun_asks(&AssistantHandle(core)).await;
                });
            }
        });
        open_lanes_with(&handle, test_lane_cfg(), "seed".into(), String::new(), runner).await;
        set_listening(&handle, false).await;

        refresh_asks(&handle).await;
        settle().await;
        for _ in 0..3 {
            refresh_asks(&handle).await;
        }
        assert_eq!(ask_turns(&calls).len(), 1);

        gate.add_permits(1);
        settle().await;
        assert_eq!(ask_turns(&calls).len(), 2, "manual requests must not be lost");
        gate.add_permits(1);
        settle().await;
        assert_eq!(ask_turns(&calls).len(), 2, "requests coalesce into one follow-up");
        assert_eq!(handle.0.lock().await.ask_lane.snapshot().state, "idle");
    }


    // ---- answered questions kept ----

    fn card(kind: CardKind, phase: &str) -> CardOut {
        CardOut {
            id: "c1".to_string(),
            kind,
            question: "Who owns backups at FESCO?".to_string(),
            lead: "The ops team.".to_string(),
            bullets: vec!["Per the vault note".to_string(), "Second line".to_string()],
            source: "deep".to_string(),
            phase: phase.to_string(),
            changed_lines: Vec::new(),
            ts: 1,
        }
    }

    #[test]
    fn a_settled_ask_card_reaches_the_note_log_and_the_chat_log() {
        let (qa, chats) = (StdMutex::new(Vec::new()), StdMutex::new(Vec::new()));
        record_card(&card(CardKind::Ask, "drafting"), &qa, &chats);
        assert!(qa.lock().unwrap().is_empty() && chats.lock().unwrap().is_empty(), "drafting is not heard");

        record_card(&card(CardKind::Ask, "checked"), &qa, &chats);
        record_card(&card(CardKind::Ask, "corrected"), &qa, &chats);
        assert_eq!(qa.lock().unwrap().len(), 2);
        let chats = chats.lock().unwrap();
        assert_eq!(chats.len(), 2);
        assert_eq!(chats[0].question, "Who owns backups at FESCO?");
        assert_eq!(chats[0].answer, "The ops team.\n- Per the vault note\n- Second line");
    }

    #[test]
    fn answer_cards_explain_and_catch_up_are_not_chat() {
        let (qa, chats) = (StdMutex::new(Vec::new()), StdMutex::new(Vec::new()));
        record_card(&card(CardKind::Answer, "checked"), &qa, &chats);
        record_card(&card(CardKind::Explain, "checked"), &qa, &chats);
        record_card(&card(CardKind::Catchup, "checked"), &qa, &chats);
        assert_eq!(qa.lock().unwrap().len(), 1, "only the answer card is in the Q&A log");
        assert!(chats.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_settled_ask_card_requests_an_ask_turn_without_a_new_utterance() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls.clone())).await;
        let emit = {
            let mut core = handle.0.lock().await;
            core.listening = false;
            let (qa, chats) = (core.qa_log.clone(), core.chat_log.clone());
            let emit = make_card_emit(handle.clone(), qa, chats, core.history.clone(), Arc::new(|_| {}));
            core.card_emit = emit.clone();
            emit
        };
        assert_eq!(ask_turns(&calls).len(), 0);

        emit(card(CardKind::Ask, "drafting"));
        settle().await;
        assert_eq!(ask_turns(&calls).len(), 0, "a drafting card requests nothing");
        emit(card(CardKind::Answer, "checked"));
        settle().await;
        assert_eq!(ask_turns(&calls).len(), 0, "only Ask cards count");

        emit(card(CardKind::Ask, "checked"));
        settle().await;
        let turns = ask_turns(&calls);
        assert_eq!(turns.len(), 1, "a settled Ask card requests a turn, listening off");
        assert!(turns[0].contains("Who owns backups at FESCO?"), "the turn carries the chat exchange: {}", turns[0]);
        assert!(turns[0].contains("The ops team."));

        close_session(&handle).await;
        emit(card(CardKind::Ask, "corrected"));
        settle().await;
        assert_eq!(ask_turns(&calls).len(), 1, "no session open, no turn");
    }

    #[tokio::test]
    async fn answer_ask_moves_the_item_leaves_the_brief_and_requests_a_refresh() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), "my goal".to_string(), ask_runner(calls.clone())).await;
        refresh_asks(&handle).await;
        settle().await;
        let id = handle.0.lock().await.ask_lane.snapshot().asks[0].id.clone();

        assert!(!answer_ask(&handle, "no-such-id".to_string(), "x".to_string()).await);
        assert!(!answer_ask(&handle, id.clone(), "  ".to_string()).await);
        assert_eq!(ask_turns(&calls).len(), 1, "a refused answer requests nothing");

        assert!(answer_ask(&handle, id.clone(), "  Ten percent.  ".to_string()).await);
        settle().await;
        let snap = handle.0.lock().await.ask_lane.snapshot();
        assert!(snap.asks.is_empty());
        assert_eq!(snap.answered.len(), 1);
        assert_eq!(snap.answered[0].id, id);
        assert_eq!(snap.answered[0].answer, "Ten percent.");
        assert_eq!(snap.answered[0].how, crate::assistant::ask_list::AnswerHow::You);
        assert_eq!(snap.brief, "my goal", "the frontend owns the brief line");
        let turns = ask_turns(&calls);
        assert_eq!(turns.len(), 2, "a refresh was requested");
        assert!(turns[1].contains("ANSWERED SO FAR (never ask these again):"));
        assert!(turns[1].contains("What are your fees? Answer: Ten percent."));
    }

    /// Ask turns say ASK on the first refresh and DONE after; the note turn
    /// (its prompt starts with TRANSCRIPT:) returns a two-section note.
    fn note_runner(calls: Arc<StdMutex<Vec<crate::assistant::claude_cli::ClaudeTurn>>>) -> Runner {
        let ask_turns_seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        Arc::new(move |turn, _on_delta, _register| {
            let is_ask_turn = turn.append_system_prompt.as_deref().is_some_and(|p| p.starts_with(ASK_PROMPT))
                && !turn.prompt.starts_with("Standing by");
            let is_note = turn.prompt.starts_with("TRANSCRIPT:");
            calls.lock().unwrap().push(turn);
            let text = if is_note {
                "=== SLUG ===\nfesco-kickoff\n=== NOTE ===\n## Summary\n\nThey talked fees.\n\n## Open questions\n\n- Who owns backups?"
            } else if is_ask_turn {
                // The first refresh runs a quick and a deep turn.
                if ask_turns_seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
                    "ASK: What are your fees? | WHY: budget"
                } else {
                    "DONE: What are your fees? | ANSWER: Ten percent of rent. | BY: call"
                }
            } else {
                "READY"
            };
            Box::pin(async move {
                ClaudeOutcome {
                    session_id: Some(uuid::Uuid::new_v4().to_string()),
                    text: text.to_string(),
                    error: None,
                }
            })
        })
    }

    #[tokio::test]
    async fn the_drafted_note_gets_the_answered_section_built_in_code() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls.clone())).await;
        refresh_asks(&handle).await;
        settle().await;
        refresh_asks(&handle).await;
        settle().await;
        assert_eq!(handle.0.lock().await.ask_lane.snapshot().answered.len(), 1);

        close_session(&handle).await;
        assert!(draft_note(&handle).await.is_ok());

        let core = handle.0.lock().await;
        assert!(core.note_markdown.ends_with(
            "## Answered questions\n\n- **What are your fees?** Ten percent of rent. (On the call)"
        ), "note was: {}", core.note_markdown);
        assert!(core.note_markdown.starts_with("## Summary"));
        assert_eq!(core.note_draft.as_ref().unwrap().markdown, core.note_markdown, "the saved file gets it too");
        let note_prompt = calls
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.prompt.starts_with("TRANSCRIPT:"))
            .map(|t| t.prompt.clone())
            .expect("a note turn ran");
        assert!(note_prompt.contains("QUESTIONS ANSWERED DURING THE CALL:\n- What are your fees? Answer: Ten percent of rent."));
    }

    #[tokio::test]
    async fn the_answered_list_does_not_leak_into_a_meeting_whose_session_fails_to_open() {
        for (label, force_enabled, as_file) in [("disabled", Some(false), false), ("session dir fails", Some(true), true)] {
            let handle = AssistantHandle::new();
            let calls = Arc::new(StdMutex::new(Vec::new()));
            open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls.clone())).await;
            refresh_asks(&handle).await;
            settle().await;
            refresh_asks(&handle).await;
            settle().await;
            assert_eq!(handle.0.lock().await.ask_lane.snapshot().answered.len(), 1, "{}: meeting 1 answered one", label);
            close_session(&handle).await;

            // Meeting 2: the open stops early.
            let root = std::env::temp_dir().join(format!("meetily-leak-test-{}", uuid::Uuid::new_v4()));
            if as_file {
                std::fs::write(&root, b"x").unwrap();
            }
            open_session_in(&handle, force_enabled, &root).await;
            let _ = std::fs::remove_file(&root);

            {
                let core = handle.0.lock().await;
                assert!(!core.session_open, "{}", label);
                let snap = core.ask_lane.snapshot();
                assert!(snap.answered.is_empty(), "{}: answered leaked into meeting 2", label);
                assert!(snap.asks.is_empty(), "{}", label);
                assert!(core.ask_lane.answered_items().is_empty(), "{}", label);
            }
            assert!(draft_note(&handle).await.is_ok());
            let core = handle.0.lock().await;
            assert!(!core.note_markdown.contains("Answered questions"), "{}: note was: {}", label, core.note_markdown);
        }
    }

    #[tokio::test]
    async fn a_note_with_nothing_answered_gets_no_answered_section() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls.clone())).await;
        refresh_asks(&handle).await;
        settle().await;
        close_session(&handle).await;
        assert!(draft_note(&handle).await.is_ok());
        let core = handle.0.lock().await;
        assert!(!core.note_markdown.contains("Answered questions"));
        assert!(core.note_markdown.ends_with("- Who owns backups?"));
    }
}
