// assistant/lanes.rs
//
// Two `claude -p` lineages forked from one seeded trunk, fired together on
// every accepted trigger. Fast puts words on the rail in seconds; deep
// re-derives the same answer with a stronger model and settles the card in
// place.
//
// Nothing is ever queued. A trigger landing while a lane is already in
// flight forks that lane's lineage for itself rather than waiting, so two
// concurrent sessions never contend over one transcript file. Only the
// primary (non-forked) call advances that lane's transcript cursor.
//
// Port of v1's AnswerLanes.swift. v1 renders straight into a MeetingStore
// this crate has no equivalent of; here every card, drafting or settled, is
// emitted through `EmitFn` and the frontend owns dedupe-by-id.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::card::{self, ParsedCard};
use super::claude_cli::{ClaudeOutcome, ClaudeTurn, TurnHandle, NO_TOOLS, READ_ONLY_TOOLS, WRITE_TOOLS};
use super::transcript::{Speaker, TranscriptLog};
use super::vision::{encode, CallApp, FrameStore, ImageBlock, KeptFrame};

const FAST_PROMPT: &str = include_str!("prompts/fast.md");
const DEEP_PROMPT: &str = include_str!("prompts/deep.md");

/// v1's EngineConfig.explainWindowSeconds / catchupWindowSeconds. Not user
/// settings, so plain constants rather than AssistantSettings fields.
const EXPLAIN_WINDOW_SECS: f64 = 15.0;
const CATCHUP_WINDOW_SECS: f64 = 300.0;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// One `claude -p` turn, abstracted so tests can inject a fake that never
/// spawns a process. Production wiring is `claude_runner` below.
pub type Runner = Arc<
    dyn Fn(
            ClaudeTurn,
            Box<dyn FnMut(String) + Send>,
            Box<dyn FnOnce(TurnHandle) + Send>,
        ) -> BoxFuture<ClaudeOutcome>
        + Send
        + Sync,
>;

/// Wires `Runner` to a real `claude` binary via Task 3's `run_turn`. Tests
/// use a fake `Runner` instead; this is what Task 10 hands to production.
pub fn claude_runner(binary: PathBuf) -> Runner {
    Arc::new(move |turn, mut on_delta, register| {
        let binary = binary.clone();
        Box::pin(async move {
            super::claude_cli::run_turn(
                &binary,
                turn,
                move |chunk: &str| on_delta(chunk.to_string()),
                register,
            )
            .await
        })
    })
}

/// Turns a kept frame on disk into an image block. Production is
/// `encode::for_send`; tests inject a fake so they never touch disk.
pub type Encoder = Arc<dyn Fn(&Path) -> Result<ImageBlock, String> + Send + Sync>;

/// Which kept frames a turn wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameMode {
    /// Only the newest frame since the lane's cursor (zero or one).
    Newest,
    /// Every frame since the lane's cursor.
    All,
}

/// The frame store plus the encoder the lanes use for it.
#[derive(Clone)]
pub struct FrameFeed {
    store: FrameStore,
    encode: Encoder,
}

impl FrameFeed {
    pub fn new(store: FrameStore) -> Self {
        Self {
            store,
            encode: Arc::new(|p: &Path| encode::for_send(p)),
        }
    }

    pub fn with_encoder(store: FrameStore, encode: Encoder) -> Self {
        Self { store, encode }
    }

    /// The frames a lane at `cursor` should send, and the new cursor value
    /// (the newest seq at this moment). The caller moves the cursor only
    /// for a primary turn, like the transcript cursor.
    pub(super) fn select(&self, cursor: u64, mode: FrameMode) -> (FrameBatch, u64) {
        let latest = self.store.latest_seq();
        let mut frames: Vec<KeptFrame> = self
            .store
            .since(cursor)
            .into_iter()
            .filter(|f| f.seq <= latest)
            .collect();
        if mode == FrameMode::Newest && frames.len() > 1 {
            frames.drain(..frames.len() - 1);
        }
        (self.batch(frames), latest)
    }

    fn batch(&self, frames: Vec<KeptFrame>) -> FrameBatch {
        FrameBatch {
            frames,
            encode: Some(self.encode.clone()),
        }
    }
}

/// Frames chosen for one turn, not yet encoded. Encoding happens on a
/// blocking thread when the turn runs, so building a turn stays instant.
#[derive(Clone, Default)]
pub(super) struct FrameBatch {
    frames: Vec<KeptFrame>,
    encode: Option<Encoder>,
}

impl FrameBatch {
    fn none() -> Self {
        Self::default()
    }

    /// Encodes the frames into `turn.images`. A frame that fails to encode
    /// is skipped.
    async fn attach(self, turn: &mut ClaudeTurn) {
        let Some(encode) = self.encode else { return };
        if self.frames.is_empty() {
            return;
        }
        let frames = self.frames;
        let done = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            for f in frames {
                match encode(&f.path) {
                    Ok(img) => out.push((frame_label(f.app, f.captured_at_ms), img)),
                    Err(e) => log::debug!("{} frame {} skipped: {}", f.app.name(), f.seq, e),
                }
            }
            out
        })
        .await
        .unwrap_or_default();
        for (label, img) in done {
            turn.image_labels.push(label);
            turn.images.push(img);
        }
    }
}

fn frame_label(app: CallApp, captured_at_ms: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(captured_at_ms).single() {
        Some(t) => format!("{} window at {}:", app.name(), t.format("%H:%M:%S")),
        None => format!("{} window:", app.name()),
    }
}

pub(super) type DeltaCb = Box<dyn FnMut(String) + Send>;
pub(super) type RegisterCb = Box<dyn FnOnce(TurnHandle) + Send>;

/// Runs a turn with its frames. When it fails and carried images, runs once
/// more with the same prompt and no images; `on_retry` lets the caller drop
/// whatever the failed attempt streamed. `make_cbs` supplies fresh callbacks
/// for each attempt.
pub(super) async fn run_with_retry(
    runner: &Runner,
    mut turn: ClaudeTurn,
    batch: FrameBatch,
    mut make_cbs: impl FnMut() -> (DeltaCb, RegisterCb),
    mut on_retry: impl FnMut(),
    should_retry: impl Fn() -> bool,
) -> ClaudeOutcome {
    batch.attach(&mut turn).await;
    let had_images = !turn.images.is_empty();
    let (delta, register) = make_cbs();
    let outcome = (runner)(turn.clone(), delta, register).await;
    if had_images {
        if let Some(err) = &outcome.error {
            // A turn killed by close() reports an error too; nothing may
            // start once the session is closed.
            if !should_retry() {
                return outcome;
            }
            log::warn!("assistant turn with images failed ({}), retrying without them", err);
            on_retry();
            turn.images.clear();
            turn.image_labels.clear();
            let (delta, register) = make_cbs();
            return (runner)(turn, delta, register).await;
        }
    }
    outcome
}

