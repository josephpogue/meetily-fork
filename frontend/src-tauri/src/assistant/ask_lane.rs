// assistant/ask_lane.rs
//
// Ask Next: claude lineages forked from the same trunk as the answer lanes.
// Like the answer cards, the list comes in two passes: a quick sub-lane
// (the fast model, no tools) shows a list first, and a deep sub-lane (the
// deep model, read-only tools over Joseph's vault) later replaces or
// corrects it. Each turn sends what changed since that sub-lane's last one
// (transcript delta, a brief edit, dismissals, newly answered questions, a
// private Ask exchange) and gets back the whole list of questions Joseph
// should still ask, plus a DONE line for each question that was answered.
// An answered question is kept with its answer, never deleted. At most one turn per sub-lane runs
// at a time. Requests that land meanwhile coalesce into one follow-up,
// fired through `AskHooks::rerun`, because only core holds the transcript.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use super::ask_list::{self, AnswerHow, AnsweredItem, AskItem, AskList, RawList};
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
    /// Every question answered this session, newest first. Never shrinks.
    pub answered: Vec<AnsweredItem>,
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

/// One private exchange Joseph had with the assistant in Ask (an Ask card),
/// for the Ask next sub-lanes to hear once: the answer may settle a question.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatExchange {
    pub question: String,
    /// The card's lead and bullets, as text.
    pub answer: String,
}

/// Shared with core, whose card hook appends to it; each session's lane reads it.
pub type ChatLog = Arc<Mutex<Vec<ChatExchange>>>;

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
    /// How many of `AskState::answered` (oldest first) the model has been told about.
    answered_sent: usize,
    /// How many of the chat log's exchanges the model has heard.
    chats_sent: usize,
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
    /// Every question answered this session, oldest first (the snapshot shows
    /// newest first). Only a new session, which makes a new lane, resets it.
    answered: Vec<AnsweredItem>,
    /// Private Ask exchanges; the card hook in core appends.
    chats: ChatLog,
    /// Bumped by `close`; a turn that finishes under an older generation is
    /// dropped on the floor.
    generation: u64,
    live_runs: Vec<TurnHandle>,
    /// Zoom window frames, when the assistant can see the window.
    frames: Option<FrameFeed>,
    /// Transcript point the shown list covers, and who wrote it.
    shown_point: usize,
    shown_by: Shown,
    /// The list the running (or last) deep turn was given as CURRENT LIST.
    /// Deep runs one turn at a time, so one slot is enough.
    deep_base: AskList,
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
    sent_answered: usize,
    sent_chats: usize,
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
        // The state follows the quick sub-lane; deep work shows only through
        // `checking`, unless nothing has been shown yet.
        let state = if self.unavailable {
            "unavailable"
        } else if self.sub(Which::Fast).in_flight {
            "thinking"
        } else if self.failed {
            "failed"
        } else if self.shown_by == Shown::None && self.sub(Which::Deep).in_flight {
            "thinking"
        } else {
            "idle"
        };
        AsksOut {
            state: state.to_string(),
            asks: self.list.asks.clone(),
            for_you: self.list.for_you.clone(),
            answered: self.answered.iter().rev().cloned().collect(),
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
        let unsent_answered = &self.answered[sub.answered_sent.min(self.answered.len())..];
        let (unsent_chats, chats_len) = {
            let chats = self.chats.lock().unwrap_or_else(|e| e.into_inner());
            (chats[sub.chats_sent.min(chats.len())..].to_vec(), chats.len())
        };
        let has_news = !delta.is_empty()
            || brief_changed
            || !unsent.is_empty()
            || !unsent_answered.is_empty()
            || !unsent_chats.is_empty();
        if !force && !has_news {
            return None;
        }
        let prompt = ask_prompt(
            &delta,
            &in_progress,
            brief_changed.then(|| self.brief.as_str()),
            &Unsent { dismissed: unsent, answered: unsent_answered, chats: &unsent_chats },
            &self.list,
        );
        let sent_dismissed = self.dismissed.len();
        let sent_answered = self.answered.len();
        let sent_version = self.brief_version;
        let (batch, new_frame_cursor) = match self.frames.clone() {
            Some(feed) => feed.select(sub.frame_cursor, FrameMode::All),
            None => (FrameBatch::default(), sub.frame_cursor),
        };
        let turn = lane_turn(&cfg, which, lane_id, prompt, false);
        let generation = self.generation;
        if which == Which::Deep {
            self.deep_base = self.list.clone();
        }
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
            sent_answered,
            sent_chats: chats_len,
            new_cursor,
        })
    }

    /// Takes in a finished turn. The sub-lane's own cursors advance on any
    /// readable reply, because its model has seen that transcript; whether
    /// the list changes is a separate question.
    fn land(&mut self, job: &Job, parsed: Option<RawList>, error: Option<String>) -> Landed {
        let which = job.which;
        let point = job.new_cursor;
        let sub = self.sub_mut(which);
        sub.in_flight = false;
        let error = match parsed {
            Some(raw) => {
                sub.cursor = job.new_cursor;
                sub.frame_cursor = job.new_frame_cursor;
                sub.brief_sent = job.sent_version;
                sub.dismissed_sent = job.sent_dismissed;
                sub.answered_sent = job.sent_answered;
                sub.chats_sent = job.sent_chats;
                let now = chrono::Utc::now().timestamp_millis();
                match which {
                    Which::Fast => {
                        let applies = point > self.shown_point
                            || (point == self.shown_point && self.shown_by != Shown::Deep);
                        if applies {
                            self.list = ask_list::build(&self.list, &mut self.answered, raw, &self.dismissed, now);
                            self.changed.clear();
                            self.shown_by = Shown::Fast;
                            self.shown_point = point;
                            self.updated_at = Some(now);
                            self.failed = false;
                        } else {
                            // The quick list is stale, but its model has seen
                            // this transcript and will not say DONE again, so
                            // what it marked answered is kept.
                            let before = self.answered.len();
                            ask_list::absorb_done(&self.list, &mut self.answered, raw.done, now);
                            if self.answered.len() != before {
                                self.list = ask_list::without_answered(&self.list, &self.answered);
                            }
                            log::debug!(
                                "assistant ask: quick list at point {} dropped, the checked list covers point {}",
                                point,
                                self.shown_point
                            );
                        }
                    }
                    Which::Deep => {
                        // A deep list built from less transcript than the
                        // shown quick list keeps the quick list's newer items.
                        let raw = if point < self.shown_point && !ask_list::done_only(&raw) {
                            merge_raw(&self.list, &self.deep_base, raw, &self.dismissed, &self.answered)
                        } else {
                            raw
                        };
                        let next = ask_list::build(&self.list, &mut self.answered, raw, &self.dismissed, now);
                        self.changed = changed_ids(&self.list, &next);
                        self.list = next;
                        self.shown_by = Shown::Deep;
                        self.shown_point = self.shown_point.max(point);
                        self.updated_at = Some(now);
                        self.failed = false;
                    }
                }
                None
            }
            None => {
                // A deep failure only reports once some result has been shown.
                if which == Which::Fast || self.shown_by == Shown::None {
                    self.failed = true;
                }
                Some(error.unwrap_or_else(|| "Ask next could not read the model's reply.".to_string()))
            }
        };
        let rerun = self.sub(which).dirty;
        Landed { out: self.snapshot(), rerun, error }
    }
}

