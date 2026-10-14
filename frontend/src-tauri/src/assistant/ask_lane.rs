// assistant/ask_lane.rs
//
// Ask Next: a third claude lineage forked from the same trunk as the answer
// lanes. Each turn sends what changed since the last one (transcript delta,
// a brief edit, dismissals) and gets back the whole list of questions
// Joseph should still ask. At most one turn runs at a time. Requests that
// land meanwhile coalesce into one follow-up, fired through
// `AskHooks::rerun`, because only core holds the transcript.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use super::ask_list::{self, AskItem, AskList};
use super::claude_cli::{ClaudeOutcome, ClaudeTurn, TurnHandle, NO_TOOLS};
use super::lanes::{run_with_retry, DeltaCb, FrameBatch, FrameFeed, FrameMode, RegisterCb, Runner};
use super::transcript::TranscriptLog;

pub const ASK_PROMPT: &str = include_str!("prompts/ask.md");

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
}

pub type AskEmitFn = Arc<dyn Fn(AsksOut) + Send + Sync>;

#[derive(Clone)]
pub struct AskHooks {
    pub emit: AskEmitFn,
    /// Called once when a turn finishes and requests landed while it ran.
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
    pub model: String,
}

#[derive(Default)]
struct AskState {
    lane_id: Option<String>,
    cfg: Option<AskConfig>,
    unavailable: bool,
    /// Transcript cursor; advances only when a turn succeeds.
    cursor: usize,
    in_flight: bool,
    dirty: bool,
    failed: bool,
    list: AskList,
    updated_at: Option<i64>,
    brief: String,
    /// Bumped on every brief edit; `brief_sent` is the version the model
    /// last received. The trunk seed already carries the opening brief, so
    /// both start at 0.
    brief_version: u64,
    brief_sent: u64,
    /// Every question Joseph dismissed this session, for filtering.
    dismissed: Vec<String>,
    /// Dismissals the model has not been told about yet.
    dismissed_unsent: Vec<String>,
    /// Bumped by `close`; a turn that finishes under an older generation is
    /// dropped on the floor.
    generation: u64,
    live_runs: Vec<TurnHandle>,
    /// Zoom window frames, when the assistant can see the window.
    frames: Option<FrameFeed>,
    /// Last frame seq sent; advances only when a turn succeeds.
    frame_cursor: u64,
}