#[derive(Debug, Clone)]
pub struct LaneConfig {
    /// Working directory for every lane turn (v1's laneSessionDir).
    pub cwd: PathBuf,
    pub fast_model: String,
    pub fast_effort: String,
    pub deep_model: String,
    pub deep_effort: String,
    /// Extra directories the deep lane may read.
    pub deep_read_dirs: Vec<PathBuf>,
    /// System prompt for `draft_note`'s turn. `note.md` is Task 9's file;
    /// the caller supplies its contents here so this module has no forward
    /// dependency on it.
    pub note_prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CardKind {
    Answer,
    Ask,
    Explain,
    Catchup,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardOut {
    pub id: String,
    pub kind: CardKind,
    pub question: String,
    pub lead: String,
    pub bullets: Vec<String>,
    pub source: String,
    /// "drafting" | "checked" | "corrected"
    pub phase: String,
    pub changed_lines: Vec<String>,
    pub ts: i64,
}

pub type EmitFn = Arc<dyn Fn(CardOut) + Send + Sync>;

#[derive(Default)]
struct LaneState {
    trunk_id: Option<String>,
    fast_id: Option<String>,
    deep_id: Option<String>,
    fast_cursor: usize,
    deep_cursor: usize,
    fast_in_flight: bool,
    deep_in_flight: bool,
    /// What the fast pass said, per card, so the deep pass can tell a
    /// rewording from a contradiction.
    fast_drafts: HashMap<String, ParsedCard>,
    catchup_used: bool,
    /// Mid-call brief edits. Each lane's next turn carries the new text;
    /// only a primary turn (not a fork) marks it seen, because only a
    /// primary turn continues the lane's own session.
    brief: String,
    brief_version: u64,
    fast_brief_seen: u64,
    deep_brief_seen: u64,
    live_runs: Vec<TurnHandle>,
    cfg: Option<LaneConfig>,
    /// Zoom window frames, when the assistant can see the window.
    frames: Option<FrameFeed>,
    /// Last frame seq each lane sent (0 = none yet).
    fast_frame_cursor: u64,
    deep_frame_cursor: u64,
    /// Set by `close`: no retry may start after it.
    closed: bool,
}

pub struct AnswerLanes {
    runner: Runner,
    state: Arc<Mutex<LaneState>>,
}

enum LaneTools {
    None,
    ReadOnly,
}

struct LaneCall {
    lane_id: String,
    model: String,
    effort: String,
    system_prompt: String,
    tools: LaneTools,
    fork: bool,
    cwd: PathBuf,
    add_dirs: Vec<PathBuf>,
    prompt: String,
    partial: bool,
}

fn build_turn(call: LaneCall) -> ClaudeTurn {
    let mut turn = ClaudeTurn {
        prompt: call.prompt,
        cwd: call.cwd,
        model: Some(call.model),
        effort: Some(call.effort),
        resume: Some(call.lane_id),
        fork: call.fork,
        append_system_prompt: Some(call.system_prompt),
        partial: call.partial,
        safe_mode: true,
        add_dirs: call.add_dirs,
        ..Default::default()
    };
    match call.tools {
        LaneTools::None => {
            turn.disallowed_tools = NO_TOOLS.iter().map(|s| s.to_string()).collect();
        }
        LaneTools::ReadOnly => {
            turn.allowed_tools = READ_ONLY_TOOLS.iter().map(|s| s.to_string()).collect();
            turn.disallowed_tools = WRITE_TOOLS.iter().map(|s| s.to_string()).collect();
        }
    }
    turn
}

fn turn_prompt(delta: &str, trigger: &str, extra: Option<&str>) -> String {
    let mut out = String::new();
    if !delta.is_empty() {
        out.push_str("NEW TRANSCRIPT since your last turn:\n");
        out.push_str(delta);
        out.push_str("\n\n");
    }
    out.push_str("TRIGGER: ");
    out.push_str(trigger);
    out.push('\n');
    if let Some(extra) = extra {
        out.push('\n');
        out.push_str(extra);
        out.push('\n');
    }
    out.push_str("\nAnswer in the card format, or reply SKIP.");
    out
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Clone, Copy)]
enum Lane {
    Fast,
    Deep,
}

/// The brief update this lane has not seen yet, if any.
fn take_brief_update(s: &mut LaneState, lane: Lane, fork: bool) -> Option<String> {
    let version = s.brief_version;
    let seen = match lane {
        Lane::Fast => &mut s.fast_brief_seen,
        Lane::Deep => &mut s.deep_brief_seen,
    };
    if *seen >= version {
        return None;
    }
    if !fork {
        *seen = version;
    }
    Some(s.brief.clone())
}

/// The frames this lane's next turn carries. Only a primary turn (not a
/// fork) moves the lane's frame cursor.
fn take_frames(s: &mut LaneState, lane: Lane, mode: FrameMode, fork: bool) -> FrameBatch {
    let Some(feed) = s.frames.clone() else {
        return FrameBatch::none();
    };
    let cursor = match lane {
        Lane::Fast => &mut s.fast_frame_cursor,
        Lane::Deep => &mut s.deep_frame_cursor,
    };
    let (batch, latest) = feed.select(*cursor, mode);
    if !fork {
        *cursor = latest;
    }
    batch
}

fn fast_frame_mode(kind: CardKind) -> FrameMode {
    match kind {
        CardKind::Answer | CardKind::Explain => FrameMode::Newest,
        CardKind::Ask | CardKind::Catchup => FrameMode::All,
    }
}

fn with_brief_update(update: Option<String>, prompt: String) -> String {
    match update {
        None => prompt,
        Some(brief) if brief.trim().is_empty() => format!(
            "The meeting brief was cleared. Work from the transcript alone.\n\n{}",
            prompt
        ),
        Some(brief) => format!(
            "The meeting brief was updated. This replaces the earlier one:\n{}\n\n{}",
            brief.trim(),
            prompt
        ),
    }
}


/// Spawns one lane turn. Deltas accumulate and, once SKIP can be ruled out,
/// parse into a card and reach `stream_cb`. The finished turn's text (or the
/// accumulated stream if the runner returned no final text) parses once more
/// and reaches `done_cb`.
fn spawn_turn(
    runner: Runner,
    state: Arc<Mutex<LaneState>>,
    turn: ClaudeTurn,
    batch: FrameBatch,
    stream_cb: impl Fn(ParsedCard) + Send + Sync + 'static,
    done_cb: impl FnOnce(ParsedCard) + Send + 'static,
) {
    tokio::spawn(async move {
        let accumulated = Arc::new(Mutex::new(String::new()));
        let stream_cb = Arc::new(stream_cb);
        let make_cbs = {
            let accumulated = accumulated.clone();
            let state = state.clone();
            move || -> (DeltaCb, RegisterCb) {
                let accumulated = accumulated.clone();
                let stream_cb = stream_cb.clone();
                let on_delta: DeltaCb = Box::new(move |chunk: String| {
                    let whole = {
                        let mut acc = accumulated.lock().unwrap();
                        acc.push_str(&chunk);
                        acc.clone()
                    };
                    if card::can_decide(&whole) {
                        stream_cb(card::parse(&whole));
                    }
                });
                let state = state.clone();
                let register: RegisterCb = Box::new(move |handle: TurnHandle| {
                    state.lock().unwrap().live_runs.push(handle);
                });
                (on_delta, register)
            }
        };
        let drop_streamed = {
            let accumulated = accumulated.clone();
            move || accumulated.lock().unwrap().clear()
        };

        let open_state = state.clone();
        let outcome = run_with_retry(&runner, turn, batch, make_cbs, drop_streamed, move || {
            !open_state.lock().unwrap().closed
        })
        .await;
        if let Some(err) = &outcome.error {
            log::warn!("assistant lane turn failed: {}", err);
        }
        let text = if outcome.text.is_empty() {
            accumulated.lock().unwrap().clone()
        } else {
            outcome.text
        };
        done_cb(card::parse(&text));
    });
}

fn present_or_update_draft(
    emit: &EmitFn,
    card_id: &str,
    kind: CardKind,
    question: &str,
    parsed: ParsedCard,
) {
    if parsed.is_skip || parsed.is_empty {
        return;
    }
    emit(CardOut {
        id: card_id.to_string(),
        kind,
        question: question.to_string(),
        lead: parsed.lead,
        bullets: parsed.bullets,
        source: parsed.source,
        phase: "drafting".to_string(),
        changed_lines: Vec::new(),
        ts: now_ms(),
    });
}

fn settle(
    state: &Arc<Mutex<LaneState>>,
    emit: &EmitFn,
    card_id: &str,
    kind: CardKind,
    question: &str,
    deep: ParsedCard,
) {
    if deep.is_skip || deep.is_empty {
        // The fast draft stays on the rail as-is. Log it, or a deep pass
        // that declined looks exactly like one that never came back.
        log::debug!(
            "deep pass declined to settle ({}), the draft stands",
            if deep.is_skip { "SKIP" } else { "empty reply" }
        );
        state.lock().unwrap().fast_drafts.remove(card_id);
        return;
    }
    let fast = state
        .lock()
        .unwrap()
        .fast_drafts
        .remove(card_id)
        .unwrap_or_default();
    let contradicted = card::contradicts(&fast, &deep);
    let changed = if contradicted {
        card::changed_bullets(&fast, &deep)
    } else {
        Vec::new()
    };
    emit(CardOut {
        id: card_id.to_string(),
        kind,
        question: question.to_string(),
        lead: deep.lead,
        bullets: deep.bullets,
        source: deep.source,
        phase: if contradicted { "corrected" } else { "checked" }.to_string(),
        changed_lines: changed,
        ts: now_ms(),
    });
}

impl AnswerLanes {
    pub fn new(runner: Runner) -> Self {
        Self {
            runner,
            state: Arc::new(Mutex::new(LaneState::default())),
        }
    }