/// A deep list that was built from less transcript than the shown quick list:
/// deep's items in deep's order, then each shown item that deep neither kept
/// nor was given (it came from transcript deep never heard). Items deep was
/// given and dropped stay dropped. Dismissed questions are removed before the
/// cap, so they cannot crowd out a newer item. Capped like any list, deep's
/// first. A question that is answered (already, or by a DONE in `raw`) never
/// comes back as open, and is removed before the cap too.
fn merge_raw(shown: &AskList, base: &AskList, mut raw: RawList, dismissed: &[String], answered: &[AnsweredItem]) -> RawList {
    let done_now: Vec<String> = raw.done.iter().map(|d| d.0.clone()).collect();
    let gone = |q: &str| {
        dismissed.iter().any(|d| ask_list::same_item(d, q))
            || answered.iter().any(|a| ask_list::same_item(&a.question, q))
            || done_now.iter().any(|d| ask_list::same_item(d, q))
    };
    raw.asks.retain(|(q, _)| !gone(q));
    raw.for_you.retain(|q| !gone(q));
    let newer = |items: &[AskItem], base: &[AskItem], have: &dyn Fn(&str) -> bool| -> Vec<String> {
        items
            .iter()
            .filter(|i| !gone(&i.question) && !have(&i.question) && !base.iter().any(|b| ask_list::same_item(&b.question, &i.question)))
            .map(|i| i.question.clone())
            .collect()
    };
    let ask_extra: Vec<(String, String)> = {
        let have = |q: &str| raw.asks.iter().any(|(d, _)| ask_list::same_item(d, q));
        let qs = newer(&shown.asks, &base.asks, &have);
        shown
            .asks
            .iter()
            .filter(|i| qs.contains(&i.question))
            .map(|i| (i.question.clone(), i.why.clone()))
            .collect()
    };
    let you_extra = {
        let have = |q: &str| raw.for_you.iter().any(|d| ask_list::same_item(d, q));
        newer(&shown.for_you, &base.for_you, &have)
    };
    raw.asks.extend(ask_extra);
    raw.asks.truncate(ask_list::MAX_ASKS);
    raw.for_you.extend(you_extra);
    raw.for_you.truncate(ask_list::MAX_FOR_YOU);
    raw
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

/// Held by a spawned turn task until its result lands. If the task dies first
/// the sub-lane would stay "in flight" for good, so the drop clears it.
struct InFlightGuard {
    state: Arc<Mutex<AskState>>,
    hooks: AskHooks,
    which: Which,
    generation: u64,
    armed: bool,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let (out, dirty) = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.generation != self.generation {
                return;
            }
            let sub = s.sub_mut(self.which);
            sub.in_flight = false;
            let dirty = sub.dirty;
            (s.snapshot(), dirty)
        };
        log::warn!("assistant ask {:?} turn ended without a result", self.which);
        (self.hooks.emit)(out);
        if dirty {
            (self.hooks.rerun)();
        }
    }
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
            // Clears in_flight if this task panics or is dropped before `land`.
            let mut guard = InFlightGuard {
                state: state.clone(),
                hooks: hooks.clone(),
                which: job.which,
                generation: job.generation,
                armed: true,
            };
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
                guard.armed = false;
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

    /// Joseph typed the answer to a list question: the item with that id moves
    /// to the answered list ("you"). False when no item has that id or the
    /// answer is empty. The brief is not touched.
    pub fn answer(&self, id: &str, answer: &str) -> bool {
        let answer = answer.trim();
        if answer.is_empty() {
            return false;
        }
        let out = {
            let mut s = self.state.lock().unwrap();
            let removed = if let Some(pos) = s.list.asks.iter().position(|i| i.id == id) {
                s.list.asks.remove(pos)
            } else if let Some(pos) = s.list.for_you.iter().position(|i| i.id == id) {
                s.list.for_you.remove(pos)
            } else {
                return false;
            };
            s.answered.push(AnsweredItem {
                id: removed.id,
                question: removed.question,
                answer: ask_list::collapse(answer),
                how: AnswerHow::You,
                at: chrono::Utc::now().timestamp_millis(),
            });
            s.snapshot()
        };
        (self.hooks.emit)(out);
        true
    }

    /// Where core's card hook records Ask exchanges for this session's lane.
    pub fn set_chat_log(&self, log: ChatLog) {
        self.state.lock().unwrap().chats = log;
    }

    /// Every answered question, oldest first, for the end-of-meeting note.
    /// Stays readable after `close`.
    pub fn answered_items(&self) -> Vec<AnsweredItem> {
        self.state.lock().unwrap().answered.clone()
    }

    /// Kills in-flight turns and stops all further turns. The last list
    /// stays visible until the next session opens a fresh lane.
    pub fn close(&self) {
        let out = self.stop_turns();
        (self.hooks.emit)(out);
    }

    /// `close` without the snapshot emit, for a lane that is being replaced:
    /// its list must not be announced again, because the history collector
    /// records every emit and the replacement belongs to a newer meeting.
    pub fn discard(&self) {
        self.stop_turns();
    }

    fn stop_turns(&self) -> AsksOut {
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
        out
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

/// What a sub-lane has not been told yet, besides the transcript and brief.
struct Unsent<'a> {
    dismissed: &'a [String],
    answered: &'a [AnsweredItem],
    chats: &'a [ChatExchange],
}

