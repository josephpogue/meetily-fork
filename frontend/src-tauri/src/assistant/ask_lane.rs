// assistant/ask_lane.rs
//
// Ask Next: claude lineages forked from the same trunk as the answer lanes.
// Like the answer cards, the list comes in two passes: a quick sub-lane
// (the fast model, no tools) shows a list first, and a deep sub-lane (the
// deep model, read-only tools over Joseph's vault) later replaces or
// corrects it. Each turn sends what changed since that sub-lane's last one
// (transcript delta, a brief edit, dismissals) and gets back the whole list
// of questions Joseph should still ask. At most one turn per sub-lane runs
// at a time. Requests that land meanwhile coalesce into one follow-up,
// fired through `AskHooks::rerun`, because only core holds the transcript.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use super::ask_list::{self, AskItem, AskList, RawList};
use super::claude_cli::{
    ClaudeOutcome, ClaudeTurn, TurnHandle, NO_TOOLS, READ_ONLY_TOOLS, WRITE_TOOLS,
};
use super::lanes::{run_with_retry, DeltaCb, FrameBatch, FrameFeed, FrameMode, RegisterCb, Runner};
use super::transcript::TranscriptLog;

pub const ASK_PROMPT: &str = include_str!("prompts/ask.md");
/// Appended after `ASK_PROMPT` for the deep sub-lane.
pub const ASK_DEEP_PROMPT: &str = include_str!("prompts/ask_deep.md");

/// The deep sub-lane's system prompt: the shared contract, then its extra role.
pub fn ask_deep_system_prompt() -> String {
    format!("{}\n\n{}", ASK_PROMPT, ASK_DEEP_PROMPT)
}

/// Higher than the fast lane's low: the quality of the questions is the
/// whole point, and nobody is waiting two seconds on this list.
const ASK_EFFORT: &str = "medium";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AsksOut {
    /// "idle" | "thinking" | "failed" | "unavailable"
    pub state: String,
    pub asks: Vec<AskItem>,
    pub for_you: Vec<AskItem>,
    pub updated_at: Option<i64>,
    pub brief: String,
    /// True while the shown list came from the deep sub-lane.
    pub checked: bool,
    /// True while the shown list is a quick one and the deep sub-lane is
    /// still working on a fuller one.
    pub checking: bool,
    /// Ids of the items the last deep result added or reworded relative to
    /// the list it replaced. Empty for a quick result.
    pub changed: Vec<String>,
}

pub type AskEmitFn = Arc<dyn Fn(AsksOut) + Send + Sync>;

#[derive(Clone)]
pub struct AskHooks {
    pub emit: AskEmitFn,
    /// Called once when a turn finishes and a sub-lane has a follow-up
    /// waiting. Core answers by calling `AskLane::rerun`.
    pub rerun: Arc<dyn Fn() + Send + Sync>,
    pub on_error: Arc<dyn Fn(String) + Send + Sync>,
}