    /// Seed one trunk with the meeting brief, then fork it twice. Runs at
    /// meeting start where there is no latency pressure.
    pub async fn open(&mut self, seed: String, cfg: &LaneConfig, _emit: EmitFn) {
        // v1's `open` only logs on failure; nothing to present, so `emit`
        // goes unused here but stays in the signature for interface parity.
        {
            self.state.lock().unwrap().cfg = Some(cfg.clone());
        }

        let trunk_id = uuid::Uuid::new_v4().to_string();
        let trunk_prompt = format!(
            "This is the brief for the meeting that is about to start. Read it, hold it, and answer nothing yet. Reply with exactly the word READY.\n\n{}",
            seed
        );
        let trunk_turn = ClaudeTurn {
            prompt: trunk_prompt,
            cwd: cfg.cwd.clone(),
            model: Some(cfg.fast_model.clone()),
            effort: Some(cfg.fast_effort.clone()),
            session_id: Some(trunk_id.clone()),
            append_system_prompt: Some(FAST_PROMPT.to_string()),
            disallowed_tools: NO_TOOLS.iter().map(|s| s.to_string()).collect(),
            partial: false,
            safe_mode: true,
            ..Default::default()
        };
        let outcome = self.run_bare_turn(trunk_turn).await;
        if let Some(err) = &outcome.error {
            log::warn!("assistant trunk seed failed: {}", err);
            return;
        }
        let trunk_id = outcome.session_id.unwrap_or(trunk_id);
        self.state.lock().unwrap().trunk_id = Some(trunk_id.clone());

        let (fast_id, deep_id) = tokio::join!(
            self.fork_lane(
                &trunk_id,
                &cfg.fast_model,
                &cfg.fast_effort,
                FAST_PROMPT,
                cfg.cwd.clone()
            ),
            self.fork_lane(
                &trunk_id,
                &cfg.deep_model,
                &cfg.deep_effort,
                DEEP_PROMPT,
                cfg.cwd.clone()
            )
        );

        let mut s = self.state.lock().unwrap();
        s.fast_id = fast_id;
        s.deep_id = deep_id;
    }

    async fn fork_lane(
        &self,
        trunk_id: &str,
        model: &str,
        effort: &str,
        system: &'static str,
        cwd: PathBuf,
    ) -> Option<String> {
        let turn = ClaudeTurn {
            prompt: "Standing by. Reply with exactly the word READY.".to_string(),
            cwd,
            model: Some(model.to_string()),
            effort: Some(effort.to_string()),
            resume: Some(trunk_id.to_string()),
            fork: true,
            append_system_prompt: Some(system.to_string()),
            disallowed_tools: NO_TOOLS.iter().map(|s| s.to_string()).collect(),
            partial: false,
            safe_mode: true,
            ..Default::default()
        };
        let outcome = self.run_bare_turn(turn).await;
        if let Some(err) = &outcome.error {
            log::warn!("assistant fork failed: {}", err);
        }
        outcome.session_id
    }

    async fn run_bare_turn(&self, turn: ClaudeTurn) -> ClaudeOutcome {
        let state = self.state.clone();
        let on_delta: Box<dyn FnMut(String) + Send> = Box::new(|_| {});
        let register: Box<dyn FnOnce(TurnHandle) + Send> = Box::new(move |handle| {
            state.lock().unwrap().live_runs.push(handle);
        });
        (self.runner)(turn, on_delta, register).await
    }

    /// Whether both lanes forked successfully. False while `open` is still
    /// running, or after it failed (a missing/unauthenticated claude binary,
    /// a killed trunk seed, and so on): every trigger and command that needs
    /// a lane checks this first rather than assuming `open` succeeded.
    pub fn is_ready(&self) -> bool {
        let s = self.state.lock().unwrap();
        s.fast_id.is_some() && s.deep_id.is_some()
    }

    pub fn close(&mut self) {
        let handles: Vec<TurnHandle> = {
            let mut s = self.state.lock().unwrap();
            s.closed = true;
            std::mem::take(&mut s.live_runs)
        };
        for handle in handles {
            tokio::spawn(async move {
                handle.kill().await;
            });
        }
    }

    pub fn set_brief(&mut self, text: String) {
        let mut s = self.state.lock().unwrap();
        s.brief = text;
        s.brief_version += 1;
    }

    /// Hands the lanes the frames of the Zoom window, or None when the
    /// assistant cannot see it. Cursors restart at 0.
    pub fn set_frames(&mut self, feed: Option<FrameFeed>) {
        let mut s = self.state.lock().unwrap();
        s.frames = feed;
        s.fast_frame_cursor = 0;
        s.deep_frame_cursor = 0;
    }

    #[cfg(test)]
    pub(super) fn has_frames(&self) -> bool {
        self.state.lock().unwrap().frames.is_some()
    }

    /// The seeded trunk the lanes forked from, for the ask lane to fork too.
    pub fn trunk_id(&self) -> Option<String> {
        self.state.lock().unwrap().trunk_id.clone()
    }

    /// A proactive or gated trigger: both lanes fire, fast streams, deep
    /// settles.
    pub fn answer(&mut self, question: String, kind: CardKind, log: &TranscriptLog, emit: EmitFn) {
        self.answer_with_note(question, None, kind, log, emit);
    }