fn ask_prompt(
    delta: &str,
    in_progress: &str,
    brief: Option<&str>,
    unsent: &Unsent,
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
    if !unsent.dismissed.is_empty() {
        out.push_str("Joseph dismissed these. Never suggest them again:\n");
        for q in unsent.dismissed {
            out.push_str("- ");
            out.push_str(q);
            out.push('\n');
        }
        out.push('\n');
    }
    if !unsent.answered.is_empty() {
        out.push_str("ANSWERED SO FAR (never ask these again):\n");
        for a in unsent.answered {
            out.push_str(&format!("- {} Answer: {}\n", a.question, a.answer));
        }
        out.push('\n');
    }
    if !unsent.chats.is_empty() {
        out.push_str("JOSEPH ASKED THE ASSISTANT PRIVATELY:\n");
        for c in unsent.chats {
            out.push_str(&format!("Q: {}\nA: {}\n", c.question, c.answer));
        }
        out.push_str("If that exchange answers a question on CURRENT LIST, mark it DONE with BY: chat.\n\n");
    }
    out.push_str("CURRENT LIST:\n");
    out.push_str(&ask_list::render(current));
    out.push_str("\n\nReply with the full updated list in the ASK / YOU format, plus a DONE line for each question answered since your last turn, or NONE.");
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
        assert_eq!(quick.state, "idle", "deep work shows only through checking");
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

    const Q_A: &str = "ASK: What storage do you run today? | WHY: storage platform unknown";

    /// Per-model reply queues (then NONE). The deep model's first turn passes
    /// the gate; every later one waits for a permit from the test.
    fn scripted(
        fast: Vec<&'static str>,
        deep: Vec<&'static str>,
        deep_gate: Arc<Semaphore>,
        calls: Calls,
    ) -> Runner {
        let fast = Arc::new(Mutex::new(VecDeque::from(fast)));
        let deep = Arc::new(Mutex::new(VecDeque::from(deep)));
        Arc::new(move |turn, _on_delta, _register| {
            let is_deep = turn.model.as_deref() == Some("deep-model");
            calls.lock().unwrap().push(turn);
            let queue = if is_deep { deep.clone() } else { fast.clone() };
            let text = queue.lock().unwrap().pop_front().unwrap_or("NONE");
            let gate = is_deep.then(|| deep_gate.clone());
            Box::pin(async move {
                if let Some(gate) = gate {
                    gate.acquire().await.unwrap().forget();
                }
                ClaudeOutcome { session_id: None, text: text.to_string(), error: None }
            })
        })
    }

    /// Drives a lane to: shown list from the quick lane at point 2, a deep
    /// turn built at point 1 (given the list shown then) still running.
    /// Returns the lane, the gate holding the deep turn and the call log.
    async fn stale_deep_setup(
        fast: Vec<&'static str>,
        deep: Vec<&'static str>,
    ) -> (AskLane, Arc<Semaphore>, Calls) {
        let calls: Calls = Arc::default();
        let gate = Arc::new(Semaphore::new(1));
        let lane = AskLane::for_test_two(scripted(fast, deep, gate.clone(), calls.clone()), AskHooks::default());
        let mut log = sample_log();
        lane.request(&log, false); // both fire at point 1; deep's first turn passes
        settle().await;
        lane.request(&log, true); // deep fires at point 1 again, held; quick reply is dropped
        settle().await;
        log.ingest(&tu("system", "someone else handles backups", 4.0, 5.0));
        log.ingest(&tu("mic", "right", 6.0, 7.0));
        lane.request(&log, false); // quick fires at point 2; deep is busy
        settle().await;
        (lane, gate, calls)
    }

    #[tokio::test]
    async fn a_stale_checked_list_is_merged_with_the_quick_items_it_never_heard() {
        let ab = "ASK: What storage do you run today? | WHY: storage platform unknown\nASK: Who handles backups? | WHY: owner unknown";
        let a2c = "ASK: Which storage platform do you run today? | WHY: platform unknown\nASK: When does your support contract end? | WHY: renewal date vague";
        // Deep's first turn sees [A]; the quick lane later adds B at point 2.
        let (lane, gate, calls) = stale_deep_setup(vec![Q_A, Q_A, ab], vec![Q_A, a2c]).await;
        let shown = lane.snapshot();
        assert!(!shown.checked, "the quick list is on screen");
        assert_eq!(shown.asks.len(), 2);
        let (id_a, id_b) = (shown.asks[0].id.clone(), shown.asks[1].id.clone());
        let fast_before = model_calls(&calls, "ask-model");

        gate.add_permits(1); // deep lands, built at point 1
        settle().await;
        let out = lane.snapshot();
        let questions: Vec<_> = out.asks.iter().map(|a| a.question.as_str()).collect();
        assert_eq!(
            questions,
            vec![
                "Which storage platform do you run today?",
                "When does your support contract end?",
                "Who handles backups?"
            ]
        );
        assert!(out.checked);
        assert_eq!(out.asks[0].id, id_a, "the reworded question keeps its id");
        assert_eq!(out.asks[2].id, id_b);
        assert_eq!(out.changed, vec![id_a, out.asks[1].id.clone()]);
        assert_eq!(model_calls(&calls, "ask-model"), fast_before, "no forced quick rerun");
    }

    #[tokio::test]
    async fn a_stale_checked_list_keeps_dropping_what_it_was_given_and_dropped() {
        let ab = "ASK: What storage do you run today? | WHY: storage platform unknown\nASK: Who handles backups? | WHY: owner unknown";
        let (lane, gate, _calls) = stale_deep_setup(vec![ab, ab, ab], vec![ab, Q_A]).await;
        assert_eq!(lane.snapshot().asks.len(), 2);
        gate.add_permits(1);
        settle().await;
        let out = lane.snapshot();
        let questions: Vec<_> = out.asks.iter().map(|a| a.question.as_str()).collect();
        assert_eq!(questions, vec!["What storage do you run today?"], "deep judged B and dropped it");
        assert!(out.checked);
    }

    #[tokio::test]
    async fn a_stale_checked_list_cuts_the_newer_quick_item_at_the_cap() {
        let an = "ASK: What storage do you run today? | WHY: storage platform unknown\nASK: Do you use encryption at rest? | WHY: security unknown";
        let five = "ASK: What is your total headcount? | WHY: size\nASK: Which cloud provider hosts production? | WHY: hosting\nASK: How often do backups run? | WHY: rpo\nASK: Who approves capital purchases? | WHY: buyer\nASK: When does the lease expire? | WHY: timing";
        let (lane, gate, _calls) = stale_deep_setup(vec![Q_A, Q_A, an], vec![Q_A, five]).await;
        gate.add_permits(1);
        settle().await;
        let out = lane.snapshot();
        let questions: Vec<_> = out.asks.iter().map(|a| a.question.as_str()).collect();
        assert_eq!(
            questions,
            vec![
                "What is your total headcount?",
                "Which cloud provider hosts production?",
                "How often do backups run?",
                "Who approves capital purchases?",
                "When does the lease expire?"
            ]
        );
    }

    #[tokio::test]
    async fn a_stale_checked_list_drops_dismissed_questions_before_the_cap() {
        let d = "ASK: Which storage platform do you run today? | WHY: platform unknown";
        let n = "ASK: Do you use encryption at rest? | WHY: security unknown";
        let five = "ASK: What is your total headcount? | WHY: size\nASK: What storage platform do you run today? | WHY: platform\nASK: Which cloud provider hosts production? | WHY: hosting\nASK: How often do backups run? | WHY: rpo\nASK: Who approves capital purchases? | WHY: buyer";
        let calls: Calls = Arc::default();
        let gate = Arc::new(Semaphore::new(1));
        let lane = AskLane::for_test_two(scripted(vec![d, d, n], vec![d, five], gate.clone(), calls.clone()), AskHooks::default());
        let mut log = sample_log();
        lane.request(&log, false); // both fire at point 1; deep's first turn passes
        settle().await;
        lane.request(&log, true); // deep fires at point 1 again, held
        settle().await;
        let id_d = lane.snapshot().asks[0].id.clone();
        assert!(lane.dismiss(&id_d));
        log.ingest(&tu("system", "someone else handles backups", 4.0, 5.0));
        log.ingest(&tu("mic", "right", 6.0, 7.0));
        lane.request(&log, false); // quick fires at point 2 and adds N
        settle().await;
        gate.add_permits(1); // deep lands, built at point 1
        settle().await;
        let out = lane.snapshot();
        let questions: Vec<_> = out.asks.iter().map(|a| a.question.as_str()).collect();
        assert_eq!(
            questions,
            vec![
                "What is your total headcount?",
                "Which cloud provider hosts production?",
                "How often do backups run?",
                "Who approves capital purchases?",
                "Do you use encryption at rest?"
            ]
        );
    }

    #[tokio::test]
    async fn a_quick_failure_shows_failed_even_while_the_deep_turn_runs() {
        let calls: Calls = Arc::default();
        let recorded = calls.clone();
        let gate = Arc::new(Semaphore::new(0));
        let g = gate.clone();
        let runner: Runner = Arc::new(move |turn, _d, _r| {
            let is_deep = turn.model.as_deref() == Some("deep-model");
            recorded.lock().unwrap().push(turn);
            let g = g.clone();
            Box::pin(async move {
                if is_deep {
                    g.acquire().await.unwrap().forget();
                    ClaudeOutcome { session_id: None, text: QUICK.into(), error: None }
                } else {
                    ClaudeOutcome { session_id: None, text: String::new(), error: Some("quick broke".into()) }
                }
            })
        });
        let lane = AskLane::for_test_two(runner, AskHooks::default());
        lane.request(&sample_log(), false);
        settle().await;
        let out = lane.snapshot();
        assert_eq!(out.state, "failed");
        assert!(out.checking, "deep is still running");
    }

    #[tokio::test]
    async fn a_deep_failure_after_an_empty_quick_result_does_not_show_failed() {
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
                    ClaudeOutcome { session_id: None, text: "NONE".into(), error: None }
                }
            })
        });
        let lane = AskLane::for_test_two(runner, rec.hooks.clone());
        lane.request(&sample_log(), false);
        settle().await;
        let out = lane.snapshot();
        assert!(out.asks.is_empty());
        assert_eq!(out.state, "idle", "the empty quick list was applied, so no failure shows");
        assert_eq!(rec.errors.lock().unwrap().as_slice(), ["deep broke"]);
    }

    #[tokio::test]
    async fn a_panicking_turn_does_not_leave_its_sub_lane_in_flight() {
        let calls: Calls = Arc::default();
        let recorded = calls.clone();
        let first_deep = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let runner: Runner = Arc::new(move |turn, _d, _r| {
            let is_deep = turn.model.as_deref() == Some("deep-model");
            recorded.lock().unwrap().push(turn);
            if is_deep && first_deep.swap(false, Ordering::SeqCst) {
                panic!("runner blew up");
            }
            Box::pin(async move {
                ClaudeOutcome { session_id: None, text: QUICK.to_string(), error: None }
            })
        });
        let lane = AskLane::for_test_two(runner, AskHooks::default());
        let log = sample_log();
        lane.request(&log, false);
        settle().await;
        let out = lane.snapshot();
        assert!(!out.checking, "the dead deep turn no longer counts as running");
        assert_eq!(out.state, "idle");

        lane.request(&log, true);
        settle().await;
        assert_eq!(model_calls(&calls, "deep-model"), 2, "the deep sub-lane fires again");
        assert!(lane.snapshot().checked);
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

    // ---- answered questions kept ----

    const STORAGE: &str = "What storage do you run today?";
    const DONE_STORAGE: &str = "DONE: What storage do you run today? | ANSWER: NetApp, about 40 TB. | BY: call";

    #[tokio::test]
    async fn a_question_the_call_answered_moves_to_answered_and_stays_off_the_list() {
        let calls: Calls = Arc::default();
        let replies: Vec<&'static str> = vec![
            "ASK: What storage do you run today? | WHY: storage platform unknown",
            DONE_STORAGE,
            "ASK: What storage do you run today? | WHY: storage platform unknown\nASK: Who handles backups? | WHY: owner unknown",
        ];
        let lane = AskLane::for_test(seq_runner(replies, calls.clone()), AskHooks::default());
        let log = sample_log();

        lane.request(&log, true);
        settle().await;
        let first = lane.snapshot();
        assert_eq!(first.asks[0].question, STORAGE);
        assert!(first.answered.is_empty());
        let id = first.asks[0].id.clone();

        lane.request(&log, true);
        settle().await;
        let second = lane.snapshot();
        assert!(second.asks.is_empty(), "the answered question left the open list");
        assert_eq!(second.answered.len(), 1);
        assert_eq!(second.answered[0].id, id);
        assert_eq!(second.answered[0].question, STORAGE);
        assert_eq!(second.answered[0].answer, "NetApp, about 40 TB.");
        assert_eq!(second.answered[0].how, AnswerHow::Call);
        assert!(second.answered[0].at > 0);

        lane.request(&log, true);
        settle().await;
        let third = lane.snapshot();
        assert_eq!(third.asks.len(), 1, "an answered question never comes back as ASK");
        assert_eq!(third.asks[0].question, "Who handles backups?");
        assert_eq!(third.answered, second.answered);
    }

    #[tokio::test]
    async fn a_done_only_reply_keeps_the_open_list_and_its_ids() {
        let calls: Calls = Arc::default();
        let replies: Vec<&'static str> = vec![
            "ASK: What storage do you run today? | WHY: platform\nASK: Who handles backups? | WHY: owner\nASK: Where is the data center? | WHY: site\nYOU: Is pricing in scope?",
            "DONE: What storage do you run today? | ANSWER: x | BY: call",
            "NONE",
        ];
        let lane = AskLane::for_test(seq_runner(replies, calls.clone()), AskHooks::default());
        let log = sample_log();
        lane.request(&log, true);
        settle().await;
        let first = lane.snapshot();
        let ids: Vec<String> = first.asks.iter().map(|a| a.id.clone()).collect();
        assert_eq!(ids.len(), 3);

        lane.request(&log, true);
        settle().await;
        let second = lane.snapshot();
        let open: Vec<(&str, &str)> = second.asks.iter().map(|a| (a.id.as_str(), a.question.as_str())).collect();
        assert_eq!(open, vec![(ids[1].as_str(), "Who handles backups?"), (ids[2].as_str(), "Where is the data center?")]);
        assert_eq!(second.for_you.len(), 1, "the For-you item stays too");
        assert_eq!(second.for_you[0].id, first.for_you[0].id);
        assert_eq!(second.answered.len(), 1);
        assert_eq!(second.answered[0].id, ids[0]);
        assert_eq!(second.answered[0].answer, "x");

        // A reply of exactly NONE still empties the list.
        lane.request(&log, true);
        settle().await;
        let third = lane.snapshot();
        assert!(third.asks.is_empty() && third.for_you.is_empty());
        assert_eq!(third.answered.len(), 1);
    }

    #[tokio::test]
    async fn a_reply_with_an_ask_line_still_replaces_the_open_list() {
        let calls: Calls = Arc::default();
        let replies: Vec<&'static str> = vec![
            "ASK: What storage do you run today? | WHY: platform\nASK: Who handles backups? | WHY: owner",
            "ASK: Where is the data center? | WHY: site\nDONE: What storage do you run today? | ANSWER: x | BY: call",
        ];
        let lane = AskLane::for_test(seq_runner(replies, calls.clone()), AskHooks::default());
        let log = sample_log();
        lane.request(&log, true);
        settle().await;
        lane.request(&log, true);
        settle().await;
        let out = lane.snapshot();
        let open: Vec<&str> = out.asks.iter().map(|a| a.question.as_str()).collect();
        assert_eq!(open, vec!["Where is the data center?"]);
    }

    #[tokio::test]
    async fn a_multi_line_answer_is_stored_on_one_line() {
        let calls: Calls = Arc::default();
        let replies: Vec<&'static str> = vec![GOOD];
        let lane = AskLane::for_test(seq_runner(replies, calls.clone()), AskHooks::default());
        lane.request(&sample_log(), true);
        settle().await;
        let out = lane.snapshot();
        assert!(lane.answer(&out.for_you[0].id, "Sam\n  Lee\n"));
        assert_eq!(lane.snapshot().answered[0].answer, "Sam Lee");
    }

    #[tokio::test]
    async fn answered_is_newest_first_and_never_shrinks() {
        let calls: Calls = Arc::default();
        let replies: Vec<&'static str> = vec![
            "ASK: Who handles backups? | WHY: owner unknown\nDONE: Where is the data center? | ANSWER: In Atlanta. | BY: call",
            "ASK: Who handles backups? | WHY: owner unknown\nDONE: When does the support contract end? | ANSWER: In March. | BY: chat",
            "NONE",
        ];
        let lane = AskLane::for_test(seq_runner(replies, calls), AskHooks::default());
        let log = sample_log();
        lane.request(&log, true);
        settle().await;
        lane.request(&log, true);
        settle().await;
        let out = lane.snapshot();
        let order: Vec<_> = out.answered.iter().map(|a| a.question.as_str()).collect();
        assert_eq!(order, vec!["When does the support contract end?", "Where is the data center?"]);
        assert_eq!(out.answered[0].how, AnswerHow::Chat);

        // A refresh that says NONE, a brief edit, a dismissal and a close remove nothing.
        lane.request(&log, true);
        settle().await;
        lane.set_brief("A new goal".to_string());
        assert!(lane.snapshot().asks.is_empty(), "NONE emptied the open list");
        assert_eq!(lane.snapshot().answered.len(), 2);
        lane.close();
        let after = lane.snapshot();
        assert_eq!(after.answered, out.answered, "answered stays readable after close");
        assert_eq!(lane.answered_items().len(), 2);
        assert_eq!(lane.answered_items()[0].question, "Where is the data center?", "the note list is oldest first");
    }

    #[tokio::test]
    async fn dismissing_an_open_item_does_not_touch_answered() {
        let calls: Calls = Arc::default();
        let replies: Vec<&'static str> = vec![
            "ASK: Who handles backups? | WHY: owner unknown\nASK: What storage do you run today? | WHY: platform unknown",
            "ASK: Who handles backups? | WHY: owner unknown\nDONE: What storage do you run today? | ANSWER: NetApp, about 40 TB. | BY: call",
        ];
        let lane = AskLane::for_test(seq_runner(replies, calls), AskHooks::default());
        let log = sample_log();
        lane.request(&log, true);
        settle().await;
        lane.request(&log, true);
        settle().await;
        let id = lane.snapshot().asks[0].id.clone();
        assert!(lane.dismiss(&id));
        assert_eq!(lane.snapshot().answered.len(), 1);
    }

    #[tokio::test]
    async fn answer_moves_a_for_you_item_to_answered_as_you() {
        let calls: Calls = Arc::default();
        let replies: Vec<&'static str> = vec![GOOD, GOOD];
        let lane = AskLane::for_test(seq_runner(replies, calls.clone()), AskHooks::default());
        let log = sample_log();
        lane.set_brief("My goal".to_string());
        lane.request(&log, true);
        settle().await;
        let out = lane.snapshot();
        let (you_id, ask_id) = (out.for_you[0].id.clone(), out.asks[0].id.clone());

        assert!(!lane.answer("no-such-id", "x"));
        assert!(!lane.answer(&you_id, "   "), "an empty answer is refused");
        assert_eq!(lane.snapshot().for_you.len(), 1);

        assert!(lane.answer(&you_id, "  Yes, pricing is in scope.  "));
        let after = lane.snapshot();
        assert!(after.for_you.is_empty());
        assert_eq!(after.brief, "My goal", "the lane does not touch the brief");
        assert_eq!(after.answered.len(), 1);
        assert_eq!(after.answered[0].id, you_id);
        assert_eq!(after.answered[0].question, "Is pricing in scope?");
        assert_eq!(after.answered[0].answer, "Yes, pricing is in scope.");
        assert_eq!(after.answered[0].how, AnswerHow::You);

        // An ASK item can be answered the same way.
        assert!(lane.answer(&ask_id, "Credit check and references."));
        assert_eq!(lane.snapshot().answered[0].id, ask_id);

        // The next reply lists both again: neither returns.
        lane.request(&log, true);
        settle().await;
        let again = lane.snapshot();
        assert!(again.asks.is_empty() && again.for_you.is_empty());
        assert_eq!(again.answered.len(), 2);
    }

    #[tokio::test]
    async fn answered_questions_are_sent_to_each_sub_lane_once() {
        let calls: Calls = Arc::default();
        let lane = AskLane::for_test_two(
            two_runner(DONE_STORAGE, DONE_STORAGE, None, None, calls.clone()),
            AskHooks::default(),
        );
        let log = sample_log();
        lane.request(&log, true);
        settle().await;
        assert_eq!(lane.snapshot().answered.len(), 1, "both sub-lanes said DONE, the first answer stays");
        lane.request(&log, true);
        settle().await;
        lane.request(&log, true);
        settle().await;

        let calls = calls.lock().unwrap();
        for model in ["ask-model", "deep-model"] {
            let prompts: Vec<_> =
                calls.iter().filter(|t| t.model.as_deref() == Some(model)).map(|t| t.prompt.clone()).collect();
            assert!(!prompts[0].contains("ANSWERED SO FAR"));
            assert!(prompts[1].contains("ANSWERED SO FAR (never ask these again):"));
            assert!(prompts[1].contains("What storage do you run today? Answer: NetApp, about 40 TB."));
            assert!(!prompts[2].contains("ANSWERED SO FAR"), "{} is told once", model);
        }
    }

    #[tokio::test]
    async fn an_ask_card_exchange_is_heard_once_by_each_sub_lane() {
        let calls: Calls = Arc::default();
        let lane = AskLane::for_test_two(two_runner("NONE", "NONE", None, None, calls.clone()), AskHooks::default());
        let chats: ChatLog = Arc::default();
        lane.set_chat_log(chats.clone());
        let log = sample_log();
        lane.request(&log, false);
        settle().await;
        assert_eq!(calls.lock().unwrap().len(), 2);

        chats.lock().unwrap().push(ChatExchange {
            question: "Who owns backups at FESCO?".to_string(),
            answer: "The ops team.\n- Per the vault note".to_string(),
        });
        lane.request(&log, false); // the exchange alone counts as news
        settle().await;
        lane.request(&log, false);
        settle().await;

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 4, "one turn per sub-lane for the exchange, none after");
        for t in &calls[2..] {
            assert!(t.prompt.contains("JOSEPH ASKED THE ASSISTANT PRIVATELY:"));
            assert!(t.prompt.contains("Q: Who owns backups at FESCO?"));
            assert!(t.prompt.contains("A: The ops team.\n- Per the vault note"));
            assert!(t.prompt.contains("BY: chat"));
        }
        assert!(!calls[0].prompt.contains("JOSEPH ASKED"));
    }

    #[test]
    fn merge_raw_never_brings_an_answered_question_back() {
        let item = |id: &str, q: &str| AskItem { id: id.into(), question: q.into(), why: String::new() };
        let shown = AskList {
            asks: vec![item("n1", "Who handles backups?"), item("n2", STORAGE)],
            for_you: vec![],
        };
        let answered = vec![AnsweredItem {
            id: "n1".into(),
            question: "Who handles backups?".into(),
            answer: "Ops.".into(),
            how: AnswerHow::Call,
            at: 1,
        }];
        let raw = RawList {
            asks: vec![("Who handles the backups?".into(), String::new()), ("When does support end?".into(), String::new())],
            for_you: vec![],
            done: vec![(STORAGE.into(), "NetApp.".into(), AnswerHow::Call)],
            ..Default::default()
        };
        let merged = merge_raw(&shown, &AskList::default(), raw, &[], &answered);
        let qs: Vec<_> = merged.asks.iter().map(|a| a.0.as_str()).collect();
        assert_eq!(qs, vec!["When does support end?"], "answered and DONE questions are out, extras included");
        assert_eq!(merged.done.len(), 1, "the DONE line itself survives for build");
    }

    #[tokio::test]
    async fn a_stale_quick_list_still_records_what_it_marked_answered() {
        let calls: Calls = Arc::default();
        let fast_gate = Arc::new(Semaphore::new(0));
        let fast = "ASK: Who handles backups? | WHY: owner unknown\nDONE: What storage do you run today? | ANSWER: NetApp. | BY: call";
        let deep = "ASK: What storage do you run today? | WHY: platform unknown\nASK: Who handles backups? | WHY: owner unknown";
        let lane = AskLane::for_test_two(
            two_runner(fast, deep, Some(fast_gate.clone()), None, calls),
            AskHooks::default(),
        );
        lane.request(&sample_log(), false);
        settle().await;
        let checked = lane.snapshot();
        assert!(checked.checked);
        assert_eq!(checked.asks.len(), 2);

        fast_gate.add_permits(1); // the quick turn lands after the checked list covers its point
        settle().await;
        let out = lane.snapshot();
        assert_eq!(out.answered.len(), 1);
        assert_eq!(out.answered[0].question, STORAGE);
        assert_eq!(out.asks.len(), 1, "the checked list lost the answered question");
        assert_eq!(out.asks[0].question, "Who handles backups?");
    }
}
