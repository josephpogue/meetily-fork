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
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Listener, Manager, Wry};

use crate::audio::TranscriptUpdate;
use crate::database::repositories::assistant_items::{AssistantItem, AssistantItemsRepository};
use crate::database::repositories::assistant_meta::AssistantMetaRepository;

use super::ask_lane::{AskConfig, AskHooks, AskLane, AsksOut, ChatExchange, ChatLog};
use super::lanes::{claude_runner, AnswerLanes, CardKind, CardOut, FrameFeed, LaneConfig, Runner};
use super::note::{self, NoteDraft, NOTE_PROMPT};
use super::settings::{self, AssistantSettings};
use super::transcript::{AssemblerOut, Speaker, TranscriptLog};
use super::trigger::{TriggerEngine, TriggerMode};
use super::vision::capture::{self, CaptureConfig, CaptureHandle};
use super::vision::{CallApp, FrameStore};
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
pub struct NoteOut {
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
    /// Sends the note's state to the panel (`assistant-note`); a no-op until
    /// `wire` sets it, and replaced in tests to see what was sent.
    note_emit: Arc<dyn Fn(NoteOut) + Send + Sync>,
    /// (card id, "Q: ...\nA: ...") per settled card, so a deleted card's entry
    /// can be removed before the note is drafted.
    qa_log: QaLog,
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
    /// and is cleared once used. Stop clears it: the call's own brief does
    /// not carry over, only one typed after Stop.
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
        Self::with_history(super::history::new_handle())
    }
}