    /// Same as `answer`, with `extra` appended to the turn prompt only
    /// (never shown as the card's question). Used for a direct ask, which
    /// must never SKIP, without polluting what Joseph sees on the card.
    pub fn answer_with_note(
        &mut self,
        question: String,
        extra: Option<&str>,
        kind: CardKind,
        log: &TranscriptLog,
        emit: EmitFn,
    ) {
        let cfg = { self.state.lock().unwrap().cfg.clone() };
        let Some(cfg) = cfg else {
            log::warn!("assistant trigger dropped: lanes are not configured yet");
            return;
        };
        let (fast_id, deep_id) = {
            let s = self.state.lock().unwrap();
            (s.fast_id.clone(), s.deep_id.clone())
        };
        let (Some(fast_id), Some(deep_id)) = (fast_id, deep_id) else {
            log::warn!("assistant trigger dropped: lanes are not open yet");
            return;
        };

        let card_id = uuid::Uuid::new_v4().to_string();
        let (fast_delta_text, fast_delta_cursor) = {
            let cursor = self.state.lock().unwrap().fast_cursor;
            log.delta_since(cursor)
        };
        let (deep_delta_text, deep_delta_cursor) = {
            let cursor = self.state.lock().unwrap().deep_cursor;
            log.delta_since(cursor)
        };

        // Fast lane. It self-gates: SKIP renders nothing at all.
        let (fast_fork, fast_brief, fast_frames) = {
            let mut s = self.state.lock().unwrap();
            let fork = s.fast_in_flight;
            if !fork {
                s.fast_cursor = fast_delta_cursor;
            }
            s.fast_in_flight = true;
            let frames = take_frames(&mut *s, Lane::Fast, fast_frame_mode(kind), fork);
            (fork, take_brief_update(&mut *s, Lane::Fast, fork), frames)
        };
        let fast_turn = build_turn(LaneCall {
            lane_id: fast_id,
            model: cfg.fast_model.clone(),
            effort: cfg.fast_effort.clone(),
            system_prompt: FAST_PROMPT.to_string(),
            tools: LaneTools::None,
            fork: fast_fork,
            cwd: cfg.cwd.clone(),
            add_dirs: Vec::new(),
            prompt: with_brief_update(fast_brief, turn_prompt(&fast_delta_text, &question, extra)),
            partial: true,
        });
        let stream_emit = emit.clone();
        let stream_card_id = card_id.clone();
        let stream_question = question.clone();
        let done_state = self.state.clone();
        let done_card_id = card_id.clone();
        spawn_turn(
            self.runner.clone(),
            self.state.clone(),
            fast_turn,
            fast_frames,
            move |parsed: ParsedCard| {
                present_or_update_draft(&stream_emit, &stream_card_id, kind, &stream_question, parsed);
            },
            move |parsed: ParsedCard| {
                let mut s = done_state.lock().unwrap();
                s.fast_in_flight = false;
                if parsed.is_skip {
                    log::debug!("fast pass returned SKIP, nothing rendered");
                    return;
                }
                s.fast_drafts.insert(done_card_id, parsed);
            },
        );

        // Deep lane, started at the same instant. It never waits on fast.
        let (deep_fork, deep_brief, deep_frames) = {
            let mut s = self.state.lock().unwrap();
            let fork = s.deep_in_flight;
            if !fork {
                s.deep_cursor = deep_delta_cursor;
            }
            s.deep_in_flight = true;
            let frames = take_frames(&mut *s, Lane::Deep, FrameMode::All, fork);
            (fork, take_brief_update(&mut *s, Lane::Deep, fork), frames)
        };
        let deep_turn = build_turn(LaneCall {
            lane_id: deep_id,
            model: cfg.deep_model.clone(),
            effort: cfg.deep_effort.clone(),
            system_prompt: DEEP_PROMPT.to_string(),
            tools: LaneTools::ReadOnly,
            fork: deep_fork,
            cwd: cfg.cwd.clone(),
            add_dirs: cfg.deep_read_dirs.clone(),
            prompt: with_brief_update(deep_brief, turn_prompt(&deep_delta_text, &question, extra)),
            partial: true,
        });
        let settle_state = self.state.clone();
        let done_state = self.state.clone();
        let settle_emit = emit;
        let settle_card_id = card_id;
        let settle_question = question;
        spawn_turn(
            self.runner.clone(),
            self.state.clone(),
            deep_turn,
            deep_frames,
            |_parsed: ParsedCard| {},
            move |parsed: ParsedCard| {
                done_state.lock().unwrap().deep_in_flight = false;
                settle(
                    &settle_state,
                    &settle_emit,
                    &settle_card_id,
                    kind,
                    &settle_question,
                    parsed,
                );
            },
        );
    }

    /// Explain and catch-up are transcript-local. There is nothing to
    /// verify, so they are fast-lane only and land already checked.
    pub fn explain(&mut self, log: &TranscriptLog, emit: EmitFn) -> Result<(), String> {
        let window = log.window(EXPLAIN_WINDOW_SECS, Some(Speaker::Them));
        if window.is_empty() {
            log::info!("explain: nothing on the Them channel yet, nothing to explain");
            return Err("Nothing from the other side yet to explain.".to_string());
        }
        self.fast_only(
            CardKind::Explain,
            "Explain the last 15 seconds".to_string(),
            format!(
                "EXPLAIN this, in plain language, for someone who just tuned in:\n\n{}",
                window
            ),
            emit,
        );
        Ok(())
    }

    pub fn catchup(&mut self, log: &TranscriptLog, emit: EmitFn) {
        let already_used = { self.state.lock().unwrap().catchup_used };
        let (body, label) = if already_used {
            (
                log.window(CATCHUP_WINDOW_SECS, None),
                "Catch-up, last 5 minutes".to_string(),
            )
        } else {
            (log.all(), "Catch-up, the meeting so far".to_string())
        };
        self.state.lock().unwrap().catchup_used = true;
        if body.is_empty() {
            log::info!("catch-up: no transcript yet");
            return;
        }
        self.fast_only(
            CardKind::Catchup,
            label,
            format!("CATCH ME UP on this stretch of the meeting:\n\n{}", body),
            emit,
        );
    }

    fn fast_only(&mut self, kind: CardKind, question: String, body: String, emit: EmitFn) {
        log::info!("assistant fast-only request entered: kind={:?}", kind);
        let cfg = { self.state.lock().unwrap().cfg.clone() };
        let Some(cfg) = cfg else {
            log::warn!("assistant fast-only request dropped: lanes are not configured yet");
            return;
        };
        let fast_id = { self.state.lock().unwrap().fast_id.clone() };
        let Some(fast_id) = fast_id else {
            log::warn!("assistant fast-only request dropped: lanes are not open yet");
            return;
        };
        log::info!("assistant fast-only request past guards, spawning a turn: kind={:?}", kind);

        let card_id = uuid::Uuid::new_v4().to_string();
        let (fast_fork, fast_brief, fast_frames) = {
            let mut s = self.state.lock().unwrap();
            let fork = s.fast_in_flight;
            s.fast_in_flight = true;
            let frames = take_frames(&mut *s, Lane::Fast, fast_frame_mode(kind), fork);
            (fork, take_brief_update(&mut *s, Lane::Fast, fork), frames)
        };
        let turn = build_turn(LaneCall {
            lane_id: fast_id,
            model: cfg.fast_model.clone(),
            effort: cfg.fast_effort.clone(),
            system_prompt: FAST_PROMPT.to_string(),
            tools: LaneTools::None,
            fork: fast_fork,
            cwd: cfg.cwd.clone(),
            add_dirs: Vec::new(),
            prompt: with_brief_update(fast_brief, body),
            partial: true,
        });

        let stream_emit = emit.clone();
        let stream_card_id = card_id.clone();
        let stream_question = question.clone();
        let done_state = self.state.clone();
        let done_emit = emit;
        let done_card_id = card_id;
        let done_question = question;
        spawn_turn(
            self.runner.clone(),
            self.state.clone(),
            turn,
            fast_frames,
            move |parsed: ParsedCard| {
                present_or_update_draft(&stream_emit, &stream_card_id, kind, &stream_question, parsed);
            },
            move |parsed: ParsedCard| {
                done_state.lock().unwrap().fast_in_flight = false;
                if parsed.is_skip || parsed.is_empty {
                    log::info!(
                        "assistant fast-only turn finished with nothing to show: kind={:?} is_skip={} is_empty={}",
                        kind, parsed.is_skip, parsed.is_empty
                    );
                    return;
                }
                // Nothing to verify, so it is neutral ink the moment it lands.
                done_emit(CardOut {
                    id: done_card_id,
                    kind,
                    question: done_question,
                    lead: parsed.lead,
                    bullets: parsed.bullets,
                    source: parsed.source,
                    phase: "checked".to_string(),
                    changed_lines: Vec::new(),
                    ts: now_ms(),
                });
            },
        );
    }