impl AskState {
    fn snapshot(&self) -> AsksOut {
        let state = if self.unavailable {
            "unavailable"
        } else if self.in_flight {
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

    /// Forks the ask lineage off the seeded trunk. `trunk_id` is None when
    /// the trunk seed failed; the lane then reports "unavailable" for this
    /// call and every request is a no-op.
    pub async fn open(&self, trunk_id: Option<String>, cfg: AskConfig, brief: String) {
        {
            let mut s = self.state.lock().unwrap();
            s.brief = brief;
            s.cfg = Some(cfg.clone());
        }
        let lane_id = match trunk_id {
            Some(trunk) => {
                let turn = ClaudeTurn {
                    prompt: "Standing by. Reply with exactly the word READY.".to_string(),
                    cwd: cfg.cwd.clone(),
                    model: Some(cfg.model.clone()),
                    effort: Some(ASK_EFFORT.to_string()),
                    resume: Some(trunk),
                    fork: true,
                    append_system_prompt: Some(ASK_PROMPT.to_string()),
                    disallowed_tools: no_tools(),
                    partial: false,
                    safe_mode: true,
                    ..Default::default()
                };
                let outcome = self.run_bare(turn).await;
                match outcome.error {
                    Some(err) => {
                        log::warn!("assistant ask lane fork failed: {}", err);
                        None
                    }
                    None => outcome.session_id,
                }
            }
            None => None,
        };
        let out = {
            let mut s = self.state.lock().unwrap();
            s.unavailable = lane_id.is_none();
            s.lane_id = lane_id;
            s.snapshot()
        };
        (self.hooks.emit)(out);
    }

    async fn run_bare(&self, turn: ClaudeTurn) -> ClaudeOutcome {
        let state = self.state.clone();
        let register: Box<dyn FnOnce(TurnHandle) + Send> = Box::new(move |handle| {
            state.lock().unwrap().live_runs.push(handle);
        });
        (self.runner)(turn, Box::new(|_| {}), register).await
    }

    /// Runs one ask turn now, or marks a follow-up if one is already
    /// running. `force` runs even when nothing changed (the Refresh button);
    /// an automatic request with nothing new is a no-op.
    pub fn request(&self, log: &TranscriptLog, force: bool) {
        let (turn, batch, new_frame_cursor, generation, sent_version, sent_dismissed, new_cursor, out) = {
            let mut s = self.state.lock().unwrap();
            let (Some(lane_id), Some(cfg)) = (s.lane_id.clone(), s.cfg.clone()) else {
                return;
            };
            if s.in_flight {
                s.dirty = true;
                return;
            }
            let (delta, new_cursor) = log.delta_since(s.cursor);
            let in_progress = log.open_preview();
            let brief_changed = s.brief_version != s.brief_sent;
            let has_news = !delta.is_empty() || brief_changed || !s.dismissed_unsent.is_empty();
            if !force && !has_news {
                return;
            }
            let prompt = ask_prompt(
                &delta,
                &in_progress,
                brief_changed.then(|| s.brief.as_str()),
                &s.dismissed_unsent,
                &s.list,
            );
            s.in_flight = true;
            s.dirty = false;
            let turn = ClaudeTurn {
                prompt,
                cwd: cfg.cwd,
                model: Some(cfg.model),
                effort: Some(ASK_EFFORT.to_string()),
                resume: Some(lane_id),
                append_system_prompt: Some(ASK_PROMPT.to_string()),
                disallowed_tools: no_tools(),
                partial: false,
                safe_mode: true,
                ..Default::default()
            };
            let (batch, new_frame_cursor) = match s.frames.clone() {
                Some(feed) => feed.select(s.frame_cursor, FrameMode::All),
                None => (FrameBatch::default(), s.frame_cursor),
            };
            (
                turn,
                batch,
                new_frame_cursor,
                s.generation,
                s.brief_version,
                s.dismissed_unsent.len(),
                new_cursor,
                s.snapshot(),
            )
        };
        (self.hooks.emit)(out);

        let runner = self.runner.clone();
        let state = self.state.clone();
        let hooks = self.hooks.clone();
        tokio::spawn(async move {
            let register_state = state.clone();
            let outcome = run_with_retry(
                &runner,
                turn,
                batch,
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

            let (out, rerun, error) = {
                let mut s = state.lock().unwrap();
                if s.generation != generation {
                    return;
                }
                s.in_flight = false;
                let error = match parsed {
                    Some(raw) => {
                        let next = ask_list::build(&s.list, raw, &s.dismissed);
                        s.list = next;
                        s.cursor = new_cursor;
                        s.frame_cursor = new_frame_cursor;
                        s.brief_sent = sent_version;
                        s.dismissed_unsent.drain(..sent_dismissed);
                        s.updated_at = Some(chrono::Utc::now().timestamp_millis());
                        s.failed = false;
                        None
                    }
                    None => {
                        s.failed = true;
                        Some(
                            outcome
                                .error
                                .clone()
                                .unwrap_or_else(|| "Ask next could not read the model's reply.".to_string()),
                        )
                    }
                };
                (s.snapshot(), std::mem::take(&mut s.dirty), error)
            };
            (hooks.emit)(out);
            if let Some(err) = error {
                log::warn!("assistant ask turn failed: {}", err);
                (hooks.on_error)(err);
            }
            if rerun {
                (hooks.rerun)();
            }
        });
    }

    /// Hands the lane the frames of the Zoom window, or None when the
    /// assistant cannot see it. The cursor restarts at 0.
    pub fn set_frames(&self, feed: Option<FrameFeed>) {
        let mut s = self.state.lock().unwrap();
        s.frames = feed;
        s.frame_cursor = 0;
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

    /// Removes the item at once and remembers it so it never comes back.
    /// False when no item has that id.
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
            s.dismissed.push(removed.question.clone());
            s.dismissed_unsent.push(removed.question);
            s.snapshot()
        };
        (self.hooks.emit)(out);
        true
    }

    /// Kills an in-flight turn and stops all further turns. The last list
    /// stays visible until the next session opens a fresh lane.
    pub fn close(&self) {
        let (handles, out) = {
            let mut s = self.state.lock().unwrap();
            s.generation += 1;
            s.in_flight = false;
            s.dirty = false;
            s.lane_id = None;
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

    impl AskLane {
        fn for_test(runner: Runner, hooks: AskHooks) -> Self {
            let lane = AskLane::new(runner, hooks);
            {
                let mut s = lane.state.lock().unwrap();
                s.lane_id = Some("ask-1".to_string());
                s.cfg = Some(AskConfig { cwd: std::env::temp_dir(), model: "ask-model".to_string() });
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
            AskConfig { cwd: std::env::temp_dir(), model: "ask-model".to_string() },
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
            AskConfig { cwd: std::env::temp_dir(), model: "ask-model".to_string() },
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
}