impl AssistantCore {
    /// A core that collects into `history`, the same handle the app manages
    /// as its own state so the meeting-save commands never take the core lock.
    pub fn with_history(history: super::history::HistoryHandle) -> Self {
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
            note_emit: Arc::new(|_| {}),
            qa_log: Arc::new(StdMutex::new(Vec::new())),
            history,
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

    #[cfg(test)]
    fn history_handle_for_test(&self) -> super::history::HistoryHandle {
        self.history.clone()
    }

    fn reset_ask_lane(&mut self) {
        let brief = self.ask_lane.snapshot().brief;
        // Silent: the old list was already announced when the session
        // closed, and a second emit here would reach the history collector
        // after the new recording reset it.
        self.ask_lane.discard();
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

    /// Tells the panel the note's state. Every place that changes `note_state`
    /// calls this right after, so the panel never keeps a state the core left.
    fn emit_note(&self) {
        (self.note_emit)(NoteOut {
            state: self.note_state.label(),
            markdown: self.note_markdown.clone(),
            error: self.note_error.clone(),
        });
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
    qa_log: QaLog,
    chat_log: ChatLog,
    history: super::history::HistoryHandle,
    forward: super::lanes::EmitFn,
) -> super::lanes::EmitFn {
    Arc::new(move |card: CardOut| {
        // A card Joseph deleted while it drafted must not re-enter the logs
        // when it settles (the collector already refuses it).
        if !history.lock().unwrap().is_deleted(&card.id) {
            record_card(&card, &qa_log, &chat_log);
        }
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

type QaLog = Arc<StdMutex<Vec<(String, String)>>>;

/// The Q&A log's texts, joined for the note.
fn qa_log_text(qa_log: &StdMutex<Vec<(String, String)>>) -> String {
    qa_log.lock().unwrap().iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n\n")
}

/// The answered questions the lane holds, less the ones Joseph deleted.
fn answered_kept(lane: &AskLane, deleted: &[String]) -> Vec<super::ask_list::AnsweredItem> {
    lane.answered_items().into_iter().filter(|a| !deleted.contains(&a.id)).collect()
}

/// The open questions on the lane's list, less the ones Joseph deleted.
fn open_asks_kept(lane: &AskLane, deleted: &[String]) -> Vec<String> {
    lane.snapshot().asks.into_iter().filter(|a| !deleted.contains(&a.id)).map(|a| a.question).collect()
}

fn is_settled_ask(card: &CardOut) -> bool {
    matches!(card.kind, CardKind::Ask) && (card.phase == "checked" || card.phase == "corrected")
}

/// What a settled card leaves behind. An answer or an Ask card goes to the
/// note's Q&A log; an Ask card (Joseph's own question) also goes to the chat
/// log, so Ask next can hear it and mark a list question answered.
fn record_card(card: &CardOut, qa_log: &StdMutex<Vec<(String, String)>>, chat_log: &StdMutex<Vec<ChatExchange>>) {
    let settled = card.phase == "checked" || card.phase == "corrected";
    if !settled {
        return;
    }
    if matches!(card.kind, CardKind::Answer | CardKind::Ask) {
        qa_log
            .lock()
            .unwrap()
            .push((card.id.clone(), format!("Q: {}\nA: {}", card.question, card.lead)));
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

/// The Zoom or Teams window titles the capture grabbed in the recording that
/// is ending, first-seen order. Empty when the assistant or "See the Zoom
/// window" is off. Read at Stop, before the next open drops the store.
pub async fn window_titles(handle: &AssistantHandle) -> Vec<(CallApp, String)> {
    let core = handle.0.lock().await;
    core.frames.as_ref().map(|f| f.window_titles()).unwrap_or_default()
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
    // The meeting saves the brief as it stood at Stop, so every edit lands in
    // the history at once, whether or not a session is open.
    core.history.lock().unwrap().set_brief(&text);
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
    // Between `recording-started` and the new lane the old lane is still
    // installed; touching it would announce the old meeting to the new history.
    if !core.session_open || !core.ask_lane.dismiss(&id) {
        return;
    }
    // A question Joseph dismissed is not saved with the meeting, nor kept in
    // the note's Q&A log. One the model merely dropped from a later list
    // stays, because it was shown. A session open means the collector holds a
    // new call, so there are no saved rows to reach.
    forget_live_items(&core, &[id]);
    if core.session_open && core.enabled {
        core.ask_lane.request(&core.transcript, false);
    }
}

/// Where a live item leaves the session's records, for every delete: the
/// history collector (so the meeting save never has it and no observe path
/// brings it back) and the note's Q&A log. Returns the removed entries' row ids.
fn forget_live_items(core: &AssistantCore, ids: &[String]) -> Vec<String> {
    let removed = core.history.lock().unwrap().delete(ids);
    core.qa_log.lock().unwrap().retain(|(id, _)| !ids.contains(id));
    removed
}

/// Deletes assistant items by their live ids (a card's id, a question's id, a
/// to-do's id), during the call or after Stop. They leave the collector, so
/// the meeting save never has them, and the note's Q&A log. With the session
/// open the ask lane also forgets them for good (same as the x); with it
/// closed the lane is not touched (see `dismiss_ask`). Holds the items write
/// gate throughout, so it never interleaves with the Stop save. When the
/// meeting was already saved at Stop, its saved rows are deleted first and
/// `assistant.json` and the vault note are rebuilt from the rows left; a
/// database failure there returns Err and changes nothing, so a retry still
/// finds the items. Once the rows are gone the collector, the Q&A log, the
/// lane and a ready note draft are updated even when rebuilding the files
/// failed (that Err comes back after, and a retry rebuilds them). A ready note
/// draft is made again without the items. Returns how many items were removed.
pub async fn delete_items(handle: &AssistantHandle, pool: &SqlitePool, ids: Vec<String>) -> Result<u32, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    let history = handle.0.lock().await.history.clone();
    let _gate = super::history::lock_items_gate(&history).await;
    let (saved_meeting, saved_ids) = {
        let h = history.lock().unwrap();
        (h.saved_meeting_id(), h.saved_ids_of(&ids))
    };
    let mut files = Ok(());
    if let Some(meeting_id) = &saved_meeting {
        let (_, gone) = super::history::delete_saved_rows(pool, meeting_id, &saved_ids).await?;
        files = super::history::after_items_deleted(pool, meeting_id, &gone).await;
        if let Err(e) = &files {
            log::warn!("assistant: rows of {} deleted but its files not rebuilt: {}", meeting_id, e);
        }
    }
    let removed = {
        let core = handle.0.lock().await;
        let removed = forget_live_items(&core, &ids);
        if core.session_open {
            let mut dismissed = false;
            for id in &ids {
                dismissed |= core.ask_lane.dismiss(id) || core.ask_lane.dismiss_todo(id);
            }
            if dismissed && core.enabled {
                core.ask_lane.request(&core.transcript, false);
            }
        }
        removed
    };
    redraft_if_ready(handle).await;
    files.map(|_| removed.len() as u32)
}

/// Deletes a past meeting's saved rows by row id (the Assistant tab). Holds
/// the items write gate throughout. When that meeting is the one the live
/// collector saved, its entries leave the collector too, the cards leave the
/// note's Q&A log, a ready note is made again without them, and `on_deleted`
/// gets every live id of the removed entries so the panel can hide them. A
/// database failure on the rows returns Err and changes nothing; a failure
/// rebuilding the files afterwards returns Err only after the live session
/// was updated, and a retry rebuilds them.
pub async fn delete_past_items(
    handle: &AssistantHandle,
    pool: &SqlitePool,
    meeting_id: &str,
    ids: Vec<String>,
    on_deleted: impl FnOnce(Vec<String>),
) -> Result<u32, String> {
    let history = handle.0.lock().await.history.clone();
    let _gate = super::history::lock_items_gate(&history).await;
    let (n, gone) = super::history::delete_saved_rows(pool, meeting_id, &ids).await?;
    let files = super::history::after_items_deleted(pool, meeting_id, &gone).await;
    if let Err(e) = &files {
        log::warn!("assistant: rows of {} deleted but its files not rebuilt: {}", meeting_id, e);
    }
    let live_ids = {
        let core = handle.0.lock().await;
        let mut history = core.history.lock().unwrap();
        if history.saved_meeting_id().as_deref() != Some(meeting_id) {
            return files.map(|_| n);
        }
        let live_ids = history.delete_saved(&ids);
        core.qa_log.lock().unwrap().retain(|(id, _)| !live_ids.contains(id));
        live_ids
    };
    if !live_ids.is_empty() {
        redraft_if_ready(handle).await;
        on_deleted(live_ids);
    }
    files.map(|_| n)
}

/// After a delete: a note still Ready is drafted again without the deleted
/// items, in the background. The draft starts only if the note is still Ready
/// when it takes the lock, so a Save or a draft that ran meanwhile is left alone.
async fn redraft_if_ready(handle: &AssistantHandle) {
    let ready = matches!(handle.0.lock().await.note_state, NoteState::Ready);
    if ready {
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move {
            let _ = draft_note_if_ready(&handle).await;
        });
    }
}

/// Joseph typed the answer to a list question. The item moves to the
/// answered list; the brief is the frontend's to update. False for an
/// unknown id or an empty answer.
pub async fn answer_ask(handle: &AssistantHandle, id: String, answer: String) -> bool {
    let core = handle.0.lock().await;
    // No open session on the current lane (see `dismiss_ask`): change nothing.
    if !core.session_open || !core.ask_lane.answer(&id, &answer) {
        return false;
    }
    if core.session_open && core.enabled {
        core.ask_lane.request(&core.transcript, false);
    }
    true
}

/// Joseph typed a to-do (`todo:` in the Ask box). It joins the list as a
/// You to-do stamped with the newest utterance's time; the next ask turn
/// lists it as typed so the model never adds it again. The error text goes
/// to the Ask box, so nothing is set on the status.
pub async fn add_todo(handle: &AssistantHandle, text: String) -> Result<(), String> {
    // The recording's active seconds (pauses left out), the same scale as the
    // transcript's audio times. None when no recording is reachable.
    let active = crate::audio::recording_commands::get_recording_state().await["active_duration"].as_f64();
    add_todo_at(handle, text, active).await
}

/// `add_todo` with the recording's active seconds passed in, so a test can
/// set them. The to-do's call time is that recording clock, never earlier
/// than the newest utterance's start, and just that start when the active
/// time is unknown. Wall clock would run ahead after a pause: transcript
/// times come from audio samples, which stop while the recording is paused.
async fn add_todo_at(handle: &AssistantHandle, text: String, active_secs: Option<f64>) -> Result<(), String> {
    let text = super::ask_list::collapse(&text);
    if text.is_empty() {
        return Err("Type the to-do after todo:".to_string());
    }
    let core = handle.0.lock().await;
    if !core.session_open || !core.enabled {
        return Err("Start a recording with the assistant on to add a to-do.".to_string());
    }
    let latest = core.transcript.latest_start();
    let call_time = active_secs.filter(|s| s.is_finite()).map_or(latest, |s| latest.max(s));
    core.ask_lane.add_todo(&text, call_time);
    Ok(())
}

/// Joseph dismissed a to-do. An unknown id, or no open session on the
/// current lane (see `dismiss_ask`), changes nothing.
pub async fn dismiss_todo(handle: &AssistantHandle, id: String) {
    let core = handle.0.lock().await;
    if !core.session_open || !core.ask_lane.dismiss_todo(&id) {
        return;
    }
    if core.session_open && core.enabled {
        core.ask_lane.request(&core.transcript, false);
    }
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
    draft_note_gated(handle, false).await
}

/// The draft a delete asks for: runs only if the note is still Ready when it
/// takes the lock (Saved, or a draft already running, is left as it is).
async fn draft_note_if_ready(handle: &AssistantHandle) -> Result<(), String> {
    draft_note_gated(handle, true).await
}

/// Marks the note Drafting and builds the draft's inputs from what is left
/// now (the transcript, the Q&A log, the answered and open questions less the
/// deleted ones). Returns them with the collector's delete generation as it
/// stood, so a delete that lands while the draft runs can be told apart.
fn start_draft(core: &mut AssistantCore) -> (String, String, u64) {
    core.note_state = NoteState::Drafting;
    core.note_error = None;
    core.emit_note();
    let transcript_text = core.transcript.all();
    let qa_log_text = qa_log_text(&core.qa_log);
    // After Stop the lane still holds what Joseph deleted: leave it out.
    let (deleted, generation) = {
        let h = core.history.lock().unwrap();
        (h.deleted_ids(), h.delete_generation())
    };
    let open_asks = open_asks_kept(&core.ask_lane, &deleted);
    let mut draft_input = qa_log_text;
    let answered = answered_kept(&core.ask_lane, &deleted);
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
    (transcript_text, draft_input, generation)
}

async fn draft_note_gated(handle: &AssistantHandle, only_if_ready: bool) -> Result<(), String> {
    let (mut transcript_text, mut qa_log_text, mut generation) = {
        let mut core = handle.0.lock().await;
        if only_if_ready && !matches!(core.note_state, NoteState::Ready) {
            return Ok(());
        }
        if core.session_open {
            return Err("stop the recording before drafting a note".to_string());
        }
        start_draft(&mut core)
    };

    loop {
        let result = {
            let mut core = handle.0.lock().await;
            core.lanes.draft_note(transcript_text, qa_log_text).await
        };

        let mut core = handle.0.lock().await;
        match result {
            Ok(raw) => match note::parse_note(&raw) {
                Ok(draft) => {
                    // An item deleted while this draft ran may be named in the
                    // model's text, which was written from the old inputs: draft
                    // again from what is left, staying Drafting until that lands.
                    if core.history.lock().unwrap().delete_generation() != generation {
                        if core.session_open {
                            // A new call opened meanwhile and reset the note.
                            return Ok(());
                        }
                        (transcript_text, qa_log_text, generation) = start_draft(&mut core);
                        continue;
                    }
                    // The preview carries the answered list as it stands, built in
                    // code. The draft keeps only the model's body: the saved file
                    // builds its own code sections from the meeting's rows.
                    let deleted = core.history.lock().unwrap().deleted_ids();
                    core.note_markdown = note::with_answered(&draft.markdown, &answered_kept(&core.ask_lane, &deleted));
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
        return Ok(());
    }
}

/// What the vault note is built from: the meeting's rows (saved rows when the
/// meeting was saved at Stop, else the live collector's), its title, the Brief.
struct MeetingFacts {
    saved_id: Option<String>,
    rows: Vec<AssistantItem>,
    title: Option<String>,
    brief: String,
}

async fn meeting_facts(pool: &SqlitePool, history: &super::history::HistoryHandle) -> Result<MeetingFacts, String> {
    let (saved_id, live_brief, live_rows) = {
        let h = history.lock().unwrap();
        let saved_id = h.saved_meeting_id();
        let live_rows = if saved_id.is_none() { h.finalize(h.opened_ms()) } else { Vec::new() };
        (saved_id, h.brief().to_string(), live_rows)
    };
    let Some(id) = saved_id.clone() else {
        return Ok(MeetingFacts { saved_id, rows: live_rows, title: None, brief: live_brief });
    };
    let rows = AssistantItemsRepository::list_for_meeting(pool, &id).await.map_err(|e| e.to_string())?;
    let title = sqlx::query_scalar::<_, Option<String>>("SELECT title FROM meetings WHERE id = ?")
        .bind(&id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|t| !t.trim().is_empty());
    // After Stop the collector's Brief belongs to the NEXT call, so a saved
    // meeting takes the brief it was saved with.
    let brief = AssistantMetaRepository::get(pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .map(|m| m.brief)
        .unwrap_or_default();
    Ok(MeetingFacts { saved_id, rows, title, brief })
}

/// Slugs of the vault's project pages, for the note bar's dropdown.
pub async fn list_vault_projects(handle: &AssistantHandle) -> Vec<String> {
    let root = resolve_vault_root(&handle.0.lock().await.settings.vault_root);
    note::list_projects(&root)
}

/// The project the finished meeting looks like it belongs to, if any.
pub async fn suggest_vault_project(handle: &AssistantHandle, pool: &SqlitePool) -> Option<String> {
    let (root, history, transcript) = {
        let core = handle.0.lock().await;
        let transcript = if core.session_transcript.is_empty() { core.transcript.all() } else { core.session_transcript.clone() };
        (resolve_vault_root(&core.settings.vault_root), core.history.clone(), transcript)
    };
    let facts = match meeting_facts(pool, &history).await {
        Ok(f) => f,
        Err(e) => {
            log::warn!("assistant: no vault project suggestion, meeting data unreadable: {}", e);
            return None;
        }
    };
    note::suggest_project(&root, facts.title.as_deref().unwrap_or(""), &facts.brief, &transcript)
}

/// What a save returns to the note bar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedNote {
    pub note_path: String,
    pub project: Option<String>,
}

/// Files the drafted note in the vault under `project` (None: no project) and
/// links it on the project page. A project that is not a page in the vault is
/// refused with an error and the note stays Ready; any other failure puts the
/// note in Failed with its message, as it always did.
pub async fn save_note(
    handle: &AssistantHandle,
    pool: &SqlitePool,
    project: Option<String>,
) -> Result<SavedNote, String> {
    let mut core = handle.0.lock().await;
    let Some(draft) = core.note_draft.clone() else {
        core.note_state = NoteState::Failed;
        core.note_error = Some("no note draft to save".to_string());
        core.emit_note();
        return Err("no note draft to save".to_string());
    };
    let vault_root = resolve_vault_root(&core.settings.vault_root);
    note::check_project(&vault_root, project.as_deref())?;
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let dry_run = std::env::var("ASSISTANT_DRY_RUN").ok().as_deref() == Some("1");
    let transcript_text = core.session_transcript.clone();
    let history = core.history.clone();

    let result = async {
        let facts = meeting_facts(pool, &history).await?;
        let filed = note::save_note(note::SaveInput {
            vault_root: &vault_root,
            date: &date,
            draft: &draft,
            title: facts.title.as_deref(),
            brief: &facts.brief,
            rows: &facts.rows,
            transcript: &transcript_text,
            project: project.as_deref(),
            dry_run,
        })?;
        Ok::<_, String>((filed, facts.saved_id))
    }
    .await;

    match result {
        Ok((filed, saved_id)) => {
            if dry_run {
                log::info!("assistant: dry-run, not written: {:?}", filed);
            } else {
                log::info!("assistant: note saved: {:?}", filed);
                if let Some(id) = saved_id {
                    let filed_at = chrono::Utc::now().to_rfc3339();
                    if let Err(e) = AssistantMetaRepository::set_filed(
                        pool,
                        &id,
                        filed.project.as_deref(),
                        &filed.note_path.to_string_lossy(),
                        &filed.transcript_path.to_string_lossy(),
                        &filed_at,
                    )
                    .await
                    {
                        log::warn!("assistant: filed the note but could not record it for {}: {}", id, e);
                    }
                }
            }
            core.note_state = NoteState::Saved;
            core.note_error = None;
            core.emit_note();
            Ok(SavedNote { note_path: filed.note_path.to_string_lossy().into_owned(), project: filed.project })
        }
        Err(e) => {
            core.note_state = NoteState::Failed;
            core.note_error = Some(e.clone());
            core.emit_note();
            Err(e)
        }
    }
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
    core.history.lock().unwrap().set_brief(&brief);
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

/// The brief the next session opens with: what was typed while no session
/// was open, taken once. `None` when nothing was typed since the last call.
async fn take_pending_brief(handle: &AssistantHandle) -> Option<String> {
    let mut core = handle.0.lock().await;
    core.brief_edited = false;
    core.pending_brief.take()
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
        core.note_state = NoteState::Idle;
        core.note_draft = None;
        core.note_markdown.clear();
        core.note_error = None;
        // The panel must not keep the last call's note state (a Saved note
        // would make the next Stop skip the note bar).
        core.emit_note();
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
    let brief = take_pending_brief(handle).await;

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

/// The Settings switch "see the Zoom or Teams window", applied to a running call.
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
            frames_dirs(app.as_ref()).await
        } else {
            None
        }
    } else {
        None
    };
    apply_see_zoom_window_in(handle, on, dir).await;
}

/// `apply_see_zoom_window` with the frames folder already resolved.
async fn apply_see_zoom_window_in(
    handle: &AssistantHandle,
    on: bool,
    dirs: Option<(PathBuf, PathBuf)>,
) {
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
    let Some((zoom_dir, teams_dir)) = dirs else {
        log::warn!("assistant: no folder for call frames, running without sight");
        return;
    };
    let store = FrameStore::new();
    core.capture = Some(capture::start(
        CaptureConfig::new(zoom_dir, teams_dir),
        store.clone(),
    ));
    core.frames = Some(store.clone());
    let feed = FrameFeed::new(store);
    core.lanes.set_frames(Some(feed.clone()));
    core.ask_lane.set_frames(Some(feed));
}

/// Where this recording's kept frames are saved, as (Zoom folder, Teams
/// folder): `zoom-frames` and `teams-frames` inside the meeting folder, or
/// when the folder is not known yet, `zoom-frames/<date_time>` and
/// `teams-frames/<date_time>` under the app data dir. Never the temp session
/// dir. A folder is only made when its first frame is saved.
async fn frames_dirs(app: Option<&AppHandle<Wry>>) -> Option<(PathBuf, PathBuf)> {
    if let Ok(Some(folder)) = crate::audio::recording_commands::get_meeting_folder_path().await {
        let folder = PathBuf::from(folder);
        return Some((folder.join("zoom-frames"), folder.join("teams-frames")));
    }
    let base = app?.path().app_data_dir().ok()?;
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H%M%S").to_string();
    Some((
        base.join("zoom-frames").join(&stamp),
        base.join("teams-frames").join(&stamp),
    ))
}

/// Makes this recording's frame store and starts the capture task (it
/// returns at once). A missing folder only means no frames: the assistant
/// carries on without sight.
async fn start_capture(handle: &AssistantHandle, app: Option<&AppHandle<Wry>>) {
    let Some((zoom_dir, teams_dir)) = frames_dirs(app).await else {
        log::warn!("assistant: no folder for call frames, running without sight");
        return;
    };
    let store = FrameStore::new();
    let capture = capture::start(CaptureConfig::new(zoom_dir, teams_dir), store.clone());
    let mut core = handle.0.lock().await;
    core.frames = Some(store);
    core.capture = Some(capture);
}

/// Bound to `recording-stopped`. Leaves the transcript and Q&A log in place
/// for `draft_note`; aborts any turn still in flight.
async fn close_session(handle: &AssistantHandle) {
    let mut core = handle.0.lock().await;
    core.stop_capture();
    // The call is over, so its brief is not the next call's: whatever a
    // mid-call switch-off left pending goes too. A brief typed after this
    // lands in `pending_brief` again and opens the next session.
    core.pending_brief = None;
    core.brief_edited = false;
    if !core.session_open {
        return;
    }
    core.session_open = false;
    core.lanes.close();
    core.ask_lane.close();
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
        let note_app = app.clone();
        core.note_emit = Arc::new(move |out: NoteOut| {
            if let Err(e) = note_app.emit("assistant-note", &out) {
                log::warn!("assistant: failed to emit assistant-note: {}", e);
            }
        });
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
        let history = app.state::<super::history::HistoryHandle>().inner().clone();
        app.listen("recording-started", move |_event| {
            log::info!("assistant: recording-started event received");
            // A new recording starts a new history, and its start time is now.
            // Only here: a mid-call off/on reopens the session without this.
            history.lock().unwrap().reset(chrono::Utc::now().timestamp_millis());
            let handle = handle.clone();
            let history = history.clone();
            tauri::async_runtime::spawn(async move {
                // A brief typed before recording waits in `pending_brief`; the
                // reset above cleared the history's copy, so put it back even
                // when the assistant is off and no session will take it.
                let pending = handle.0.lock().await.pending_brief.clone();
                if let Some(brief) = pending {
                    history.lock().unwrap().set_brief(&brief);
                }
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
    async fn window_titles_are_this_recordings_and_empty_without_sight() {
        let handle = AssistantHandle::new();
        assert!(window_titles(&handle).await.is_empty(), "no store, no titles");

        let store = FrameStore::new();
        store.record_title(CallApp::Teams, "Storage roadmap review | Microsoft Teams");
        {
            let mut core = handle.0.lock().await;
            core.frames = Some(store.clone());
            core.session_open = true;
        }
        close_session(&handle).await;
        assert_eq!(
            window_titles(&handle).await,
            vec![(CallApp::Teams, "Storage roadmap review | Microsoft Teams".to_string())],
            "readable after the recording-stopped close, as Stop needs"
        );

        // The next recording opens with no store, so it cannot see these.
        set_enabled(&handle, false).await;
        assert!(window_titles(&handle).await.is_empty());
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
        apply_see_zoom_window_in(&handle, true, Some((dir.clone(), dir.join("teams-frames")))).await;
        {
            let core = handle.0.lock().await;
            assert!(core.capture.is_some());
            assert!(core.frames.is_some());
            assert!(core.lanes.has_frames() && core.ask_lane.has_frames());
        }

        // On again: nothing restarts.
        apply_see_zoom_window_in(&handle, true, Some((dir.clone(), dir.join("teams-frames")))).await;
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
        apply_see_zoom_window_in(&handle, true, Some((std::env::temp_dir(), std::env::temp_dir()))).await;
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
    async fn every_brief_edit_reaches_the_history_with_or_without_a_session() {
        let handle = AssistantHandle::new();
        let history = handle.0.lock().await.history_handle_for_test();
        set_brief(&handle, "before".to_string()).await;
        assert_eq!(history.lock().unwrap().brief(), "before");
        handle.0.lock().await.session_open = true;
        set_brief(&handle, "during".to_string()).await;
        assert_eq!(history.lock().unwrap().brief(), "during");
        set_brief(&handle, String::new()).await;
        assert_eq!(history.lock().unwrap().brief(), "");
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

    /// The closing call's own brief does not open the next call.
    #[tokio::test]
    async fn the_next_session_does_not_inherit_the_closing_calls_brief() {
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
            core.ask_hooks = hooks;
        }
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), "goal A".to_string(), ask_runner(calls.clone())).await;
        close_session(&handle).await;
        assert_eq!(handle.0.lock().await.pending_brief, None);

        let brief = take_pending_brief(&handle).await.unwrap_or_default();
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), brief, ask_runner(calls)).await;
        let core = handle.0.lock().await;
        assert_eq!(core.brief, "");
        assert_eq!(core.ask_lane.snapshot().brief, "");
        assert_eq!(core.history.lock().unwrap().brief(), "");
        assert_eq!(emitted.lock().unwrap().last().unwrap().brief, "");
    }

    /// A brief typed after the previous call's Stop is the new call's brief.
    #[tokio::test]
    async fn a_brief_typed_between_calls_is_the_next_calls_brief() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), "goal A".to_string(), ask_runner(calls.clone())).await;
        close_session(&handle).await;
        set_brief(&handle, "goal B".to_string()).await;

        let brief = take_pending_brief(&handle).await.unwrap_or_default();
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), brief, ask_runner(calls)).await;
        let core = handle.0.lock().await;
        assert_eq!(core.brief, "goal B");
        assert_eq!(core.ask_lane.snapshot().brief, "goal B");
        assert_eq!(core.history.lock().unwrap().brief(), "goal B");
    }

    /// Switching the assistant off in a call and closing it at Stop leaves
    /// nothing for the next call either.
    #[tokio::test]
    async fn stop_after_a_mid_call_switch_off_leaves_no_brief_for_the_next_call() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), "goal A".to_string(), ask_runner(calls)).await;
        set_enabled(&handle, false).await;
        assert_eq!(handle.0.lock().await.pending_brief.as_deref(), Some("goal A"), "off and on again in one call keeps the brief");
        close_session(&handle).await;
        assert_eq!(handle.0.lock().await.pending_brief, None);
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
    async fn add_todo_needs_text_and_a_live_assistant_then_joins_the_list() {
        let handle = AssistantHandle::new();
        assert_eq!(add_todo(&handle, "  \n ".to_string()).await, Err("Type the to-do after todo:".to_string()));
        assert_eq!(
            add_todo(&handle, "Send Dana the runbook".to_string()).await,
            Err("Start a recording with the assistant on to add a to-do.".to_string())
        );
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls.clone())).await;
        {
            let mut core = handle.0.lock().await;
            core.transcript.ingest(&tu("system", "hello there", 2405.0, 2406.0));
            core.transcript.ingest(&tu("mic", "ok", 2430.5, 2431.0));
        }
        assert_eq!(add_todo(&handle, "  Send   Dana the runbook ".to_string()).await, Ok(()));
        assert_eq!(add_todo(&handle, "Send Dana the runbook".to_string()).await, Ok(()));
        let out = handle.0.lock().await.ask_lane.snapshot();
        assert_eq!(out.todos.len(), 1, "an exact repeat is a no-op");
        assert_eq!(out.todos[0].text, "Send Dana the runbook");
        assert_eq!(out.todos[0].call_time, 2430.5);

        set_enabled(&handle, false).await;
        assert_eq!(
            add_todo(&handle, "Another".to_string()).await,
            Err("Start a recording with the assistant on to add a to-do.".to_string())
        );
    }