impl Default for AskHooks {
    fn default() -> Self {
        Self {
            emit: Arc::new(|_| {}),
            rerun: Arc::new(|| {}),
            on_error: Arc::new(|_| {}),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AskConfig {
    pub cwd: PathBuf,
    /// The fast sub-lane's model.
    pub model: String,
    pub deep_model: String,
    pub deep_effort: String,
    /// Extra directories the deep sub-lane may read (Joseph's vault).
    pub deep_read_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Fast,
    Deep,
}

impl Which {
    const ALL: [Which; 2] = [Which::Fast, Which::Deep];

    fn idx(self) -> usize {
        match self {
            Which::Fast => 0,
            Which::Deep => 1,
        }
    }
}

/// Which sub-lane wrote the list on screen.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Shown {
    #[default]
    None,
    Fast,
    Deep,
}

/// One claude lineage and everything that is private to it.
#[derive(Default)]
struct SubLane {
    lane_id: Option<String>,
    /// Transcript cursor; advances only when a turn succeeds.
    cursor: usize,
    in_flight: bool,
    /// A request landed while a turn ran; one forced follow-up is owed.
    dirty: bool,
    /// Last frame seq sent; advances only when a turn succeeds.
    frame_cursor: u64,
    /// The brief version the model last received. The trunk seed already
    /// carries the opening brief, so brief_version and this start at 0.
    brief_sent: u64,
    /// How many of `AskState::dismissed` the model has been told about.
    dismissed_sent: usize,
}

#[derive(Default)]
struct AskState {
    cfg: Option<AskConfig>,
    subs: [SubLane; 2],
    /// Set by `open` when no sub-lane could be forked.
    unavailable: bool,
    failed: bool,
    list: AskList,
    updated_at: Option<i64>,
    brief: String,
    /// Bumped on every brief edit.
    brief_version: u64,
    /// Every question Joseph dismissed this session, for filtering and for
    /// telling each sub-lane once.
    dismissed: Vec<String>,
    /// Bumped by `close`; a turn that finishes under an older generation is
    /// dropped on the floor.
    generation: u64,
    live_runs: Vec<TurnHandle>,
    /// Zoom window frames, when the assistant can see the window.
    frames: Option<FrameFeed>,
    /// Transcript point the shown list covers, and who wrote it.
    shown_point: usize,
    shown_by: Shown,
    /// The largest point a quick result was applied at.
    fast_applied_point: usize,
    changed: Vec<String>,
}

/// Everything one fired turn needs once it comes back.
struct Job {
    which: Which,
    turn: ClaudeTurn,
    batch: FrameBatch,
    new_frame_cursor: u64,
    generation: u64,
    sent_version: u64,
    sent_dismissed: usize,
    new_cursor: usize,
}

struct Landed {
    out: AsksOut,
    /// A sub-lane has a follow-up waiting.
    rerun: bool,
    error: Option<String>,
}

impl AskState {
    fn sub(&self, which: Which) -> &SubLane {
        &self.subs[which.idx()]
    }

    fn sub_mut(&mut self, which: Which) -> &mut SubLane {
        &mut self.subs[which.idx()]
    }

    fn snapshot(&self) -> AsksOut {
        let any_flight = self.subs.iter().any(|s| s.in_flight);
        let state = if self.unavailable {
            "unavailable"
        } else if any_flight {
            "thinking"
        } else if self.failed {
            "failed"
        } else {
            "idle"
        };
        AsksOut {
            state: state.to_string(),
            asks: self.list.asks.clone(),
            for_you: self.list.for_you.clone(),
            updated_at: self.updated_at,
            brief: self.brief.clone(),
            checked: self.shown_by == Shown::Deep,
            checking: self.sub(Which::Deep).in_flight && self.shown_by != Shown::Deep,
            changed: self.changed.clone(),
        }
    }
}

impl AskState {
    /// Builds the turn for one sub-lane, or None when it has nothing to do:
    /// no lane, a turn already running (a follow-up is then owed), or no
    /// news and not forced. `rerun` fires only a sub-lane that owes a
    /// follow-up.
    fn prepare(&mut self, which: Which, log: &TranscriptLog, force: bool, rerun: bool) -> Option<Job> {
        let cfg = self.cfg.clone()?;
        let sub = self.sub(which);
        let lane_id = sub.lane_id.clone()?;
        if rerun && (!sub.dirty || sub.in_flight) {
            return None;
        }
        if sub.in_flight {
            self.sub_mut(which).dirty = true;
            return None;
        }
        let (delta, new_cursor) = log.delta_since(sub.cursor);
        let in_progress = log.open_preview();
        let brief_changed = self.brief_version != sub.brief_sent;
        let unsent = &self.dismissed[sub.dismissed_sent.min(self.dismissed.len())..];
        let has_news = !delta.is_empty() || brief_changed || !unsent.is_empty();
        if !force && !has_news {
            return None;
        }
        let prompt = ask_prompt(
            &delta,
            &in_progress,
            brief_changed.then(|| self.brief.as_str()),
            unsent,
            &self.list,
        );
        let sent_dismissed = self.dismissed.len();
        let sent_version = self.brief_version;
        let (batch, new_frame_cursor) = match self.frames.clone() {
            Some(feed) => feed.select(sub.frame_cursor, FrameMode::All),
            None => (FrameBatch::default(), sub.frame_cursor),
        };
        let turn = lane_turn(&cfg, which, lane_id, prompt, false);
        let generation = self.generation;
        let sub = self.sub_mut(which);
        sub.in_flight = true;
        sub.dirty = false;
        Some(Job {
            which,
            turn,
            batch,
            new_frame_cursor,
            generation,
            sent_version,
            sent_dismissed,
            new_cursor,
        })
    }

    /// Takes in a finished turn. The sub-lane's own cursors advance on any
    /// readable reply, because its model has seen that transcript; whether
    /// the list changes is a separate question.
    fn land(&mut self, job: &Job, parsed: Option<RawList>, error: Option<String>) -> Landed {
        let which = job.which;
        let point = job.new_cursor;
        let mut owes_fast = false;
        let sub = self.sub_mut(which);
        sub.in_flight = false;
        let error = match parsed {
            Some(raw) => {
                sub.cursor = job.new_cursor;
                sub.frame_cursor = job.new_frame_cursor;
                sub.brief_sent = job.sent_version;
                sub.dismissed_sent = job.sent_dismissed;
                match which {
                    Which::Fast => {
                        let applies = point > self.shown_point
                            || (point == self.shown_point && self.shown_by != Shown::Deep);
                        if applies {
                            self.list = ask_list::build(&self.list, raw, &self.dismissed);
                            self.changed.clear();
                            self.shown_by = Shown::Fast;
                            self.shown_point = point;
                            self.fast_applied_point = self.fast_applied_point.max(point);
                            self.updated_at = Some(chrono::Utc::now().timestamp_millis());
                            self.failed = false;
                        } else {
                            log::debug!(
                                "assistant ask: quick list at point {} dropped, the checked list covers point {}",
                                point,
                                self.shown_point
                            );
                        }
                    }
                    Which::Deep => {
                        let next = ask_list::build(&self.list, raw, &self.dismissed);
                        self.changed = changed_ids(&self.list, &next);
                        self.list = next;
                        self.shown_by = Shown::Deep;
                        self.shown_point = point;
                        self.updated_at = Some(chrono::Utc::now().timestamp_millis());
                        self.failed = false;
                        // The quick lane already heard more than this list
                        // covers: let it lay that over the checked list.
                        owes_fast = self.fast_applied_point > point;
                    }
                }
                None
            }
            None => {
                let on_screen = !self.list.asks.is_empty() || !self.list.for_you.is_empty();
                if which == Which::Fast || !on_screen {
                    self.failed = true;
                }
                Some(error.unwrap_or_else(|| "Ask next could not read the model's reply.".to_string()))
            }
        };
        if owes_fast && self.sub(Which::Fast).lane_id.is_some() {
            self.sub_mut(Which::Fast).dirty = true;
        } else {
            owes_fast = false;
        }
        let rerun = owes_fast || self.sub(which).dirty;
        Landed { out: self.snapshot(), rerun, error }
    }
}

/// Ids of the items in `next` that are new, or worded differently, relative
/// to `prev`.
fn changed_ids(prev: &AskList, next: &AskList) -> Vec<String> {
    let before = |id: &str| prev.asks.iter().chain(prev.for_you.iter()).find(|p| p.id == id);
    next.asks
        .iter()
        .chain(next.for_you.iter())
        .filter(|n| before(&n.id).map_or(true, |p| p.question != n.question))
        .map(|n| n.id.clone())
        .collect()
}

/// The claude turn for one sub-lane. The fast one reads nothing; the deep
/// one may read the vault and never write or reach the network, the same
/// policy as the deep answer lane.
fn lane_turn(cfg: &AskConfig, which: Which, lane_id: String, prompt: String, fork: bool) -> ClaudeTurn {
    let mut turn = ClaudeTurn {
        prompt,
        cwd: cfg.cwd.clone(),
        resume: Some(lane_id),
        fork,
        partial: false,
        safe_mode: true,
        ..Default::default()
    };
    match which {
        Which::Fast => {
            turn.model = Some(cfg.model.clone());
            turn.effort = Some(ASK_EFFORT.to_string());
            turn.append_system_prompt = Some(ASK_PROMPT.to_string());
            turn.disallowed_tools = no_tools();
        }
        Which::Deep => {
            turn.model = Some(cfg.deep_model.clone());
            turn.effort = Some(cfg.deep_effort.clone());
            turn.append_system_prompt = Some(ask_deep_system_prompt());
            turn.allowed_tools = READ_ONLY_TOOLS.iter().map(|s| s.to_string()).collect();
            turn.disallowed_tools = WRITE_TOOLS.iter().map(|s| s.to_string()).collect();
            turn.add_dirs = cfg.deep_read_dirs.clone();
        }
    }
    turn
}

pub struct AskLane {
    runner: Runner,
    hooks: AskHooks,
    state: Arc<Mutex<AskState>>,
}

impl AskLane {
    pub fn new(runner: Runner, hooks: AskHooks) -> Self {
        Self {
            runner,
            hooks,
            state: Arc::new(Mutex::new(AskState::default())),
        }
    }

    /// Forks both ask lineages off the seeded trunk. `trunk_id` is None when
    /// the trunk seed failed; the lane then reports "unavailable" for this
    /// call and every request is a no-op. A deep fork that fails leaves the
    /// quick sub-lane working alone.
    pub async fn open(&self, trunk_id: Option<String>, cfg: AskConfig, brief: String) {
        {
            let mut s = self.state.lock().unwrap();
            s.brief = brief;
            s.cfg = Some(cfg.clone());
        }
        let (fast_id, deep_id) = match trunk_id {
            Some(trunk) => {
                tokio::join!(
                    self.fork(&trunk, &cfg, Which::Fast),
                    self.fork(&trunk, &cfg, Which::Deep)
                )
            }
            None => (None, None),
        };
        let out = {
            let mut s = self.state.lock().unwrap();
            s.unavailable = fast_id.is_none() && deep_id.is_none();
            s.sub_mut(Which::Fast).lane_id = fast_id;
            s.sub_mut(Which::Deep).lane_id = deep_id;
            s.snapshot()
        };
        (self.hooks.emit)(out);
    }

    async fn fork(&self, trunk: &str, cfg: &AskConfig, which: Which) -> Option<String> {
        let mut turn = lane_turn(
            cfg,
            which,
            trunk.to_string(),
            "Standing by. Reply with exactly the word READY.".to_string(),
            true,
        );
        // The standby turn reads nothing.
        turn.allowed_tools.clear();
        turn.add_dirs.clear();
        turn.disallowed_tools = no_tools();
        let outcome = self.run_bare(turn).await;
        match outcome.error {
            Some(err) => {
                log::warn!("assistant ask {:?} lane fork failed: {}", which, err);
                None
            }
            None => outcome.session_id,
        }
    }

    async fn run_bare(&self, turn: ClaudeTurn) -> ClaudeOutcome {
        let state = self.state.clone();
        let register: Box<dyn FnOnce(TurnHandle) + Send> = Box::new(move |handle| {
            state.lock().unwrap().live_runs.push(handle);
        });
        (self.runner)(turn, Box::new(|_| {}), register).await
    }

    /// Runs one ask turn per idle sub-lane now, or marks a follow-up for a
    /// sub-lane that is already running. `force` runs even when nothing
    /// changed (the Refresh button); an automatic request with nothing new
    /// is a no-op for that sub-lane.
    pub fn request(&self, log: &TranscriptLog, force: bool) {
        self.fire(log, force, false);
    }

    /// The follow-up core runs after `AskHooks::rerun`: only the sub-lanes
    /// that owe one, forced.
    pub fn rerun(&self, log: &TranscriptLog) {
        self.fire(log, true, true);
    }

    fn fire(&self, log: &TranscriptLog, force: bool, rerun: bool) {
        let (jobs, out) = {
            let mut s = self.state.lock().unwrap();
            let jobs: Vec<Job> = Which::ALL
                .into_iter()
                .filter_map(|which| s.prepare(which, log, force, rerun))
                .collect();
            if jobs.is_empty() {
                return;
            }
            (jobs, s.snapshot())
        };
        (self.hooks.emit)(out);
        for job in jobs {
            self.spawn(job);
        }
    }

    fn spawn(&self, job: Job) {
        let runner = self.runner.clone();
        let state = self.state.clone();
        let hooks = self.hooks.clone();
        tokio::spawn(async move {
            let register_state = state.clone();
            let generation = job.generation;
            let outcome = run_with_retry(
                &runner,
                job.turn.clone(),
                job.batch.clone(),
                move || -> (DeltaCb, RegisterCb) {
                    let register_state = register_state.clone();
                    (
                        Box::new(|_| {}),
                        Box::new(move |handle| {
                            register_state.lock().unwrap().live_runs.push(handle);
                        }),
                    )
                },
                || {},
                || state.lock().unwrap().generation == generation,
            )
            .await;
            let parsed = match &outcome.error {
                None => ask_list::parse(&outcome.text),
                Some(_) => None,
            };

            let landed = {
                let mut s = state.lock().unwrap();
                if s.generation != generation {
                    return;
                }
                s.land(&job, parsed, outcome.error.clone())
            };
            (hooks.emit)(landed.out);
            if let Some(err) = landed.error {
                log::warn!("assistant ask {:?} turn failed: {}", job.which, err);
                (hooks.on_error)(err);
            }
            if landed.rerun {
                (hooks.rerun)();
            }
        });
    }

    /// Hands the lane the frames of the Zoom window, or None when the
    /// assistant cannot see it. The cursors restart at 0.
    pub fn set_frames(&self, feed: Option<FrameFeed>) {
        let mut s = self.state.lock().unwrap();
        s.frames = feed;
        for sub in s.subs.iter_mut() {
            sub.frame_cursor = 0;
        }
    }

    #[cfg(test)]
    pub(super) fn has_frames(&self) -> bool {
        self.state.lock().unwrap().frames.is_some()
    }

    pub fn set_brief(&self, text: String) {
        let out = {
            let mut s = self.state.lock().unwrap();
            if s.brief == text {
                return;
            }
            s.brief = text;
            s.brief_version += 1;
            s.snapshot()
        };
        (self.hooks.emit)(out);
    }

    /// Removes the item at once and remembers it so it never comes back,
    /// not even from a deep turn that is already running. False when no item
    /// has that id.
    pub fn dismiss(&self, id: &str) -> bool {
        let out = {
            let mut s = self.state.lock().unwrap();
            let removed = if let Some(pos) = s.list.asks.iter().position(|i| i.id == id) {
                s.list.asks.remove(pos)
            } else if let Some(pos) = s.list.for_you.iter().position(|i| i.id == id) {
                s.list.for_you.remove(pos)
            } else {
                return false;
            };
            s.dismissed.push(removed.question);
            s.snapshot()
        };
        (self.hooks.emit)(out);
        true
    }

    /// Kills in-flight turns and stops all further turns. The last list
    /// stays visible until the next session opens a fresh lane.
    pub fn close(&self) {
        let (handles, out) = {
            let mut s = self.state.lock().unwrap();
            s.generation += 1;
            for sub in s.subs.iter_mut() {
                sub.in_flight = false;
                sub.dirty = false;
                sub.lane_id = None;
            }
            (std::mem::take(&mut s.live_runs), s.snapshot())
        };
        for handle in handles {
            tokio::spawn(async move {
                handle.kill().await;
            });
        }
        (self.hooks.emit)(out);
    }

    pub fn snapshot(&self) -> AsksOut {
        self.state.lock().unwrap().snapshot()
    }

    /// The ASK questions still on the list, for the end-of-meeting note.
    pub fn open_asks(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .list
            .asks
            .iter()
            .map(|a| a.question.clone())
            .collect()
    }
}

fn no_tools() -> Vec<String> {
    NO_TOOLS.iter().map(|s| s.to_string()).collect()
}

fn ask_prompt(
    delta: &str,
    in_progress: &str,
    brief: Option<&str>,
    dismissed: &[String],
    current: &AskList,
) -> String {
    let mut out = String::new();
    if let Some(brief) = brief {
        if brief.trim().is_empty() {
            out.push_str("MEETING BRIEF: Joseph cleared the brief. Infer the purpose of the call from the transcript.\n\n");
        } else {
            out.push_str("MEETING BRIEF (updated, this replaces the earlier one):\n");
            out.push_str(brief.trim());
            out.push_str("\n\n");
        }
    }
    if !delta.is_empty() {
        out.push_str("NEW TRANSCRIPT since your last turn:\n");
        out.push_str(delta);
        out.push_str("\n\n");
    }
    if !in_progress.is_empty() {
        out.push_str("STILL SPEAKING (may be cut off mid-sentence):\n");
        out.push_str(in_progress);
        out.push_str("\n\n");
    }
    if !dismissed.is_empty() {
        out.push_str("Joseph dismissed these. Never suggest them again:\n");
        for q in dismissed {
            out.push_str("- ");
            out.push_str(q);
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str("CURRENT LIST:\n");
    out.push_str(&ask_list::render(current));
    out.push_str("\n\nReply with the full updated list in the ASK / YOU format, or NONE.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::claude_cli::ClaudeOutcome;
    use crate::audio::TranscriptUpdate;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Semaphore;

    const GOOD: &str = "ASK: How do you screen tenants? | WHY: brief asks about screening\nYOU: Is pricing in scope?";

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

    /// Them says one thing (closed by the speaker flip), You is mid-utterance.
    fn sample_log() -> TranscriptLog {
        let mut log = TranscriptLog::default();
        log.ingest(&tu("system", "our fee is ten percent", 0.0, 1.0));
        log.ingest(&tu("mic", "ok", 2.0, 3.0));
        log
    }

    type Calls = Arc<Mutex<Vec<ClaudeTurn>>>;

    /// Replies with each entry of `replies` in turn, then NONE.
    fn seq_runner(replies: Vec<&'static str>, calls: Calls) -> Runner {
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        Arc::new(move |turn, _on_delta, _register| {
            calls.lock().unwrap().push(turn);
            let text = replies.lock().unwrap().pop_front().unwrap_or("NONE");
            Box::pin(async move {
                ClaudeOutcome { session_id: None, text: text.to_string(), error: None }
            })
        })
    }

    /// Holds every turn until the test adds a permit to `gate`.
    fn gated_runner(text: &'static str, gate: Arc<Semaphore>, calls: Calls) -> Runner {
        Arc::new(move |turn, _on_delta, _register| {
            calls.lock().unwrap().push(turn);
            let gate = gate.clone();
            Box::pin(async move {
                gate.acquire().await.unwrap().forget();
                ClaudeOutcome { session_id: None, text: text.to_string(), error: None }
            })
        })
    }

    struct Recorded {
        hooks: AskHooks,
        reruns: Arc<AtomicUsize>,
        errors: Arc<Mutex<Vec<String>>>,
    }

    fn recorded_hooks() -> Recorded {
        let reruns = Arc::new(AtomicUsize::new(0));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let r = reruns.clone();
        let e = errors.clone();
        Recorded {
            hooks: AskHooks {
                emit: Arc::new(|_| {}),
                rerun: Arc::new(move || {
                    r.fetch_add(1, Ordering::SeqCst);
                }),
                on_error: Arc::new(move |msg| e.lock().unwrap().push(msg)),
            },
            reruns,
            errors,
        }
    }

    fn test_cfg() -> AskConfig {
        AskConfig {
            cwd: std::env::temp_dir(),
            model: "ask-model".to_string(),
            deep_model: "deep-model".to_string(),
            deep_effort: "high".to_string(),
            deep_read_dirs: vec![PathBuf::from("/vault")],
        }
    }

    impl AskLane {
        /// A lane whose deep fork failed: only the quick sub-lane exists.
        fn for_test(runner: Runner, hooks: AskHooks) -> Self {
            let lane = AskLane::new(runner, hooks);
            {
                let mut s = lane.state.lock().unwrap();
                s.sub_mut(Which::Fast).lane_id = Some("ask-1".to_string());
                s.cfg = Some(test_cfg());
            }
            lane
        }
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    #[tokio::test]
    async fn first_request_sends_delta_and_in_progress_text() {
        let calls: Calls = Arc::default();
        let lane = AskLane::for_test(seq_runner(vec![GOOD], calls.clone()), AskHooks::default());
        lane.request(&sample_log(), false);
        settle().await;

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let turn = &calls[0];
        assert_eq!(turn.resume.as_deref(), Some("ask-1"));
        assert!(!turn.fork);
        assert_eq!(turn.effort.as_deref(), Some("medium"));
        assert_eq!(turn.model.as_deref(), Some("ask-model"));
        assert!(turn.prompt.contains("Them: our fee is ten percent"));
        assert!(turn.prompt.contains("STILL SPEAKING"));
        assert!(turn.prompt.contains("You: ok"));
        assert!(turn.prompt.contains("CURRENT LIST:\n(empty)"));

        let out = lane.snapshot();
        assert_eq!(out.state, "idle");
        assert_eq!(out.asks.len(), 1);
        assert_eq!(out.asks[0].question, "How do you screen tenants?");
        assert_eq!(out.for_you.len(), 1);
        assert!(out.updated_at.is_some());
    }

    #[tokio::test]
    async fn requests_during_a_turn_coalesce_into_one_rerun() {
        let calls: Calls = Arc::default();
        let gate = Arc::new(Semaphore::new(0));
        let rec = recorded_hooks();
        let lane = AskLane::for_test(gated_runner(GOOD, gate.clone(), calls.clone()), rec.hooks.clone());
        let log = sample_log();

        lane.request(&log, false);
        settle().await;
        assert_eq!(lane.snapshot().state, "thinking");
        lane.request(&log, false);
        lane.request(&log, true);
        lane.request(&log, false);
        settle().await;
        assert_eq!(calls.lock().unwrap().len(), 1, "requests during a turn must not spawn turns");

        gate.add_permits(1);
        settle().await;
        assert_eq!(rec.reruns.load(Ordering::SeqCst), 1, "three requests coalesce into one rerun");
        assert_eq!(lane.snapshot().state, "idle");
    }

    #[tokio::test]
    async fn automatic_request_with_nothing_new_is_a_no_op_but_force_runs() {
        let calls: Calls = Arc::default();
        let lane = AskLane::for_test(seq_runner(vec![GOOD, GOOD], calls.clone()), AskHooks::default());
        let log = sample_log();
        lane.request(&log, false);
        settle().await;

        lane.request(&log, false);
        settle().await;
        assert_eq!(calls.lock().unwrap().len(), 1);

        lane.request(&log, true);
        settle().await;
        assert_eq!(calls.lock().unwrap().len(), 2);
        assert!(!calls.lock().unwrap()[1].prompt.contains("NEW TRANSCRIPT"));
    }

    #[tokio::test]
    async fn failed_turn_keeps_previous_list_and_reports() {
        let calls: Calls = Arc::default();
        let rec = recorded_hooks();
        let lane = AskLane::for_test(
            seq_runner(vec![GOOD, "I think you should ask about fees."], calls.clone()),
            rec.hooks.clone(),
        );
        let mut log = sample_log();
        lane.request(&log, false);
        settle().await;
        let before = lane.snapshot();

        log.ingest(&tu("system", "we also do maintenance", 4.0, 5.0));
        lane.request(&log, false);
        settle().await;

        let after = lane.snapshot();
        assert_eq!(after.state, "failed");
        assert_eq!(after.asks, before.asks);
        assert_eq!(rec.errors.lock().unwrap().len(), 1);

        // The cursor did not advance, so the same delta is sent again.
        lane.request(&log, false);
        settle().await;
        assert!(calls.lock().unwrap()[2].prompt.contains("You: ok"));
    }

    #[tokio::test]
    async fn brief_edit_is_sent_once() {
        let calls: Calls = Arc::default();
        let lane = AskLane::for_test(seq_runner(vec![GOOD, GOOD], calls.clone()), AskHooks::default());
        let log = sample_log();
        lane.set_brief("Learn their fees and screening".to_string());
        assert_eq!(lane.snapshot().brief, "Learn their fees and screening");

        lane.request(&log, false);
        settle().await;
        lane.request(&log, true);
        settle().await;

        let calls = calls.lock().unwrap();
        assert!(calls[0].prompt.contains("MEETING BRIEF (updated"));
        assert!(calls[0].prompt.contains("Learn their fees and screening"));
        assert!(!calls[1].prompt.contains("MEETING BRIEF"));
    }

    #[tokio::test]
    async fn cleared_brief_tells_the_model_to_infer() {
        let calls: Calls = Arc::default();
        let lane = AskLane::for_test(seq_runner(vec![GOOD], calls.clone()), AskHooks::default());
        lane.set_brief("something".to_string());
        lane.set_brief(String::new());
        lane.request(&sample_log(), false);
        settle().await;
        assert!(calls.lock().unwrap()[0].prompt.contains("Joseph cleared the brief"));
    }

    #[tokio::test]
    async fn brief_edit_during_a_turn_is_sent_on_the_next_turn() {
        let calls: Calls = Arc::default();
        let gate = Arc::new(Semaphore::new(0));
        let lane = AskLane::for_test(gated_runner(GOOD, gate.clone(), calls.clone()), AskHooks::default());
        let log = sample_log();

        lane.request(&log, false);
        settle().await;
        lane.set_brief("Focus on maintenance".to_string());
        gate.add_permits(1);
        settle().await;

        lane.request(&log, false);
        settle().await;
        gate.add_permits(1);
        settle().await;

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "an unsent brief edit counts as news");
        assert!(!calls[0].prompt.contains("Focus on maintenance"));
        assert!(calls[1].prompt.contains("Focus on maintenance"));
    }

    #[tokio::test]
    async fn dismissed_item_is_removed_sent_once_and_never_returns() {
        let calls: Calls = Arc::default();
        let lane = AskLane::for_test(
            seq_runner(
                vec![
                    "ASK: How do you screen tenants? | WHY: risk",
                    "ASK: How do you screen new tenants? | WHY: risk",
                    "NONE",
                ],
                calls.clone(),
            ),
            AskHooks::default(),
        );
        let log = sample_log();
        lane.request(&log, false);
        settle().await;
        let id = lane.snapshot().asks[0].id.clone();

        assert!(lane.dismiss(&id));
        assert!(lane.snapshot().asks.is_empty());
        assert!(!lane.dismiss("no-such-id"));

        lane.request(&log, false);
        settle().await;
        assert!(lane.snapshot().asks.is_empty(), "a reworded dismissed question must not return");

        lane.request(&log, true);
        settle().await;
        let calls = calls.lock().unwrap();
        assert!(calls[1].prompt.contains("Joseph dismissed these"));
        assert!(calls[1].prompt.contains("How do you screen tenants?"));
        assert!(!calls[2].prompt.contains("Joseph dismissed these"));
    }

    #[tokio::test]
    async fn close_drops_a_late_result() {
        let calls: Calls = Arc::default();
        let gate = Arc::new(Semaphore::new(0));
        let rec = recorded_hooks();
        let lane = AskLane::for_test(gated_runner(GOOD, gate.clone(), calls.clone()), rec.hooks.clone());
        let log = sample_log();

        lane.request(&log, false);
        settle().await;
        lane.request(&log, false); // marks dirty
        lane.close();
        gate.add_permits(1);
        settle().await;

        let out = lane.snapshot();
        assert_eq!(out.state, "idle");
        assert!(out.asks.is_empty());
        assert!(out.updated_at.is_none());
        assert_eq!(rec.reruns.load(Ordering::SeqCst), 0);

        lane.request(&log, true);
        settle().await;
        assert_eq!(calls.lock().unwrap().len(), 1, "a closed lane runs nothing");
    }

    #[tokio::test]
    async fn open_without_a_trunk_is_unavailable() {
        let calls: Calls = Arc::default();
        let lane = AskLane::new(seq_runner(vec![], calls.clone()), AskHooks::default());
        lane.open(
            None,
            test_cfg(),
            "brief".to_string(),
        )
        .await;
        let out = lane.snapshot();
        assert_eq!(out.state, "unavailable");
        assert_eq!(out.brief, "brief");
        lane.request(&sample_log(), true);
        settle().await;
        assert!(calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn open_forks_the_trunk_with_the_ask_prompt() {
        let calls: Calls = Arc::default();
        let recorded = calls.clone();
        let runner: Runner = Arc::new(move |turn, _d, _r| {
            recorded.lock().unwrap().push(turn);
            Box::pin(async {
                ClaudeOutcome { session_id: Some("ask-9".to_string()), text: "READY".to_string(), error: None }
            })
        });
        let lane = AskLane::new(runner, AskHooks::default());
        lane.open(
            Some("trunk-1".to_string()),
            test_cfg(),
            String::new(),
        )
        .await;
        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].resume.as_deref(), Some("trunk-1"));
        assert!(calls[0].fork);
        assert_eq!(calls[0].append_system_prompt.as_deref(), Some(ASK_PROMPT));
        assert_eq!(lane.snapshot().state, "idle");
    }

    /// A fake encoder: the image data is the frame's file name.
    fn fake_feed(store: &crate::assistant::vision::FrameStore) -> FrameFeed {
        FrameFeed::with_encoder(
            store.clone(),
            Arc::new(|p: &std::path::Path| {
                Ok(crate::assistant::vision::ImageBlock {
                    media_type: "image/jpeg".to_string(),
                    base64: p.file_name().unwrap().to_string_lossy().to_string(),
                })
            }),
        )
    }

    fn image_names(t: &ClaudeTurn) -> Vec<String> {
        t.images.iter().map(|i| i.base64.clone()).collect()
    }

    #[tokio::test]
    async fn ask_turn_sends_all_frames_since_its_cursor_and_advances_on_success() {
        let store = crate::assistant::vision::FrameStore::new();
        store.push("/f/1".into(), 1);
        store.push("/f/2".into(), 2);
        let calls: Calls = Arc::default();
        let lane = AskLane::for_test(seq_runner(vec![GOOD, GOOD], calls.clone()), AskHooks::default());
        lane.set_frames(Some(fake_feed(&store)));

        lane.request(&sample_log(), true);
        settle().await;
        store.push("/f/3".into(), 3);
        lane.request(&sample_log(), true);
        settle().await;

        let calls = calls.lock().unwrap();
        assert_eq!(image_names(&calls[0]), vec!["1", "2"]);
        assert_eq!(image_names(&calls[1]), vec!["3"]);
    }

    #[tokio::test]
    async fn ask_turn_that_fails_with_images_is_retried_without_them() {
        let store = crate::assistant::vision::FrameStore::new();
        store.push("/f/1".into(), 1);
        let calls: Calls = Arc::default();
        let recorded = calls.clone();
        let runner: Runner = Arc::new(move |turn, _d, _r| {
            let failing = !turn.images.is_empty();
            recorded.lock().unwrap().push(turn);
            Box::pin(async move {
                if failing {
                    ClaudeOutcome { session_id: None, text: String::new(), error: Some("image rejected".into()) }
                } else {
                    ClaudeOutcome { session_id: None, text: GOOD.to_string(), error: None }
                }
            })
        });
        let lane = AskLane::for_test(runner, AskHooks::default());
        lane.set_frames(Some(fake_feed(&store)));
        lane.request(&sample_log(), true);
        settle().await;

        {
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert_eq!(image_names(&calls[0]), vec!["1"]);
            assert!(calls[1].images.is_empty());
            assert_eq!(calls[0].prompt, calls[1].prompt);
        }
        assert_eq!(lane.snapshot().asks.len(), 1, "the retry's answer is used");
        lane.request(&sample_log(), true);
        settle().await;
        assert!(calls.lock().unwrap()[2].images.is_empty(), "the frame that broke a turn is not resent");
    }

    // ---- fast then better: two sub-lanes ----

    const QUICK: &str = "ASK: What storage do you run today? | WHY: storage platform unknown";
    const CHECKED: &str = "ASK: What storage do you run today? | WHY: storage platform unknown\nASK: When does your support contract end? | WHY: renewal date vague";

    /// Replies per model: the fast model gets `fast`, the deep one `deep`.
    /// Either can be held behind a gate. Calls are recorded in order.
    fn two_runner(
        fast: &'static str,
        deep: &'static str,
        fast_gate: Option<Arc<Semaphore>>,
        deep_gate: Option<Arc<Semaphore>>,
        calls: Calls,
    ) -> Runner {
        Arc::new(move |turn, _on_delta, _register| {
            let is_deep = turn.model.as_deref() == Some("deep-model");
            calls.lock().unwrap().push(turn);
            let (text, gate) = if is_deep { (deep, deep_gate.clone()) } else { (fast, fast_gate.clone()) };
            Box::pin(async move {
                if let Some(gate) = gate {
                    gate.acquire().await.unwrap().forget();
                }
                ClaudeOutcome { session_id: None, text: text.to_string(), error: None }
            })
        })
    }

    fn model_calls(calls: &Calls, model: &str) -> usize {
        calls.lock().unwrap().iter().filter(|t| t.model.as_deref() == Some(model)).count()
    }

    impl AskLane {
        /// Both sub-lanes forked.
        fn for_test_two(runner: Runner, hooks: AskHooks) -> Self {
            let lane = AskLane::for_test(runner, hooks);
            lane.state.lock().unwrap().sub_mut(Which::Deep).lane_id = Some("ask-deep-1".to_string());
            lane
        }
    }

    #[tokio::test]
    async fn quick_list_shows_first_then_the_checked_list_replaces_it() {
        let calls: Calls = Arc::default();
        let deep_gate = Arc::new(Semaphore::new(0));
        let lane = AskLane::for_test_two(
            two_runner(QUICK, CHECKED, None, Some(deep_gate.clone()), calls.clone()),
            AskHooks::default(),
        );
        lane.request(&sample_log(), false);
        settle().await;

        let quick = lane.snapshot();
        assert_eq!(quick.asks.len(), 1);
        assert!(!quick.checked);
        assert!(quick.checking, "the deep sub-lane is still working");
        assert_eq!(quick.state, "thinking");
        assert!(quick.changed.is_empty());
        let first_id = quick.asks[0].id.clone();

        deep_gate.add_permits(1);
        settle().await;
        let out = lane.snapshot();
        assert_eq!(out.state, "idle");
        assert!(out.checked);
        assert!(!out.checking);
        assert_eq!(out.asks.len(), 2);
        assert_eq!(out.asks[0].question, "What storage do you run today?");
        assert_eq!(out.asks[0].id, first_id, "a question worded the same keeps its id");
        assert_eq!(out.changed, vec![out.asks[1].id.clone()], "only the new question is marked changed");
        assert_eq!(out.asks[1].question, "When does your support contract end?");
    }

    #[tokio::test]
    async fn a_reworded_question_in_the_checked_list_is_marked_changed() {
        let calls: Calls = Arc::default();
        let deep_gate = Arc::new(Semaphore::new(0));
        let lane = AskLane::for_test_two(
            two_runner(
                QUICK,
                "ASK: Which storage platform do you run today? | WHY: platform unknown",
                None,
                Some(deep_gate.clone()),
                calls,
            ),
            AskHooks::default(),
        );
        lane.request(&sample_log(), false);
        settle().await;
        let id = lane.snapshot().asks[0].id.clone();
        deep_gate.add_permits(1);
        settle().await;
        let out = lane.snapshot();
        assert_eq!(out.asks[0].id, id);
        assert_eq!(out.asks[0].question, "Which storage platform do you run today?");
        assert_eq!(out.changed, vec![id]);
    }

    #[tokio::test]
    async fn quick_result_after_the_checked_one_at_the_same_point_is_dropped() {
        let calls: Calls = Arc::default();
        let fast_gate = Arc::new(Semaphore::new(0));
        let lane = AskLane::for_test_two(
            two_runner(QUICK, CHECKED, Some(fast_gate.clone()), None, calls),
            AskHooks::default(),
        );
        lane.request(&sample_log(), false);
        settle().await;
        assert!(lane.snapshot().checked);
        assert_eq!(lane.snapshot().asks.len(), 2);

        fast_gate.add_permits(1);
        settle().await;
        let out = lane.snapshot();
        assert_eq!(out.state, "idle", "the dropped turn still finished");
        assert!(out.checked);
        assert_eq!(out.asks.len(), 2, "the deep list already covers that much transcript");
    }

    #[tokio::test]
    async fn checked_list_built_earlier_applies_then_one_quick_rerun_lays_the_news_over_it() {
        let calls: Calls = Arc::default();
        let deep_gate = Arc::new(Semaphore::new(0));
        let rec = recorded_hooks();
        let lane = AskLane::for_test_two(
            two_runner(
                "ASK: Who handles backups? | WHY: owner unknown",
                CHECKED,
                None,
                Some(deep_gate.clone()),
                calls.clone(),
            ),
            rec.hooks.clone(),
        );
        let mut log = sample_log();
        lane.request(&log, false); // both fire at point 1; deep is held
        settle().await;
        assert_eq!(lane.snapshot().asks[0].question, "Who handles backups?");

        log.ingest(&tu("system", "someone else handles backups", 4.0, 5.0));
        log.ingest(&tu("mic", "right", 6.0, 7.0));
        lane.request(&log, false); // fast fires at point 2; deep is busy
        settle().await;
        assert_eq!(model_calls(&calls, "ask-model"), 2);

        deep_gate.add_permits(1); // deep lands, built at point 1
        settle().await;
        let out = lane.snapshot();
        assert!(out.checked, "a deep result always applies");
        assert_eq!(out.asks.len(), 2);
        assert_eq!(rec.reruns.load(Ordering::SeqCst), 1);
        assert_eq!(model_calls(&calls, "ask-model"), 2, "the rerun waits for core to call rerun");

        lane.rerun(&log); // deep holds a follow-up of its own; hold it
        settle().await;
        assert_eq!(model_calls(&calls, "ask-model"), 3, "exactly one more quick call");
        let after = lane.snapshot();
        assert_eq!(after.asks[0].question, "Who handles backups?", "the quick lane re-applied what it heard");
        assert!(!after.checked);
    }

    #[tokio::test]
    async fn a_failed_deep_fork_leaves_the_quick_lane_as_it_was() {
        let calls: Calls = Arc::default();
        let recorded = calls.clone();
        let runner: Runner = Arc::new(move |turn, _d, _r| {
            let is_deep = turn.model.as_deref() == Some("deep-model");
            let forking = turn.fork;
            recorded.lock().unwrap().push(turn);
            Box::pin(async move {
                if is_deep && forking {
                    ClaudeOutcome { session_id: None, text: String::new(), error: Some("no fork".into()) }
                } else if forking {
                    ClaudeOutcome { session_id: Some("ask-9".into()), text: "READY".into(), error: None }
                } else {
                    ClaudeOutcome { session_id: None, text: GOOD.to_string(), error: None }
                }
            })
        });
        let lane = AskLane::new(runner, AskHooks::default());
        lane.open(Some("trunk-1".to_string()), test_cfg(), String::new()).await;
        assert_eq!(lane.snapshot().state, "idle");

        let log = sample_log();
        lane.request(&log, false);
        settle().await;
        assert_eq!(model_calls(&calls, "deep-model"), 1, "only the failed fork ever ran on the deep model");
        let out = lane.snapshot();
        assert_eq!(out.asks.len(), 1);
        assert!(!out.checked && !out.checking);
        assert_eq!(out.state, "idle");

        lane.request(&log, true);
        settle().await;
        assert_eq!(model_calls(&calls, "ask-model"), 3, "fork, turn, forced turn");
    }

    #[tokio::test]
    async fn dismissing_while_the_deep_turn_runs_keeps_the_question_out_of_its_result() {
        let calls: Calls = Arc::default();
        let deep_gate = Arc::new(Semaphore::new(0));
        let lane = AskLane::for_test_two(
            two_runner(QUICK, CHECKED, None, Some(deep_gate.clone()), calls),
            AskHooks::default(),
        );
        lane.request(&sample_log(), false);
        settle().await;
        let id = lane.snapshot().asks[0].id.clone();
        assert!(lane.dismiss(&id));

        deep_gate.add_permits(1);
        settle().await;
        let out = lane.snapshot();
        assert!(out.checked);
        let questions: Vec<_> = out.asks.iter().map(|a| a.question.as_str()).collect();
        assert_eq!(questions, vec!["When does your support contract end?"]);
    }

    #[tokio::test]
    async fn each_sub_lane_hears_a_brief_edit_and_a_dismissal_once() {
        let calls: Calls = Arc::default();
        let lane = AskLane::for_test_two(two_runner(QUICK, CHECKED, None, None, calls.clone()), AskHooks::default());
        let log = sample_log();
        lane.request(&log, false);
        settle().await;
        let id = lane.snapshot().asks[0].id.clone();
        lane.set_brief("Learn the storage".to_string());
        lane.dismiss(&id);
        lane.request(&log, false);
        settle().await;
        lane.request(&log, true);
        settle().await;

        let calls = calls.lock().unwrap();
        for model in ["ask-model", "deep-model"] {
            let turns: Vec<_> = calls.iter().filter(|t| t.model.as_deref() == Some(model)).collect();
            assert_eq!(turns.len(), 3, "{model}");
            assert!(turns[1].prompt.contains("Learn the storage"), "{model}");
            assert!(turns[1].prompt.contains("Joseph dismissed these"), "{model}");
            assert!(!turns[2].prompt.contains("Learn the storage"), "{model}");
            assert!(!turns[2].prompt.contains("Joseph dismissed these"), "{model}");
        }
    }

    #[tokio::test]
    async fn a_deep_failure_with_a_list_on_screen_only_reports() {
        let calls: Calls = Arc::default();
        let recorded = calls.clone();
        let rec = recorded_hooks();
        let runner: Runner = Arc::new(move |turn, _d, _r| {
            let is_deep = turn.model.as_deref() == Some("deep-model");
            recorded.lock().unwrap().push(turn);
            Box::pin(async move {
                if is_deep {
                    ClaudeOutcome { session_id: None, text: String::new(), error: Some("deep broke".into()) }
                } else {
                    ClaudeOutcome { session_id: None, text: QUICK.to_string(), error: None }
                }
            })
        });
        let lane = AskLane::for_test_two(runner, rec.hooks.clone());
        lane.request(&sample_log(), false);
        settle().await;
        let out = lane.snapshot();
        assert_eq!(out.state, "idle", "the quick list stands, nothing shows as failed");
        assert_eq!(out.asks.len(), 1);
        assert!(!out.checked && !out.checking);
        assert_eq!(rec.errors.lock().unwrap().as_slice(), ["deep broke"]);
    }

    #[tokio::test]
    async fn close_drops_a_late_deep_result() {
        let calls: Calls = Arc::default();
        let deep_gate = Arc::new(Semaphore::new(0));
        let lane = AskLane::for_test_two(
            two_runner(QUICK, CHECKED, None, Some(deep_gate.clone()), calls),
            AskHooks::default(),
        );
        lane.request(&sample_log(), false);
        settle().await;
        lane.close();
        deep_gate.add_permits(1);
        settle().await;
        let out = lane.snapshot();
        assert!(!out.checked);
        assert_eq!(out.asks.len(), 1, "the quick list stays, the deep one never lands");
        assert_eq!(out.state, "idle");
    }

    #[tokio::test]
    async fn the_deep_sub_lane_reads_the_vault_and_never_writes() {
        let calls: Calls = Arc::default();
        // Fork turns answer with a session id.
        let recorded = calls.clone();
        let runner: Runner = Arc::new(move |turn, _d, _r| {
            let forking = turn.fork;
            recorded.lock().unwrap().push(turn);
            Box::pin(async move {
                ClaudeOutcome {
                    session_id: forking.then(|| "s".to_string()),
                    text: if forking { "READY".into() } else { QUICK.into() },
                    error: None,
                }
            })
        });
        let lane = AskLane::new(runner, AskHooks::default());
        lane.open(Some("trunk-1".to_string()), test_cfg(), String::new()).await;
        lane.request(&sample_log(), false);
        settle().await;

        let calls = calls.lock().unwrap();
        let deep: Vec<_> = calls.iter().filter(|t| t.model.as_deref() == Some("deep-model")).collect();
        assert_eq!(deep.len(), 2, "fork and one turn");
        let (fork, turn) = (deep[0], deep[1]);
        assert!(fork.fork && fork.allowed_tools.is_empty() && fork.add_dirs.is_empty());
        assert_eq!(turn.effort.as_deref(), Some("high"));
        assert_eq!(turn.append_system_prompt.as_deref(), Some(ask_deep_system_prompt().as_str()));
        assert!(turn.append_system_prompt.as_deref().unwrap().starts_with(ASK_PROMPT));
        assert_eq!(turn.allowed_tools, vec!["Read", "Grep", "Glob"]);
        assert!(turn.disallowed_tools.iter().any(|t| t == "Bash") && turn.disallowed_tools.iter().any(|t| t == "Write"));
        assert_eq!(turn.add_dirs, vec![PathBuf::from("/vault")]);
        assert_eq!(turn.resume.as_deref(), Some("s"));

        let fast: Vec<_> = calls.iter().filter(|t| t.model.as_deref() == Some("ask-model")).collect();
        assert_eq!(fast[1].append_system_prompt.as_deref(), Some(ASK_PROMPT));
        assert!(fast[1].allowed_tools.is_empty() && fast[1].add_dirs.is_empty());
        assert_eq!(fast[1].effort.as_deref(), Some("medium"));
    }

    /// Real claude: a quick list first, then the deep sub-lane's checked
    /// list. Spends real turns, so it needs ASSISTANT_CLAUDE_TEST=1 on top
    /// of --ignored. Prints every emitted list with its time since request.
    #[tokio::test]
    #[ignore]
    async fn live_quick_list_then_checked_list() {
        if std::env::var("ASSISTANT_CLAUDE_TEST").ok().as_deref() != Some("1") {
            eprintln!("skipping live_quick_list_then_checked_list: set ASSISTANT_CLAUDE_TEST=1 to run");
            return;
        }
        let bin = crate::assistant::settings::resolve_claude_binary(&crate::assistant::AssistantSettings::default())
            .expect("claude binary not found");
        let runner = crate::assistant::lanes::claude_runner(bin);
        let cwd = std::env::temp_dir().join(format!("ask-live-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&cwd).unwrap();
        let brief = "Learn what storage and backup setup the client runs today, for a migration quote.";

        // Seed a trunk the way lanes.rs does.
        let trunk_id = uuid::Uuid::new_v4().to_string();
        let seed = runner.clone()(
            ClaudeTurn {
                prompt: format!(
                    "This is the brief for the meeting that is about to start. Read it, hold it, and answer nothing yet. Reply with exactly the word READY.\n\n{brief}"
                ),
                cwd: cwd.clone(),
                model: Some("claude-sonnet-5".into()),
                effort: Some("low".into()),
                session_id: Some(trunk_id.clone()),
                disallowed_tools: no_tools(),
                partial: false,
                safe_mode: true,
                ..Default::default()
            },
            Box::new(|_| {}),
            Box::new(|_| {}),
        )
        .await;
        assert!(seed.error.is_none(), "trunk seed: {:?}", seed.error);

        let emitted: Arc<Mutex<Vec<(std::time::Instant, AsksOut)>>> = Arc::default();
        let sink = emitted.clone();
        let hooks = AskHooks {
            emit: Arc::new(move |out| sink.lock().unwrap().push((std::time::Instant::now(), out))),
            ..Default::default()
        };
        let lane = AskLane::new(runner, hooks);
        let home = PathBuf::from(std::env::var("HOME").expect("HOME"));
        lane.open(
            Some(seed.session_id.unwrap_or(trunk_id)),
            AskConfig {
                cwd: cwd.clone(),
                model: "claude-sonnet-5".into(),
                deep_model: "claude-opus-5".into(),
                deep_effort: "medium".into(),
                deep_read_dirs: vec![home.join("brain")],
            },
            brief.to_string(),
        )
        .await;
        assert_eq!(lane.snapshot().state, "idle", "both forks must open");

        let mut log = TranscriptLog::default();
        log.ingest(&tu(
            "system",
            "We have some sort of SAN, I think, for the main storage. Backups, I think someone else handles that.",
            0.0,
            8.0,
        ));
        log.ingest(&tu("mic", "Okay, got it.", 9.0, 10.0));
        log.ingest(&tu("system", "Yeah, the migration is planned for next quarter.", 11.0, 14.0));

        let start = std::time::Instant::now();
        let n_before = emitted.lock().unwrap().len();
        lane.request(&log, false);
        let deadline = start + Duration::from_secs(300);
        while std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let out = lane.snapshot();
            if out.checked && out.state == "idle" {
                break;
            }
        }
        let mut seen_quick = false;
        for (at, out) in emitted.lock().unwrap().iter().skip(n_before) {
            let t = at.duration_since(start).as_secs_f64();
            let shape = format!("state={} checked={} checking={} changed={}", out.state, out.checked, out.checking, out.changed.len());
            if out.asks.is_empty() && out.for_you.is_empty() {
                println!("[{t:6.1}s] {shape}  (no list)");
                continue;
            }
            if !out.checked {
                seen_quick = true;
            }
            println!("[{t:6.1}s] {shape}");
            for a in &out.asks {
                println!("          ASK: {} | WHY: {}", a.question, a.why);
            }
            for y in &out.for_you {
                println!("          YOU: {}", y.question);
            }
        }
        let _ = std::fs::remove_dir_all(&cwd);
        let out = lane.snapshot();
        assert!(seen_quick, "a quick list (checked false) must show first");
        assert!(out.checked, "the deep list must land");
        lane.close();
    }
}