    /// One turn of the note-drafting work, on the deep lineage, at end of
    /// meeting.
    pub async fn draft_note(&mut self, transcript: String, qa_log: String) -> Result<String, String> {
        let (deep_id, cfg) = {
            let s = self.state.lock().unwrap();
            (s.deep_id.clone(), s.cfg.clone())
        };
        let Some(deep_id) = deep_id else {
            return Err("deep lane is not open yet".to_string());
        };
        let Some(cfg) = cfg else {
            return Err("assistant lanes are not configured".to_string());
        };
        // The note forks the deep session, whose history already holds every
        // frame the deep lane sent, so it gets only the frames newer than that.
        let (batch, has_frames) = {
            let s = self.state.lock().unwrap();
            let cursor = s.deep_frame_cursor;
            match s.frames.clone() {
                Some(feed) => (feed.select(cursor, FrameMode::All).0, true),
                None => (FrameBatch::none(), false),
            }
        };
        let mut prompt = format!("TRANSCRIPT:\n{}\n\nQ&A LOG:\n{}", transcript, qa_log);
        if has_frames {
            prompt.push_str(
                "\n\nPictures of the Zoom window sent earlier in this conversation, and any attached to this message, are the call's slides too.",
            );
        }
        let turn = build_turn(LaneCall {
            lane_id: deep_id,
            model: cfg.deep_model.clone(),
            effort: cfg.deep_effort.clone(),
            system_prompt: cfg.note_prompt.clone(),
            tools: LaneTools::ReadOnly,
            fork: true,
            cwd: cfg.cwd.clone(),
            add_dirs: cfg.deep_read_dirs.clone(),
            prompt,
            partial: false,
        });
        let state = self.state.clone();
        let open_state = self.state.clone();
        let outcome = run_with_retry(
            &self.runner,
            turn,
            batch,
            move || -> (DeltaCb, RegisterCb) {
                let state = state.clone();
                (
                    Box::new(|_| {}),
                    Box::new(move |handle| {
                        state.lock().unwrap().live_runs.push(handle);
                    }),
                )
            },
            || {},
            move || !open_state.lock().unwrap().closed,
        )
        .await;
        match outcome.error {
            Some(err) => Err(err),
            None => Ok(outcome.text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::TranscriptUpdate;
    use std::time::Duration;

    fn tu(source: &str, text: &str, start: f64, end: f64, is_partial: bool, seq: u64) -> TranscriptUpdate {
        TranscriptUpdate {
            text: text.to_string(),
            timestamp: "00:00:00".to_string(),
            source: source.to_string(),
            sequence_id: seq,
            chunk_start_time: start,
            is_partial,
            confidence: 1.0,
            audio_start_time: start,
            audio_end_time: end,
            duration: end - start,
        }
    }

    fn test_cfg() -> LaneConfig {
        LaneConfig {
            cwd: std::env::temp_dir(),
            fast_model: "fast-model".to_string(),
            fast_effort: "low".to_string(),
            deep_model: "deep-model".to_string(),
            deep_effort: "medium".to_string(),
            deep_read_dirs: Vec::new(),
            note_prompt: "note prompt".to_string(),
        }
    }

    impl AnswerLanes {
        fn for_test(runner: Runner, fast_id: &str, deep_id: &str, cfg: LaneConfig) -> Self {
            let lanes = AnswerLanes::new(runner);
            {
                let mut s = lanes.state.lock().unwrap();
                s.fast_id = Some(fast_id.to_string());
                s.deep_id = Some(deep_id.to_string());
                s.cfg = Some(cfg);
            }
            lanes
        }

        fn test_fast_cursor(&self) -> usize {
            self.state.lock().unwrap().fast_cursor
        }
    }

    /// Records every turn it is handed and always replies SKIP.
    fn skip_runner() -> (Runner, Arc<Mutex<Vec<ClaudeTurn>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_closure = calls.clone();
        let runner: Runner = Arc::new(move |turn, _on_delta, _register| {
            calls_for_closure.lock().unwrap().push(turn.clone());
            let outcome = ClaudeOutcome {
                session_id: None,
                text: "SKIP".to_string(),
                error: None,
            };
            Box::pin(async move { outcome })
        });
        (runner, calls)
    }

    /// Replies with `fast_text` to the fast model and `deep_text` to the
    /// deep model, streaming the text once before returning it as final.
    fn scripted_runner(fast_text: &'static str, deep_text: &'static str) -> Runner {
        Arc::new(move |turn, mut on_delta, _register| {
            let text = if turn.model.as_deref() == Some("fast-model") {
                fast_text
            } else {
                deep_text
            };
            on_delta(text.to_string());
            let outcome = ClaudeOutcome {
                session_id: None,
                text: text.to_string(),
                error: None,
            };
            Box::pin(async move { outcome })
        })
    }

    #[tokio::test]
    async fn second_rapid_answer_forks_without_advancing_cursor() {
        let (runner, calls) = skip_runner();
        let mut lanes = AnswerLanes::for_test(runner, "fast-1", "deep-1", test_cfg());
        let mut log = TranscriptLog::default();
        log.ingest(&tu("mic", "one", 0.0, 1.0, false, 1));
        log.ingest(&tu("system", "hello", 2.0, 3.0, false, 2));

        let emit: EmitFn = Arc::new(|_| {});
        lanes.answer("q1".to_string(), CardKind::Answer, &log, emit.clone());
        assert_eq!(lanes.test_fast_cursor(), 1);

        lanes.answer("q2".to_string(), CardKind::Answer, &log, emit);
        assert_eq!(lanes.test_fast_cursor(), 1, "a fork must not advance the cursor");
        tokio::time::sleep(Duration::from_millis(50)).await;

        let calls = calls.lock().unwrap();
        let fast_calls: Vec<&ClaudeTurn> = calls
            .iter()
            .filter(|t| t.resume.as_deref() == Some("fast-1"))
            .collect();
        assert_eq!(fast_calls.len(), 2);
        assert!(!fast_calls[0].fork, "the first trigger advances, it does not fork");
        assert!(fast_calls[1].fork, "the second rapid trigger forks instead of queueing");
    }

    #[tokio::test]
    async fn deep_confirms_fast_draft_settles_checked() {
        let runner = scripted_runner(
            "LEAD: We ship Proposal 3\n- Backend is done",
            "LEAD: We ship Proposal 3\n- Backend implementation is complete",
        );
        let mut lanes = AnswerLanes::for_test(runner, "fast-1", "deep-1", test_cfg());
        let log = TranscriptLog::default();
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let emitted_clone = emitted.clone();
        let emit: EmitFn = Arc::new(move |card| emitted_clone.lock().unwrap().push(card));

        lanes.answer("what do we ship?".to_string(), CardKind::Answer, &log, emit);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let cards = emitted.lock().unwrap();
        let settled = cards
            .iter()
            .find(|c| c.phase == "checked")
            .expect("deep pass should settle the card as checked");
        assert_eq!(settled.lead, "We ship Proposal 3");
        assert!(settled.changed_lines.is_empty());
    }

    #[tokio::test]
    async fn deep_contradicts_fast_draft_settles_corrected_with_changed_lines() {
        let runner = scripted_runner(
            "LEAD: We ship Proposal 3\n- Backend is done",
            "LEAD: We ship Proposal 4 instead\n- QA flagged a blocker on 3",
        );
        let mut lanes = AnswerLanes::for_test(runner, "fast-1", "deep-1", test_cfg());
        let log = TranscriptLog::default();
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let emitted_clone = emitted.clone();
        let emit: EmitFn = Arc::new(move |card| emitted_clone.lock().unwrap().push(card));

        lanes.answer("which proposal?".to_string(), CardKind::Answer, &log, emit);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let cards = emitted.lock().unwrap();
        let settled = cards
            .iter()
            .find(|c| c.phase == "corrected")
            .expect("a contradicted lead should settle as corrected");
        assert_eq!(settled.lead, "We ship Proposal 4 instead");
        assert!(!settled.changed_lines.is_empty());
    }

    #[tokio::test]
    async fn deep_skip_leaves_fast_draft_standing() {
        let runner = scripted_runner("LEAD: We ship Proposal 3\n- Backend is done", "SKIP");
        let mut lanes = AnswerLanes::for_test(runner, "fast-1", "deep-1", test_cfg());
        let log = TranscriptLog::default();
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let emitted_clone = emitted.clone();
        let emit: EmitFn = Arc::new(move |card| emitted_clone.lock().unwrap().push(card));

        lanes.answer("which proposal?".to_string(), CardKind::Answer, &log, emit);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let cards = emitted.lock().unwrap();
        assert!(
            !cards.is_empty(),
            "the fast draft should have streamed at least one drafting card"
        );
        assert!(
            cards.iter().all(|c| c.phase == "drafting"),
            "a deep SKIP must not emit a settle card, the draft stands as is"
        );
    }
    #[tokio::test]
    async fn brief_update_reaches_each_lane_once() {
        let (runner, calls) = skip_runner();
        let mut lanes = AnswerLanes::for_test(runner, "fast-1", "deep-1", test_cfg());
        let log = TranscriptLog::default();
        let emit: EmitFn = Arc::new(|_| {});

        lanes.set_brief("Learn their fees".to_string());
        lanes.answer("q1".to_string(), CardKind::Answer, &log, emit.clone());
        tokio::time::sleep(Duration::from_millis(50)).await;
        lanes.answer("q2".to_string(), CardKind::Answer, &log, emit);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let calls = calls.lock().unwrap();
        for lane in ["fast-1", "deep-1"] {
            let turns: Vec<&ClaudeTurn> = calls.iter().filter(|t| t.resume.as_deref() == Some(lane)).collect();
            assert_eq!(turns.len(), 2);
            assert!(turns[0].prompt.contains("The meeting brief was updated"), "{} first turn", lane);
            assert!(turns[0].prompt.contains("Learn their fees"));
            assert!(!turns[1].prompt.contains("Learn their fees"), "{} second turn", lane);
        }
    }

    #[tokio::test]
    async fn brief_update_on_a_fork_is_resent_on_the_next_primary_turn() {
        let (runner, calls) = skip_runner();
        let mut lanes = AnswerLanes::for_test(runner, "fast-1", "deep-1", test_cfg());
        let log = TranscriptLog::default();
        let emit: EmitFn = Arc::new(|_| {});

        lanes.answer("q1".to_string(), CardKind::Answer, &log, emit.clone());
        lanes.set_brief("Focus on maintenance".to_string());
        lanes.answer("q2".to_string(), CardKind::Answer, &log, emit.clone());
        tokio::time::sleep(Duration::from_millis(50)).await;
        lanes.answer("q3".to_string(), CardKind::Answer, &log, emit);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let calls = calls.lock().unwrap();
        let fast: Vec<&ClaudeTurn> = calls.iter().filter(|t| t.resume.as_deref() == Some("fast-1")).collect();
        assert_eq!(fast.len(), 3);
        assert!(!fast[0].prompt.contains("Focus on maintenance"));
        assert!(fast[1].fork);
        assert!(fast[1].prompt.contains("Focus on maintenance"));
        assert!(!fast[2].fork);
        assert!(fast[2].prompt.contains("Focus on maintenance"), "a fork must not mark the brief as seen");
    }

    #[tokio::test]
    async fn cleared_brief_is_announced() {
        let (runner, calls) = skip_runner();
        let mut lanes = AnswerLanes::for_test(runner, "fast-1", "deep-1", test_cfg());
        lanes.set_brief(String::new());
        lanes.answer("q1".to_string(), CardKind::Answer, &TranscriptLog::default(), Arc::new(|_| {}));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(calls.lock().unwrap()[0].prompt.contains("The meeting brief was cleared"));
    }


    // ---- Zoom frames ----

    fn fake_encoder() -> Encoder {
        Arc::new(|p: &Path| {
            Ok(ImageBlock {
                media_type: "image/jpeg".to_string(),
                base64: p.file_name().unwrap().to_string_lossy().to_string(),
            })
        })
    }

    fn store_with(n: u64) -> FrameStore {
        let store = FrameStore::new();
        for i in 1..=n {
            store.push(format!("/f/{}", i).into(), i as i64);
        }
        store
    }

    fn names(t: &ClaudeTurn) -> Vec<String> {
        t.images.iter().map(|i| i.base64.clone()).collect()
    }

    fn batch_names(feed: &FrameFeed, cursor: u64, mode: FrameMode) -> (Vec<u64>, u64) {
        let (b, latest) = feed.select(cursor, mode);
        (b.frames.iter().map(|f| f.seq).collect(), latest)
    }

    fn lanes_with_frames(runner: Runner, store: &FrameStore) -> AnswerLanes {
        let mut lanes = AnswerLanes::for_test(runner, "fast-1", "deep-1", test_cfg());
        lanes.set_frames(Some(FrameFeed::with_encoder(store.clone(), fake_encoder())));
        lanes
    }

    impl AnswerLanes {
        fn test_frame_cursors(&self) -> (u64, u64) {
            let s = self.state.lock().unwrap();
            (s.fast_frame_cursor, s.deep_frame_cursor)
        }
    }

    #[test]
    fn frame_selection_follows_each_lanes_own_cursor() {
        let store = store_with(3);
        let mut lanes = AnswerLanes::for_test(skip_runner().0, "f", "d", test_cfg());
        lanes.set_frames(Some(FrameFeed::with_encoder(store.clone(), fake_encoder())));
        let mut s = lanes.state.lock().unwrap();

        // Fast Answer at cursor 0 sends only the newest, cursor becomes 3.
        let b = take_frames(&mut s, Lane::Fast, FrameMode::Newest, false);
        assert_eq!(b.frames.iter().map(|f| f.seq).collect::<Vec<_>>(), vec![3]);
        assert_eq!(s.fast_frame_cursor, 3);

        store.push("/f/4".into(), 4);
        store.push("/f/5".into(), 5);

        // Deep (cursor 0) sends everything.
        let b = take_frames(&mut s, Lane::Deep, FrameMode::All, false);
        assert_eq!(b.frames.iter().map(|f| f.seq).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5]);
        assert_eq!(s.deep_frame_cursor, 5);

        // Fast Answer again: only the newest, 5.
        let b = take_frames(&mut s, Lane::Fast, FrameMode::Newest, false);
        assert_eq!(b.frames.iter().map(|f| f.seq).collect::<Vec<_>>(), vec![5]);

        // A fork sends the newest and leaves the cursor.
        store.push("/f/6".into(), 6);
        store.push("/f/7".into(), 7);
        let b = take_frames(&mut s, Lane::Fast, FrameMode::Newest, true);
        assert_eq!(b.frames.iter().map(|f| f.seq).collect::<Vec<_>>(), vec![7]);
        assert_eq!(s.fast_frame_cursor, 5);

        // Ask / catch-up send all since the cursor.
        let b = take_frames(&mut s, Lane::Fast, FrameMode::All, false);
        assert_eq!(b.frames.iter().map(|f| f.seq).collect::<Vec<_>>(), vec![6, 7]);
        assert_eq!(s.fast_frame_cursor, 7);

        // Nothing new, nothing sent.
        let b = take_frames(&mut s, Lane::Fast, FrameMode::All, false);
        assert!(b.frames.is_empty());
    }

    #[test]
    fn select_modes_and_note_batch() {
        let feed = FrameFeed::with_encoder(store_with(3), fake_encoder());
        assert_eq!(batch_names(&feed, 0, FrameMode::Newest), (vec![3], 3));
        assert_eq!(batch_names(&feed, 0, FrameMode::All), (vec![1, 2, 3], 3));
        assert_eq!(batch_names(&feed, 3, FrameMode::Newest), (vec![], 3));
    }

    #[tokio::test]
    async fn a_teams_frame_is_labelled_teams_and_a_zoom_frame_zoom() {
        let store = FrameStore::new();
        store.push_from(CallApp::Teams, "/f/1".into(), 1);
        store.push("/f/2".into(), 2);
        let (runner, calls) = skip_runner();
        let lanes = lanes_with_frames(runner, &store);
        let log = TranscriptLog::default();
        let emit: EmitFn = Arc::new(|_| {});
        let mut lanes = lanes;
        lanes.answer("q".to_string(), CardKind::Ask, &log, emit);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let calls = calls.lock().unwrap();
        let deep = calls.iter().find(|t| t.resume.as_deref() == Some("deep-1")).unwrap();
        assert!(deep.image_labels[0].starts_with("Teams window at "));
        assert!(deep.image_labels[1].starts_with("Zoom window at "));
    }

    #[tokio::test]
    async fn answer_sends_frames_to_each_lane_and_switch_off_sends_none() {
        let store = store_with(3);
        let (runner, calls) = skip_runner();
        let mut lanes = lanes_with_frames(runner, &store);
        let log = TranscriptLog::default();
        let emit: EmitFn = Arc::new(|_| {});

        lanes.answer("q1".to_string(), CardKind::Answer, &log, emit.clone());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(lanes.test_frame_cursors(), (3, 3));
        store.push("/f/4".into(), 4);
        store.push("/f/5".into(), 5);
        lanes.answer("q2".to_string(), CardKind::Ask, &log, emit.clone());
        tokio::time::sleep(Duration::from_millis(50)).await;

        {
            let calls = calls.lock().unwrap();
            let fast: Vec<&ClaudeTurn> = calls.iter().filter(|t| t.resume.as_deref() == Some("fast-1")).collect();
            let deep: Vec<&ClaudeTurn> = calls.iter().filter(|t| t.resume.as_deref() == Some("deep-1")).collect();
            assert_eq!(names(fast[0]), vec!["3"], "suggested answer: newest only");
            assert_eq!(names(deep[0]), vec!["1", "2", "3"]);
            assert_eq!(names(fast[1]), vec!["4", "5"], "a direct ask: all since the cursor");
            assert_eq!(names(deep[1]), vec!["4", "5"]);
            assert!(fast[0].image_labels[0].starts_with("Zoom window at "));
            assert!(fast[0].stdin_line().is_some());
        }

        // Switch off: no feed, no images, argv equals a plain turn's.
        let (runner, calls) = skip_runner();
        let mut off = AnswerLanes::for_test(runner, "fast-1", "deep-1", test_cfg());
        off.answer("q".to_string(), CardKind::Answer, &log, emit);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let calls = calls.lock().unwrap();
        assert!(calls.iter().all(|t| t.images.is_empty() && t.stdin_line().is_none()));
        assert_eq!(calls[0].argv()[0..2], ["-p".to_string(), calls[0].prompt.clone()]);
    }

    #[tokio::test]
    async fn explain_sends_newest_frame_and_catchup_sends_all() {
        let store = store_with(3);
        let (runner, calls) = skip_runner();
        let mut lanes = lanes_with_frames(runner, &store);
        let mut log = TranscriptLog::default();
        log.ingest(&tu("system", "hello there", 0.0, 1.0, false, 1));
        let emit: EmitFn = Arc::new(|_| {});
        lanes.catchup(&log, emit.clone());
        tokio::time::sleep(Duration::from_millis(50)).await;
        store.push("/f/4".into(), 4);
        store.push("/f/5".into(), 5);
        lanes.catchup(&log, emit);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let calls = calls.lock().unwrap();
        assert_eq!(names(&calls[0]), vec!["1", "2", "3"]);
        assert_eq!(names(&calls[1]), vec!["4", "5"]);
    }

    /// Fails whenever the turn carries images, after streaming junk and with
    /// no final text; succeeds when it carries none.
    fn fail_with_images_runner() -> (Runner, Arc<Mutex<Vec<ClaudeTurn>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let runner: Runner = Arc::new(move |turn, mut on_delta, _register| {
            let failing = !turn.images.is_empty();
            recorded.lock().unwrap().push(turn);
            if failing {
                on_delta("LEAD: junk from the failed try".to_string());
            } else {
                on_delta("LEAD: ok".to_string());
            }
            let outcome = ClaudeOutcome {
                session_id: None,
                text: String::new(),
                error: failing.then(|| "image rejected".to_string()),
            };
            Box::pin(async move { outcome })
        });
        (runner, calls)
    }

    #[tokio::test]
    async fn a_failed_turn_with_images_runs_once_more_without_them() {
        let store = store_with(2);
        let (runner, calls) = fail_with_images_runner();
        let mut lanes = lanes_with_frames(runner, &store);
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let e = emitted.clone();
        let emit: EmitFn = Arc::new(move |c| e.lock().unwrap().push(c));
        lanes.answer("q".to_string(), CardKind::Answer, &TranscriptLog::default(), emit);
        tokio::time::sleep(Duration::from_millis(80)).await;

        let calls = calls.lock().unwrap();
        for lane in ["fast-1", "deep-1"] {
            let t: Vec<&ClaudeTurn> = calls.iter().filter(|t| t.resume.as_deref() == Some(lane)).collect();
            assert_eq!(t.len(), 2, "{}: one try with images, one retry", lane);
            assert!(!t[0].images.is_empty());
            assert!(t[1].images.is_empty() && t[1].image_labels.is_empty());
            assert_eq!(t[0].prompt, t[1].prompt, "same prompt");
        }
        assert_eq!(lanes.test_frame_cursors(), (2, 2), "the cursor stays advanced");
        // The failed attempt's streamed text was dropped, not glued on.
        let cards = emitted.lock().unwrap();
        let last_checked = cards.iter().rev().find(|c| c.phase != "drafting").expect("deep settles");
        assert_eq!(last_checked.lead, "ok");
    }

    #[tokio::test]
    async fn a_failed_turn_without_images_is_not_retried() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let runner: Runner = Arc::new(move |turn, _d, _r| {
            recorded.lock().unwrap().push(turn);
            Box::pin(async {
                ClaudeOutcome { session_id: None, text: String::new(), error: Some("boom".into()) }
            })
        });
        let mut lanes = AnswerLanes::for_test(runner, "fast-1", "deep-1", test_cfg());
        lanes.answer("q".to_string(), CardKind::Answer, &TranscriptLog::default(), Arc::new(|_| {}));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(calls.lock().unwrap().len(), 2, "one per lane, no retries");
    }

    #[tokio::test]
    async fn the_note_sends_frames_the_deep_lane_has_not_sent_and_retries_without_them() {
        let store = store_with(3);
        let (runner, calls) = fail_with_images_runner();
        let mut lanes = lanes_with_frames(runner, &store);
        // Deep never ran: the note sends every frame.
        let out = lanes.draft_note("t".to_string(), "qa".to_string()).await;
        assert!(out.is_ok(), "{:?}", out);
        {
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert_eq!(names(&calls[0]), vec!["1", "2", "3"]);
            assert!(calls[0].prompt.contains("sent earlier in this conversation"));
            assert!(calls[1].images.is_empty());
        }

        // Deep has sent up to 2: the note sends only 3 and 4.
        lanes.state.lock().unwrap().deep_frame_cursor = 2;
        store.push("/f/4".into(), 4);
        calls.lock().unwrap().clear();
        let _ = lanes.draft_note("t".to_string(), "qa".to_string()).await;
        assert_eq!(names(&calls.lock().unwrap()[0]), vec!["3", "4"]);

        // Switch off: no pictures, and the prompt says nothing about them.
        let (runner, calls) = skip_runner();
        let mut off = AnswerLanes::for_test(runner, "fast-1", "deep-1", test_cfg());
        let _ = off.draft_note("t".to_string(), "qa".to_string()).await;
        let calls = calls.lock().unwrap();
        assert!(calls[0].images.is_empty() && !calls[0].prompt.contains("Pictures of the Zoom"));
    }

    #[tokio::test]
    async fn close_during_an_image_turn_starts_no_retry() {
        let store = store_with(2);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let started_tx = Arc::new(Mutex::new(Some(started_tx)));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let g = gate.clone();
        let runner: Runner = Arc::new(move |turn, _d, _r| {
            recorded.lock().unwrap().push(turn);
            if let Some(tx) = started_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
            let g = g.clone();
            Box::pin(async move {
                g.acquire().await.unwrap().forget();
                ClaudeOutcome { session_id: None, text: String::new(), error: Some("killed".into()) }
            })
        });
        let mut lanes = lanes_with_frames(runner, &store);
        lanes.answer("q".to_string(), CardKind::Answer, &TranscriptLog::default(), Arc::new(|_| {}));
        started_rx.await.unwrap();
        lanes.close();
        gate.add_permits(10);
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(
            calls.lock().unwrap().iter().all(|t| !t.images.is_empty()),
            "no no-image retry after close"
        );
    }

    #[tokio::test]
    async fn a_frame_that_fails_to_encode_is_skipped() {
        let store = store_with(3);
        let enc: Encoder = Arc::new(|p: &Path| {
            if p.ends_with("2") {
                Err("bad".to_string())
            } else {
                Ok(ImageBlock { media_type: "image/png".into(), base64: "x".into() })
            }
        });
        let feed = FrameFeed::with_encoder(store, enc);
        let mut turn = ClaudeTurn::default();
        feed.batch(feed.store.all()).attach(&mut turn).await;
        assert_eq!(turn.images.len(), 2);
        assert_eq!(turn.image_labels.len(), 2);
    }

    // ---- Live speed check ----

    const SPEED_CALL: &[(&str, &str)] = &[
        ("system", "Thanks for making time today, Joseph. We wanted to walk through how the migration would roll out."),
        ("mic", "Happy to. Before we start, can you tell me how many sites are in scope?"),
        ("system", "Right now it is four data centers and about sixty clusters."),
        ("mic", "Okay, and are all of them on the same version?"),
        ("system", "Mostly. Three of the data centers are on the current release and one is two versions behind."),
        ("mic", "That one will need an upgrade first, I would think."),
        ("system", "Yes, that was our thinking too. The team is nervous about downtime."),
        ("mic", "We can keep it rolling so workloads move without a stop."),
        ("system", "That helps. Our change window is Saturday night only."),
        ("mic", "Understood. Who signs off on the change window?"),
        ("system", "The infrastructure director, and she wants a written plan two weeks ahead."),
        ("mic", "I can have a draft to you by Friday."),
        ("system", "Great. Now, on the licensing side, finance is asking about the timing."),
        ("mic", "We can line the renewal up with the last wave."),
        ("system", "That works for them. Another thing, the storage team wants to go first."),
        ("mic", "Storage first is fine as long as networking is ready."),
        ("system", "Networking says they need three weeks of lead time."),
        ("mic", "Then we start storage in week four."),
        ("system", "Okay. One more thing before we wrap this section."),
        ("system", "Joseph, can you walk us through the wave plan, which sites go in which wave and how long each wave takes?"),
    ];

    /// Milliseconds from firing `answer` to the first drafting card, with the
    /// default settings' models, a fresh session each run.
    async fn speed_run(with_frame: bool, image: &Path) -> Result<u128, String> {
        let settings = crate::assistant::AssistantSettings::default();
        let bin = crate::assistant::settings::resolve_claude_binary(&settings).ok_or("claude binary not found")?;
        let cwd = std::env::temp_dir().join(format!("meetily-speed-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;
        let cfg = LaneConfig {
            cwd: cwd.clone(),
            fast_model: settings.fast_model.clone(),
            fast_effort: settings.fast_effort.clone(),
            deep_model: settings.deep_model.clone(),
            deep_effort: settings.deep_effort.clone(),
            deep_read_dirs: Vec::new(),
            note_prompt: "note prompt".to_string(),
        };
        let mut lanes = AnswerLanes::new(claude_runner(bin));
        let store = FrameStore::new();
        if with_frame {
            store.push(image.to_path_buf(), now_ms());
        }
        // The production encoder: downscaled to 1280 wide, JPEG, as in a call.
        lanes.set_frames(Some(FrameFeed::new(store)));
        let noop: EmitFn = Arc::new(|_| {});
        lanes
            .open(
                "Meeting: Migration planning call\nDate: 2026-10-14\n\nBrief:\nLearn the wave plan constraints and agree who signs off.".to_string(),
                &cfg,
                noop,
            )
            .await;
        if !lanes.is_ready() {
            return Err("lanes did not open".to_string());
        }

        let mut log = TranscriptLog::default();
        for (n, (src, text)) in SPEED_CALL.iter().enumerate() {
            log.ingest(&tu(src, text, n as f64 * 3.0, n as f64 * 3.0 + 2.0, false, n as u64 + 1));
        }
        let first: Arc<Mutex<Option<std::time::Instant>>> = Arc::new(Mutex::new(None));
        let f = first.clone();
        let emit: EmitFn = Arc::new(move |c| {
            if c.phase == "drafting" {
                f.lock().unwrap().get_or_insert_with(std::time::Instant::now);
            }
        });
        let question = SPEED_CALL.last().unwrap().1.to_string();
        let fired = std::time::Instant::now();
        lanes.answer(question, CardKind::Answer, &log, emit);
        let result = loop {
            if let Some(t) = *first.lock().unwrap() {
                break Ok(t.duration_since(fired).as_millis());
            }
            if fired.elapsed() > Duration::from_secs(120) {
                break Err("no drafting card within 120 s".to_string());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        lanes.close();
        let _ = std::fs::remove_dir_all(&cwd);
        result
    }

    /// Time to the first words on screen, frame off vs on. Spends real turns:
    /// ASSISTANT_SPEED_TEST=1 and VISION_SPEED_IMAGE=<png> required.
    #[tokio::test]
    #[ignore]
    async fn live_speed_with_and_without_a_frame() {
        if std::env::var("ASSISTANT_SPEED_TEST").ok().as_deref() != Some("1") {
            eprintln!("skipping live_speed_with_and_without_a_frame: set ASSISTANT_SPEED_TEST=1 to run");
            return;
        }
        let image = PathBuf::from(std::env::var("VISION_SPEED_IMAGE").expect("set VISION_SPEED_IMAGE to an image file"));
        let (mut off, mut on) = (Vec::new(), Vec::new());
        for run in 0..10 {
            let with_frame = run % 2 == 1;
            let ms = speed_run(with_frame, &image).await.expect("speed run");
            println!("run {:2} {:>9}: {} ms to first drafting card", run / 2 + 1, if with_frame { "frame ON" } else { "frame OFF" }, ms);
            if with_frame { on.push(ms) } else { off.push(ms) }
        }
        let median = |v: &mut Vec<u128>| {
            v.sort();
            v[v.len() / 2]
        };
        println!("RESULT frame OFF median {} ms, runs {:?}", median(&mut off), off);
        println!("RESULT frame ON  median {} ms, runs {:?}", median(&mut on), on);
    }
}