    #[tokio::test]
    async fn a_typed_todo_is_stamped_with_the_active_recording_time_not_wall_clock() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls)).await;
        handle.0.lock().await.transcript.ingest(&tu("mic", "ok", 18.0, 19.0));
        let at = |h: &AssistantHandle, text: &str| {
            h.0.try_lock().unwrap().ask_lane.snapshot().todos.iter().find(|t| t.text == text).unwrap().call_time
        };

        // Recording state not reachable: the newest utterance's start.
        add_todo_at(&handle, "No state".to_string(), None).await.unwrap();
        assert_eq!(at(&handle, "No state"), 18.0);

        // 10 min recorded, 15 min paused, 1 min more: active time is 11:00
        // (660 s) where wall clock would read 26:00 (1560 s).
        add_todo_at(&handle, "Send Dana the runbook".to_string(), Some(660.0)).await.unwrap();
        assert_eq!(at(&handle, "Send Dana the runbook"), 660.0);

        // Active time behind the newest utterance never goes earlier than it.
        handle.0.lock().await.transcript.ingest(&tu("system", "late", 700.0, 701.0));
        add_todo_at(&handle, "Book the venue".to_string(), Some(660.0)).await.unwrap();
        assert_eq!(at(&handle, "Book the venue"), 700.0);
    }

    #[tokio::test]
    async fn answering_between_recording_started_and_the_new_lane_leaks_nothing() {
        let handle = AssistantHandle::new();
        let history = handle.0.lock().await.history_handle_for_test();
        {
            let sink = history.clone();
            handle.0.lock().await.ask_hooks.emit =
                Arc::new(move |out| super::super::history::record_asks(&sink, &out));
        }
        let calls = Arc::new(StdMutex::new(Vec::new()));
        history.lock().unwrap().reset(1_000); // meeting 1: recording-started
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls)).await;
        refresh_asks(&handle).await;
        settle().await;
        let id = handle.0.lock().await.ask_lane.snapshot().asks[0].id.clone();
        close_session(&handle).await; // meeting 1: recording-stopped

        history.lock().unwrap().reset(50_000); // meeting 2: recording-started
        assert!(!answer_ask(&handle, id, "Ten percent".to_string()).await);
        assert!(history.lock().unwrap().is_empty(), "the fresh collector holds nothing from the old lane");
        assert!(handle.0.lock().await.ask_lane.snapshot().answered.is_empty());
    }

    #[tokio::test]
    async fn dismiss_todo_removes_it_and_tells_the_model_once() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls.clone())).await;
        add_todo(&handle, "Send Dana the runbook".to_string()).await.unwrap();
        let id = handle.0.lock().await.ask_lane.snapshot().todos[0].id.clone();

        dismiss_todo(&handle, "nope".to_string()).await;
        assert_eq!(handle.0.lock().await.ask_lane.snapshot().todos.len(), 1);
        dismiss_todo(&handle, id.clone()).await;
        settle().await;

        let out = handle.0.lock().await.ask_lane.snapshot();
        assert!(out.todos.is_empty());
        assert_eq!(out.dismissed_todos, vec![id]);
        let turns = ask_turns(&calls);
        assert!(turns.last().unwrap().contains("DISMISSED TO-DOS (not to-dos, never add them again):\n- Send Dana the runbook"));
    }

    #[tokio::test]
    async fn a_new_meeting_starts_with_no_todos() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls)).await;
        add_todo(&handle, "Send Dana the runbook".to_string()).await.unwrap();
        let mut core = handle.0.lock().await;
        core.reset_ask_lane();
        let out = core.ask_lane.snapshot();
        assert!(out.todos.is_empty() && out.dismissed_todos.is_empty());
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
        assert!(!core.note_draft.as_ref().unwrap().markdown.contains("Answered questions"), "the saved file builds its own from the rows");
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
    async fn a_mid_call_off_and_on_keeps_everything_shown_earlier_in_the_call() {
        let handle = AssistantHandle::new();
        let history = handle.0.lock().await.history_handle_for_test();
        history.lock().unwrap().reset(1_000); // recording-started
        let root = std::env::temp_dir().join(format!("meetily-reopen-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let card = |id: &str, ts: i64| CardOut {
            id: id.into(),
            kind: super::super::lanes::CardKind::Answer,
            question: "Q?".into(),
            lead: "lead".into(),
            bullets: vec![],
            source: String::new(),
            phase: "checked".into(),
            changed_lines: vec![],
            ts,
        };
        open_session_in(&handle, Some(true), &root).await;
        super::super::history::record_card(&history, &card("c1", 5_000));
        // Turned off, then on again, in the same call.
        open_session_in(&handle, Some(false), &root).await;
        open_session_in(&handle, Some(true), &root).await;
        super::super::history::record_card(&history, &card("c2", 9_000));
        let _ = std::fs::remove_dir_all(&root);
        let items = history.lock().unwrap().finalize(1_000);
        assert_eq!(items.len(), 2, "both cards survive the reopen");
        assert_eq!(items[0].call_time, 4.0);
        assert_eq!(items[1].call_time, 8.0);
        assert_eq!(history.lock().unwrap().opened_ms(), 1_000, "fallback start stays the recording start");
    }

    #[tokio::test]
    async fn the_last_meetings_ask_items_never_reach_the_next_meetings_history() {
        let handle = AssistantHandle::new();
        let history = handle.0.lock().await.history_handle_for_test();
        {
            // The same wiring the app has: every Ask snapshot is recorded.
            let sink = history.clone();
            handle.0.lock().await.ask_hooks.emit =
                Arc::new(move |out| super::super::history::record_asks(&sink, &out));
        }
        let calls = Arc::new(StdMutex::new(Vec::new()));
        history.lock().unwrap().reset(1_000); // meeting 1: recording-started
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls.clone())).await;
        refresh_asks(&handle).await;
        settle().await;
        refresh_asks(&handle).await;
        settle().await;
        assert!(!history.lock().unwrap().is_empty(), "meeting 1 collected its Ask items");
        assert_eq!(handle.0.lock().await.ask_lane.snapshot().answered.len(), 1);
        close_session(&handle).await; // meeting 1: recording-stopped

        // Meeting 2: recording-started resets the collector, then the session
        // opens (whatever it emits while resetting the lane must not land).
        history.lock().unwrap().reset(50_000);
        let root = std::env::temp_dir().join(format!("meetily-leak2-test-{}", uuid::Uuid::new_v4()));
        std::fs::write(&root, b"x").unwrap();
        open_session_in(&handle, Some(true), &root).await;
        let _ = std::fs::remove_file(&root);
        super::super::history::record_card(
            &history,
            &CardOut {
                id: "m2".into(),
                kind: super::super::lanes::CardKind::Answer,
                question: "Only question".into(),
                lead: "lead".into(),
                bullets: vec![],
                source: String::new(),
                phase: "checked".into(),
                changed_lines: vec![],
                ts: 60_000,
            },
        );
        let items = history.lock().unwrap().finalize(50_000);
        let seen: Vec<_> = items.iter().map(|i| i.question.clone()).collect();
        assert_eq!(items.len(), 1, "only meeting 2's own item is saved, got {:?}", seen);
    }

    #[tokio::test]
    async fn a_dismiss_or_add_between_recording_started_and_the_new_lane_leaks_nothing() {
        let handle = AssistantHandle::new();
        let history = handle.0.lock().await.history_handle_for_test();
        {
            let sink = history.clone();
            handle.0.lock().await.ask_hooks.emit =
                Arc::new(move |out| super::super::history::record_asks(&sink, &out));
        }
        let calls = Arc::new(StdMutex::new(Vec::new()));
        history.lock().unwrap().reset(1_000); // meeting 1: recording-started
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls)).await;
        add_todo(&handle, "Send Dana the runbook".to_string()).await.unwrap();
        add_todo(&handle, "Book the venue".to_string()).await.unwrap();
        let ids: Vec<String> = handle.0.lock().await.ask_lane.snapshot().todos.iter().map(|t| t.id.clone()).collect();
        close_session(&handle).await; // meeting 1: recording-stopped

        // Meeting 2: recording-started has reset the collector, but the old
        // lane is still installed until open_session replaces it.
        history.lock().unwrap().reset(50_000);
        dismiss_todo(&handle, ids[0].clone()).await;
        assert_eq!(
            add_todo(&handle, "Another".to_string()).await,
            Err("Start a recording with the assistant on to add a to-do.".to_string())
        );
        assert_eq!(handle.0.lock().await.ask_lane.snapshot().todos.len(), 2, "the old lane was not touched");
        assert!(history.lock().unwrap().is_empty(), "the fresh collector holds nothing from the old lane");
        assert!(history.lock().unwrap().finalize(50_000).is_empty());
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

    // ---- deleting items ----

    use crate::database::repositories::assistant_items::tests::{add_meeting, test_pool};
    use crate::database::repositories::assistant_items::AssistantItemsRepository;

    /// Wires the ask hook the way the app does, so every snapshot is collected.
    async fn collect_asks(handle: &AssistantHandle) -> super::super::history::HistoryHandle {
        let history = handle.0.lock().await.history_handle_for_test();
        let sink = history.clone();
        handle.0.lock().await.ask_hooks.emit = Arc::new(move |out| super::super::history::record_asks(&sink, &out));
        history.lock().unwrap().reset(0);
        history
    }

    async fn saved_questions(pool: &SqlitePool, meeting: &str) -> Vec<String> {
        AssistantItemsRepository::list_for_meeting(pool, meeting)
            .await
            .unwrap()
            .into_iter()
            .map(|i| i.question)
            .collect()
    }

    #[tokio::test]
    async fn a_dismissed_question_is_not_saved_but_an_undismissed_one_is() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", None).await;
        add_meeting(&pool, "m2", None).await;
        for (meeting, dismiss) in [("m1", true), ("m2", false)] {
            let handle = AssistantHandle::new();
            let history = collect_asks(&handle).await;
            let calls = Arc::new(StdMutex::new(Vec::new()));
            open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls)).await;
            refresh_asks(&handle).await;
            settle().await;
            let id = handle.0.lock().await.ask_lane.snapshot().asks[0].id.clone();
            if dismiss {
                dismiss_ask(&handle, id).await;
                settle().await;
            }
            close_session(&handle).await;
            super::super::history::save_for_meeting(&pool, &history, meeting).await.unwrap();
            let saved = saved_questions(&pool, meeting).await;
            if dismiss {
                assert!(saved.is_empty(), "dismissed question was saved: {:?}", saved);
            } else {
                assert_eq!(saved, vec!["What are your fees?"]);
            }
        }
    }

    #[tokio::test]
    async fn deleting_an_open_question_during_the_call_removes_it_for_good() {
        let pool = test_pool().await;
        let handle = AssistantHandle::new();
        let history = collect_asks(&handle).await;
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls.clone())).await;
        refresh_asks(&handle).await;
        settle().await;
        let id = handle.0.lock().await.ask_lane.snapshot().asks[0].id.clone();
        assert_eq!(history.lock().unwrap().finalize(0).len(), 1);

        assert_eq!(delete_items(&handle, &pool, vec![id.clone()]).await, Ok(1));
        settle().await;
        assert!(handle.0.lock().await.ask_lane.snapshot().asks.is_empty());
        assert!(history.lock().unwrap().finalize(0).is_empty());
        assert!(ask_turns(&calls).len() >= 2, "the lane was told it was dismissed");
        assert_eq!(delete_items(&handle, &pool, vec![id]).await, Ok(0), "already gone");
        assert_eq!(delete_items(&handle, &pool, vec![]).await, Ok(0));
    }

    #[tokio::test]
    async fn deleting_a_todo_during_the_call_dismisses_it_on_the_lane() {
        let pool = test_pool().await;
        let handle = AssistantHandle::new();
        let history = collect_asks(&handle).await;
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls)).await;
        add_todo(&handle, "Send Dana the runbook".to_string()).await.unwrap();
        let id = handle.0.lock().await.ask_lane.snapshot().todos[0].id.clone();
        assert_eq!(history.lock().unwrap().finalize(0).len(), 1);

        assert_eq!(delete_items(&handle, &pool, vec![id.clone()]).await, Ok(1));
        let out = handle.0.lock().await.ask_lane.snapshot();
        assert!(out.todos.is_empty());
        assert_eq!(out.dismissed_todos, vec![id]);
        assert!(history.lock().unwrap().finalize(0).is_empty());
    }

    #[tokio::test]
    async fn deleting_after_stop_updates_the_saved_meeting_and_leaves_the_lane_alone() {
        let pool = test_pool().await;
        let dir = tempfile::tempdir().unwrap();
        add_meeting(&pool, "m1", Some(dir.path().to_str().unwrap())).await;
        let handle = AssistantHandle::new();
        let history = collect_asks(&handle).await;
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls.clone())).await;
        refresh_asks(&handle).await;
        settle().await;
        refresh_asks(&handle).await;
        settle().await;
        let answered_id = handle.0.lock().await.ask_lane.snapshot().answered[0].id.clone();
        close_session(&handle).await;
        super::super::history::save_for_meeting(&pool, &history, "m1").await.unwrap();
        assert_eq!(saved_questions(&pool, "m1").await.len(), 1);
        assert!(dir.path().join("assistant.json").exists());
        let turns_before = calls.lock().unwrap().len();

        assert_eq!(delete_items(&handle, &pool, vec![answered_id]).await, Ok(1));
        assert!(saved_questions(&pool, "m1").await.is_empty());
        assert!(!dir.path().join("assistant.json").exists(), "no rows left, no file");
        assert_eq!(handle.0.lock().await.ask_lane.snapshot().answered.len(), 1, "a closed session's lane is not touched");
        settle().await;
        assert_eq!(calls.lock().unwrap().len(), turns_before, "no turn ran");
    }

    fn git_vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let st = std::process::Command::new("git").arg("-C").arg(dir.path()).args(args).status().unwrap();
            assert!(st.success(), "git {args:?}");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "T"]);
        std::fs::create_dir_all(dir.path().join("Wiki/Projects")).unwrap();
        std::fs::write(dir.path().join("Wiki/Projects/mission-control.md"), "# MC\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-q", "-m", "seed"]);
        dir
    }

    #[tokio::test]
    async fn saving_files_the_saved_rows_under_the_project_and_a_later_delete_updates_the_note() {
        let pool = test_pool().await;
        let folder = tempfile::tempdir().unwrap();
        add_meeting(&pool, "m1", Some(folder.path().to_str().unwrap())).await;
        let vault = git_vault();
        let handle = AssistantHandle::new();
        let history = collect_asks(&handle).await;
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls)).await;
        refresh_asks(&handle).await;
        settle().await;
        refresh_asks(&handle).await;
        settle().await;
        let answered_id = handle.0.lock().await.ask_lane.snapshot().answered[0].id.clone();
        close_session(&handle).await;
        history.lock().unwrap().set_brief("Mission Control review");
        super::super::history::save_for_meeting(&pool, &history, "m1").await.unwrap();
        handle.0.lock().await.settings.vault_root = vault.path().to_string_lossy().into_owned();
        assert!(draft_note(&handle).await.is_ok());

        assert_eq!(list_vault_projects(&handle).await, vec!["mission-control"]);
        let err = save_note(&handle, &pool, Some("../secret".to_string())).await.unwrap_err();
        assert!(err.contains("not a project"), "{err}");
        assert_eq!(handle.0.lock().await.note_state, NoteState::Ready, "a refusal leaves the note to save again");

        let saved = save_note(&handle, &pool, Some("mission-control".to_string())).await.unwrap();
        assert_eq!(saved.project.as_deref(), Some("mission-control"));
        let note = std::fs::read_to_string(&saved.note_path).unwrap();
        assert!(note.starts_with("# t\n\n- Date: "), "title is the meeting's: {note}");
        assert!(note.contains("- Linked projects: [[mission-control]]"));
        assert!(note.contains("## Brief\n\nMission Control review"));
        assert_eq!(note.matches("## Answered questions").count(), 1, "{note}");
        assert!(note.contains("- **What are your fees?** Ten percent of rent. (On the call)"));
        let page = std::fs::read_to_string(vault.path().join("Wiki/Projects/mission-control.md")).unwrap();
        assert!(page.contains("## Meetings\n\n- "), "{page}");
        let meta = AssistantMetaRepository::get(&pool, "m1").await.unwrap().unwrap();
        assert_eq!(meta.vault_project.as_deref(), Some("mission-control"));
        assert_eq!(meta.vault_note_path.as_deref(), Some(saved.note_path.as_str()));
        assert!(meta.filed_at.is_some());
        assert_eq!(handle.0.lock().await.note_state, NoteState::Saved);

        // A delete made after the save reaches the filed note.
        assert_eq!(delete_items(&handle, &pool, vec![answered_id]).await, Ok(1));
        let note = std::fs::read_to_string(&saved.note_path).unwrap();
        assert!(!note.contains("What are your fees?"), "{note}");
        assert!(note.contains("## Brief"), "the rest stays: {note}");
    }

    fn settled_card(id: &str, question: &str) -> CardOut {
        CardOut {
            id: id.to_string(),
            kind: CardKind::Answer,
            question: question.to_string(),
            lead: "Building two.".to_string(),
            bullets: vec![],
            source: "deep".to_string(),
            phase: "checked".to_string(),
            changed_lines: Vec::new(),
            ts: 1,
        }
    }

    fn last_note_prompt(calls: &Arc<StdMutex<Vec<crate::assistant::claude_cli::ClaudeTurn>>>) -> String {
        calls
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|t| t.prompt.starts_with("TRANSCRIPT:"))
            .map(|t| t.prompt.clone())
            .expect("a note turn ran")
    }

    #[tokio::test]
    async fn a_deleted_card_and_answered_question_stay_out_of_the_draft_and_a_ready_note_is_redrafted() {
        let pool = test_pool().await;
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls.clone())).await;
        refresh_asks(&handle).await;
        settle().await;
        refresh_asks(&handle).await;
        settle().await;
        let answered_id = {
            let core = handle.0.lock().await;
            let (qa, chats) = (core.qa_log.clone(), core.chat_log.clone());
            record_card(&settled_card("c1", "Where is the data center?"), &qa, &chats);
            record_card(&settled_card("c2", "Who owns backups?"), &qa, &chats);
            core.ask_lane.snapshot().answered[0].id.clone()
        };
        close_session(&handle).await;
        assert!(draft_note(&handle).await.is_ok());
        let first = last_note_prompt(&calls);
        assert!(first.contains("Where is the data center?") && first.contains("What are your fees?"), "{first}");
        assert!(handle.0.lock().await.note_markdown.contains("Answered questions"));

        // The note is Ready, so the delete drafts it again without them.
        assert_eq!(delete_items(&handle, &pool, vec!["c1".to_string(), answered_id]).await, Ok(0));
        settle().await;
        settle().await;
        let again = last_note_prompt(&calls);
        assert!(!again.contains("Where is the data center?"), "{again}");
        assert!(!again.contains("What are your fees?"), "{again}");
        assert!(again.contains("Who owns backups?"), "other cards stay: {again}");
        let core = handle.0.lock().await;
        assert!(!core.note_markdown.contains("Answered questions"), "{}", core.note_markdown);
        assert_eq!(core.qa_log.lock().unwrap().len(), 1);
    }
    // ---- delete and save races ----

    #[tokio::test]
    async fn a_card_deleted_while_drafting_leaves_no_trace_when_it_settles() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls.clone())).await;
        let pool = test_pool().await;
        let (qa, chats, history, emit) = {
            let mut core = handle.0.lock().await;
            let (qa, chats) = (core.qa_log.clone(), core.chat_log.clone());
            core.history.lock().unwrap().reset(0);
            let emit = make_card_emit(handle.clone(), qa.clone(), chats.clone(), core.history.clone(), Arc::new(|_| {}));
            core.card_emit = emit.clone();
            (qa, chats, core.history.clone(), emit)
        };
        let mut drafting = settled_card("c1", "Where is the data center?");
        drafting.phase = "drafting".to_string();
        emit(drafting);
        assert_eq!(delete_items(&handle, &pool, vec!["c1".to_string()]).await, Ok(1));
        let mut checked = settled_card("c1", "Where is the data center?");
        checked.kind = CardKind::Ask;
        emit(checked);
        emit(settled_card("c2", "Who owns backups?"));
        settle().await;

        assert_eq!(qa.lock().unwrap().len(), 1, "only c2 is in the Q&A log");
        assert!(chats.lock().unwrap().is_empty(), "the deleted Ask card is not chat either");
        assert!(history.lock().unwrap().finalize(0).iter().all(|i| i.lead == "Building two." && i.question != "Where is the data center?"));
        close_session(&handle).await;
        assert!(draft_note(&handle).await.is_ok());
        let prompt = last_note_prompt(&calls);
        assert!(!prompt.contains("Where is the data center?"), "{prompt}");
        assert!(prompt.contains("Who owns backups?"), "{prompt}");
    }

    async fn rename_items_table(pool: &SqlitePool, from: &str, to: &str) {
        sqlx::query(&format!("ALTER TABLE {from} RENAME TO {to}")).execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn a_failed_database_delete_changes_nothing_and_a_retry_works() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", None).await;
        let handle = AssistantHandle::new();
        let history = collect_asks(&handle).await;
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls)).await;
        refresh_asks(&handle).await;
        settle().await;
        let id = handle.0.lock().await.ask_lane.snapshot().asks[0].id.clone();
        handle.0.lock().await.qa_log.lock().unwrap().push((id.clone(), "Q: x\nA: y".to_string()));
        super::super::history::save_for_meeting(&pool, &history, "m1").await.unwrap();
        assert_eq!(saved_questions(&pool, "m1").await.len(), 1);

        rename_items_table(&pool, "meeting_assistant_items", "parked_items").await;
        assert!(delete_items(&handle, &pool, vec![id.clone()]).await.is_err());
        {
            let core = handle.0.lock().await;
            assert_eq!(core.history.lock().unwrap().finalize(0).len(), 1, "the entry is still in the collector");
            assert!(!core.history.lock().unwrap().is_deleted(&id));
            assert_eq!(core.qa_log.lock().unwrap().len(), 1, "the Q&A log keeps it");
            assert_eq!(core.ask_lane.snapshot().asks.len(), 1, "the lane keeps it");
        }

        rename_items_table(&pool, "parked_items", "meeting_assistant_items").await;
        assert_eq!(delete_items(&handle, &pool, vec![id.clone()]).await, Ok(1));
        assert!(saved_questions(&pool, "m1").await.is_empty(), "the retry deleted the row");
        let core = handle.0.lock().await;
        assert!(core.history.lock().unwrap().is_deleted(&id));
        assert!(core.qa_log.lock().unwrap().is_empty());
    }

    /// A collector with an ask ("What are your fees?") and a settled card
    /// ("c2"), a closed session, and meeting `m1` waiting to be saved.
    async fn two_items_ready_to_save(
        pool: &SqlitePool,
        folder: &std::path::Path,
    ) -> (AssistantHandle, super::super::history::HistoryHandle, String) {
        add_meeting(pool, "m1", Some(folder.to_str().unwrap())).await;
        let handle = AssistantHandle::new();
        let history = collect_asks(&handle).await;
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), ask_runner(calls)).await;
        refresh_asks(&handle).await;
        settle().await;
        let id = handle.0.lock().await.ask_lane.snapshot().asks[0].id.clone();
        super::super::history::record_card(&history, &settled_card("c2", "Who owns backups?"));
        close_session(&handle).await;
        (handle, history, id)
    }

    fn json_questions(folder: &std::path::Path) -> Vec<String> {
        let raw = std::fs::read_to_string(folder.join("assistant.json")).unwrap();
        let file: serde_json::Value = serde_json::from_str(&raw).unwrap();
        file["items"].as_array().unwrap().iter().map(|i| i["question"].as_str().unwrap().to_string()).collect()
    }

    #[tokio::test]
    async fn a_delete_waits_for_the_stop_save_then_reaches_its_rows_and_files() {
        let pool = test_pool().await;
        let folder = tempfile::tempdir().unwrap();
        let (handle, history, id) = two_items_ready_to_save(&pool, folder.path()).await;

        // The Stop save is in flight: it holds the items gate.
        let gate = super::super::history::lock_items_gate(&history).await;
        let delete = {
            let (handle, pool) = (handle.clone(), pool.clone());
            tokio::spawn(async move { delete_items(&handle, &pool, vec![id]).await })
        };
        settle().await;
        assert!(!delete.is_finished(), "a delete never runs between the save's steps");
        let snapshot = super::super::history::snapshot_for_save(&history);
        let written = super::super::history::write_rows(&pool, "m1", &snapshot).await.unwrap();
        assert_eq!(super::super::history::finish_save(&pool, &history, "m1", &snapshot, written).await, Ok(2));
        drop(gate);

        assert_eq!(delete.await.unwrap(), Ok(1));
        assert_eq!(saved_questions(&pool, "m1").await, vec!["Who owns backups?"]);
        assert_eq!(json_questions(folder.path()), vec!["Who owns backups?"]);
        assert_eq!(history.lock().unwrap().finalize(0).len(), 1);
    }

    /// The window the first round left open: a delete has decided the meeting
    /// is not saved yet and is about to take the item out of the collector,
    /// and the Stop save starts right then. The save must wait for the delete.
    #[tokio::test]
    async fn a_save_that_starts_while_a_delete_is_between_its_steps_waits_for_it() {
        let pool = test_pool().await;
        let folder = tempfile::tempdir().unwrap();
        let (handle, history, id) = two_items_ready_to_save(&pool, folder.path()).await;
        let core_lock = handle.0.lock().await;
        let delete = {
            let (handle, pool) = (handle.clone(), pool.clone());
            tokio::spawn(async move { delete_items(&handle, &pool, vec![id]).await })
        };
        tokio::task::yield_now().await;
        drop(core_lock);
        // Queued behind the delete's first look at the core, so it holds the
        // lock the delete needs for its last step.
        let core_lock = handle.0.lock().await;
        let save = {
            let (pool, history) = (pool.clone(), history.clone());
            tokio::spawn(async move { super::super::history::save_for_meeting(&pool, &history, "m1").await })
        };
        settle().await;
        drop(core_lock);

        assert_eq!(delete.await.unwrap(), Ok(1));
        save.await.unwrap().unwrap();
        assert_eq!(saved_questions(&pool, "m1").await, vec!["Who owns backups?"]);
        assert_eq!(json_questions(folder.path()), vec!["Who owns backups?"]);
        assert_eq!(history.lock().unwrap().finalize(0).len(), 1);
    }

    /// Saves and deletes started together on real threads, with the delete
    /// starting a little later each round: whichever order they take, the
    /// deleted item is in no row, no `assistant.json` and no collector.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn saves_and_deletes_started_together_never_leave_the_deleted_item_behind() {
        for round in 0..40 {
            let pool = test_pool().await;
            let folder = tempfile::tempdir().unwrap();
            let (handle, history, id) = two_items_ready_to_save(&pool, folder.path()).await;
            let save = {
                let (pool, history) = (pool.clone(), history.clone());
                tokio::spawn(async move { super::super::history::save_for_meeting(&pool, &history, "m1").await })
            };
            let delete = {
                let (handle, pool) = (handle.clone(), pool.clone());
                tokio::spawn(async move {
                    for _ in 0..round % 10 {
                        tokio::task::yield_now().await;
                    }
                    delete_items(&handle, &pool, vec![id]).await
                })
            };
            save.await.unwrap().unwrap();
            delete.await.unwrap().unwrap();
            assert_eq!(saved_questions(&pool, "m1").await, vec!["Who owns backups?"], "round {round}");
            assert_eq!(json_questions(folder.path()), vec!["Who owns backups?"], "round {round}");
            assert_eq!(history.lock().unwrap().finalize(0).len(), 1, "round {round}");
        }
    }

    #[tokio::test]
    async fn a_retry_after_the_files_failed_to_rebuild_brings_them_in_line() {
        let pool = test_pool().await;
        let folder = tempfile::tempdir().unwrap();
        let (handle, history, id) = two_items_ready_to_save(&pool, folder.path()).await;
        handle.0.lock().await.qa_log.lock().unwrap().push((id.clone(), "Q: fees\nA: ten".to_string()));
        super::super::history::save_for_meeting(&pool, &history, "m1").await.unwrap();
        assert_eq!(json_questions(folder.path()).len(), 2);

        // The row delete works, then reading the Brief to rebuild the file fails.
        rename_items_table(&pool, "meeting_assistant_meta", "parked_meta").await;
        assert!(delete_items(&handle, &pool, vec![id.clone()]).await.is_err());
        assert_eq!(saved_questions(&pool, "m1").await, vec!["Who owns backups?"], "the row is gone");
        {
            let core = handle.0.lock().await;
            assert!(core.history.lock().unwrap().is_deleted(&id), "the collector let go of it anyway");
            assert!(core.qa_log.lock().unwrap().is_empty(), "so did the Q&A log");
        }
        assert_eq!(json_questions(folder.path()).len(), 2, "the file is stale until the retry");

        rename_items_table(&pool, "parked_meta", "meeting_assistant_meta").await;
        assert_eq!(delete_items(&handle, &pool, vec![id]).await, Ok(0), "nothing left to remove");
        assert_eq!(json_questions(folder.path()), vec!["Who owns backups?"]);
    }

    #[tokio::test]
    async fn a_delete_made_while_the_note_drafts_is_drafted_again_before_the_note_is_ready() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let (history, qa) = {
            let core = handle.0.lock().await;
            (core.history_handle_for_test(), core.qa_log.clone())
        };
        let first_note = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let inner = note_runner(calls.clone());
        // Joseph deletes c1 while the first note draft is with the model.
        let runner: Runner = Arc::new(move |turn, on_delta, register| {
            if turn.prompt.starts_with("TRANSCRIPT:") && first_note.swap(false, std::sync::atomic::Ordering::SeqCst) {
                history.lock().unwrap().delete(&["c1".to_string()]);
                qa.lock().unwrap().retain(|(id, _)| id != "c1");
            }
            inner(turn, on_delta, register)
        });
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), runner).await;
        {
            let core = handle.0.lock().await;
            let (qa, chats) = (core.qa_log.clone(), core.chat_log.clone());
            record_card(&settled_card("c1", "Where is the data center?"), &qa, &chats);
            record_card(&settled_card("c2", "Who owns backups?"), &qa, &chats);
        }
        close_session(&handle).await;

        assert!(draft_note(&handle).await.is_ok());
        let prompts: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.prompt.starts_with("TRANSCRIPT:"))
            .map(|t| t.prompt.clone())
            .collect();
        assert_eq!(prompts.len(), 2, "drafted again once the first draft landed");
        assert!(prompts[0].contains("Where is the data center?"));
        assert!(!prompts[1].contains("Where is the data center?") && prompts[1].contains("Who owns backups?"), "{}", prompts[1]);
        assert_eq!(handle.0.lock().await.note_state, NoteState::Ready);
    }

    #[tokio::test]
    async fn the_x_on_an_ask_item_also_takes_it_out_of_the_q_and_a_log_and_the_draft() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls.clone())).await;
        refresh_asks(&handle).await;
        settle().await;
        let id = handle.0.lock().await.ask_lane.snapshot().asks[0].id.clone();
        {
            let core = handle.0.lock().await;
            let mut qa = core.qa_log.lock().unwrap();
            qa.push((id.clone(), "Q: Where is the data center?\nA: Building two.".to_string()));
            qa.push(("c2".to_string(), "Q: Who owns backups?\nA: Building two.".to_string()));
        }

        dismiss_ask(&handle, id).await;
        settle().await;
        assert_eq!(handle.0.lock().await.qa_log.lock().unwrap().len(), 1);
        close_session(&handle).await;
        assert!(draft_note(&handle).await.is_ok());
        let prompt = last_note_prompt(&calls);
        assert!(!prompt.contains("Where is the data center?"), "{prompt}");
        assert!(prompt.contains("Who owns backups?"), "{prompt}");
    }

    #[tokio::test]
    async fn opening_a_session_after_a_saved_note_tells_the_panel_the_note_is_idle() {
        let handle = AssistantHandle::new();
        let sent = Arc::new(StdMutex::new(Vec::<(&'static str, String)>::new()));
        {
            let mut core = handle.0.lock().await;
            let sink = sent.clone();
            core.note_emit = Arc::new(move |out: NoteOut| sink.lock().unwrap().push((out.state, out.markdown)));
            core.note_state = NoteState::Saved;
            core.note_markdown = "## Summary".to_string();
        }
        let root = std::env::temp_dir().join(format!("meetily-note-idle-{}", uuid::Uuid::new_v4()));
        open_session_in(&handle, Some(false), &root).await;

        assert_eq!(sent.lock().unwrap().last(), Some(&("idle", String::new())));
        assert_eq!(handle.0.lock().await.note_state, NoteState::Idle);
    }

    #[tokio::test]
    async fn deleting_on_the_past_meetings_tab_updates_the_live_session() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", None).await;
        add_meeting(&pool, "m2", None).await;
        let handle = AssistantHandle::new();
        let history = collect_asks(&handle).await;
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls.clone())).await;
        refresh_asks(&handle).await;
        settle().await;
        refresh_asks(&handle).await;
        settle().await;
        {
            let core = handle.0.lock().await;
            let (qa, chats) = (core.qa_log.clone(), core.chat_log.clone());
            record_card(&settled_card("c1", "Where is the data center?"), &qa, &chats);
            record_card(&settled_card("c2", "Who owns backups?"), &qa, &chats);
        }
        super::super::history::record_card(&history, &settled_card("c1", "Where is the data center?"));
        super::super::history::record_card(&history, &settled_card("c2", "Who owns backups?"));
        let answered_id = handle.0.lock().await.ask_lane.snapshot().answered[0].id.clone();
        close_session(&handle).await;
        super::super::history::save_for_meeting(&pool, &history, "m1").await.unwrap();
        assert!(draft_note(&handle).await.is_ok());
        let rows = AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap();
        let row_of = |q: &str| rows.iter().find(|r| r.question == q).unwrap().id.clone();
        let (c1_row, answered_row) = (row_of("Where is the data center?"), row_of("What are your fees?"));

        // Another meeting's tab: nothing live changes, nothing is announced.
        let announced = Arc::new(StdMutex::new(Vec::<Vec<String>>::new()));
        let sink = announced.clone();
        assert_eq!(
            delete_past_items(&handle, &pool, "m2", vec![c1_row.clone()], move |ids| sink.lock().unwrap().push(ids)).await,
            Ok(0)
        );
        assert!(announced.lock().unwrap().is_empty());
        assert_eq!(handle.0.lock().await.qa_log.lock().unwrap().len(), 2);

        // The saved meeting's tab: the live session follows.
        let sink = announced.clone();
        assert_eq!(
            delete_past_items(&handle, &pool, "m1", vec![c1_row, answered_row], move |ids| sink.lock().unwrap().push(ids)).await,
            Ok(2)
        );
        let announced = announced.lock().unwrap().clone();
        assert_eq!(announced.len(), 1);
        assert!(announced[0].contains(&"c1".to_string()), "{:?}", announced);
        assert!(announced[0].contains(&answered_id), "{:?}", announced);
        {
            let core = handle.0.lock().await;
            assert_eq!(core.qa_log.lock().unwrap().iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), vec!["c2"]);
            assert!(core.history.lock().unwrap().is_deleted("c1"));
        }
        settle().await;
        settle().await;
        let again = last_note_prompt(&calls);
        assert!(!again.contains("Where is the data center?") && !again.contains("What are your fees?"), "{again}");
        assert!(again.contains("Who owns backups?"), "{again}");
    }

    #[tokio::test]
    async fn a_redraft_never_flips_a_saved_or_drafting_note() {
        let handle = AssistantHandle::new();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        open_lanes_with(&handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls.clone())).await;
        close_session(&handle).await;
        assert!(draft_note(&handle).await.is_ok());
        assert_eq!(handle.0.lock().await.note_state, NoteState::Ready);
        let turns = calls.lock().unwrap().len();

        // A delete saw Ready and queued a redraft; Save ran before it started.
        handle.0.lock().await.note_state = NoteState::Saved;
        assert!(draft_note_if_ready(&handle).await.is_ok());
        assert_eq!(handle.0.lock().await.note_state, NoteState::Saved);
        handle.0.lock().await.note_state = NoteState::Drafting;
        assert!(draft_note_if_ready(&handle).await.is_ok());
        assert_eq!(handle.0.lock().await.note_state, NoteState::Drafting);
        assert_eq!(calls.lock().unwrap().len(), turns, "no draft turn ran");

        handle.0.lock().await.note_state = NoteState::Ready;
        assert!(draft_note_if_ready(&handle).await.is_ok());
        assert_eq!(handle.0.lock().await.note_state, NoteState::Ready);
        assert!(calls.lock().unwrap().len() > turns, "a Ready note is drafted again");
    }

    #[tokio::test]
    async fn the_note_uses_the_saved_meetings_brief_not_the_next_calls() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", None).await;
        let h = super::super::history::new_handle();
        h.lock().unwrap().reset(0);
        h.lock().unwrap().set_brief("Plan the move");
        super::super::history::save_for_meeting(&pool, &h, "m1").await.unwrap();
        // After Stop the Brief box edits the next call's brief.
        h.lock().unwrap().set_brief("Next call: pricing");
        assert_eq!(meeting_facts(&pool, &h).await.unwrap().brief, "Plan the move");

        // A meeting that was saved with no brief has none, whatever is typed after.
        add_meeting(&pool, "m2", None).await;
        let h2 = super::super::history::new_handle();
        h2.lock().unwrap().reset(0);
        super::super::history::save_for_meeting(&pool, &h2, "m2").await.unwrap();
        h2.lock().unwrap().set_brief("Typed after Stop");
        assert_eq!(meeting_facts(&pool, &h2).await.unwrap().brief, "");

        // Not saved yet: the collector's brief is the only one there is.
        let h3 = super::super::history::new_handle();
        h3.lock().unwrap().reset(0);
        h3.lock().unwrap().set_brief("Live brief");
        assert_eq!(meeting_facts(&pool, &h3).await.unwrap().brief, "Live brief");
    }
}
