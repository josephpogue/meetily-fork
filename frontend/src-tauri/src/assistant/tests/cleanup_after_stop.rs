// Round 4: the cleanup after Stop. Stop closes the call's lanes, and Draft note
// only works after Stop, so a cleanup must run on closed lanes. Only a new
// recording (supersede) cancels it. A fake runner stands in for Claude and
// never spawns a process.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use super::super::claude_cli::{ClaudeOutcome, ClaudeTurn};
use super::super::cleanup::{self, CleanupState, Reason};
use super::super::lanes::{AnswerLanes, LaneConfig, Runner};
use super::start_beside_note;

const PIC1: &str = "zoom-frames/zoom-20261118-110300-000.jpg";
const PIC2: &str = "zoom-frames/zoom-20261118-110310-000.jpg";
const REMOVE_P1: &str = r#"{"remove":[{"id":"P1","reason":"before_share"}]}"#;

/// A meeting folder with two kept pictures, deleted when the test ends.
struct Meeting {
    dir: PathBuf,
}

impl Meeting {
    fn new() -> Meeting {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "meetily-after-stop-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dir.join("zoom-frames")).unwrap();
        std::fs::write(dir.join(PIC1), b"first picture").unwrap();
        std::fs::write(dir.join(PIC2), b"second picture").unwrap();
        Meeting { dir }
    }
    fn has(&self, rel: &str) -> bool {
        self.dir.join(rel).is_file()
    }
}

impl Drop for Meeting {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn cfg() -> LaneConfig {
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

/// A runner that answers at once with `text`, or fails with `error`.
fn answering(text: &'static str, error: Option<&'static str>) -> Runner {
    Arc::new(move |_turn: ClaudeTurn, _delta, _register| {
        let outcome = ClaudeOutcome { session_id: None, text: text.to_string(), error: error.map(|e| e.to_string()) };
        Box::pin(async move { outcome })
    })
}

/// A runner that signals `started`, then waits for `release` before answering.
fn gated(text: &'static str, started: Arc<Notify>, release: Arc<Notify>) -> Runner {
    Arc::new(move |_turn: ClaudeTurn, _delta, _register| {
        let (started, release) = (started.clone(), release.clone());
        Box::pin(async move {
            started.notify_one();
            release.notified().await;
            ClaudeOutcome { session_id: None, text: text.to_string(), error: None }
        })
    })
}

/// Lanes as after Stop: a saved deep session, closed.
fn stopped_lanes(runner: Runner) -> AnswerLanes {
    let mut lanes = AnswerLanes::from_saved_session(runner, "deep-1".to_string(), cfg(), vec![]);
    lanes.close();
    lanes
}

async fn wait_started(started: &Notify) {
    tokio::time::timeout(Duration::from_secs(10), started.notified())
        .await
        .expect("the cleanup run never started");
}

async fn until_idle(folder: &Path) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while cleanup::run_in_flight(folder) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the cleanup run never finished");
}

#[tokio::test]
async fn supersede_on_one_handle_is_seen_by_a_shared_handle() {
    let lanes = AnswerLanes::from_saved_session(answering("", None), "deep-1".to_string(), cfg(), vec![]);
    let shared = lanes.share();
    assert!(!lanes.is_superseded());
    assert!(!shared.is_superseded());
    lanes.supersede();
    assert!(lanes.is_superseded());
    assert!(shared.is_superseded());
}

#[tokio::test]
async fn close_alone_does_not_supersede() {
    let mut lanes = AnswerLanes::from_saved_session(answering("", None), "deep-1".to_string(), cfg(), vec![]);
    lanes.close();
    assert!(lanes.is_closed());
    assert!(!lanes.is_superseded());
}

#[tokio::test]
async fn draft_note_after_stop_runs_the_cleanup_and_moves_the_pictures() {
    let m = Meeting::new();
    let lanes = stopped_lanes(answering(REMOVE_P1, None));

    start_beside_note(None, m.dir.clone(), lanes, "transcript".to_string());
    until_idle(&m.dir).await;

    assert!(!m.has(PIC1), "P1 should have left zoom-frames");
    assert!(m.has("zoom-frames/removed/zoom-20261118-110300-000.jpg"), "P1 should be in removed/");
    assert!(m.has(PIC2), "P2 was not named, so it stays");
    let rec = cleanup::load(&m.dir).expect("a record should be written");
    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].file, PIC1);
    assert_eq!(rec.removed[0].reason, Reason::BeforeShare);
}

#[tokio::test]
async fn a_new_recording_mid_run_moves_nothing_and_writes_no_record() {
    let m = Meeting::new();
    let (started, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let lanes = stopped_lanes(gated(REMOVE_P1, started.clone(), release.clone()));
    let handle = lanes.share();

    start_beside_note(None, m.dir.clone(), lanes, "transcript".to_string());
    wait_started(&started).await; // the fake runner is now answering
    handle.supersede();
    release.notify_one();
    until_idle(&m.dir).await;

    assert!(m.has(PIC1) && m.has(PIC2), "both pictures stay in zoom-frames");
    assert!(!m.has("zoom-frames/removed/zoom-20261118-110300-000.jpg"));
    assert!(cleanup::load(&m.dir).is_none(), "no record for a superseded run");
    assert!(!cleanup::run_in_flight(&m.dir));
}

#[tokio::test]
async fn after_a_superseded_run_the_next_draft_note_runs_the_cleanup_again() {
    let m = Meeting::new();
    let (started, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let first = stopped_lanes(gated(REMOVE_P1, started.clone(), release.clone()));
    let handle = first.share();
    start_beside_note(None, m.dir.clone(), first, "transcript".to_string());
    wait_started(&started).await;
    handle.supersede();
    release.notify_one();
    until_idle(&m.dir).await;
    assert!(cleanup::load(&m.dir).is_none());

    // The same meeting, drafted again from lanes that were not superseded.
    start_beside_note(None, m.dir.clone(), stopped_lanes(answering(REMOVE_P1, None)), "transcript".to_string());
    until_idle(&m.dir).await;

    assert!(!m.has(PIC1));
    assert!(m.has("zoom-frames/removed/zoom-20261118-110300-000.jpg"));
    assert_eq!(cleanup::load(&m.dir).expect("record").state, CleanupState::Done);
}

#[tokio::test]
async fn lanes_superseded_before_draft_note_start_nothing() {
    let m = Meeting::new();
    let lanes = stopped_lanes(answering(REMOVE_P1, None));
    lanes.supersede();

    start_beside_note(None, m.dir.clone(), lanes, "transcript".to_string());
    assert!(!cleanup::run_in_flight(&m.dir), "no run should be marked in flight");
    until_idle(&m.dir).await;
    for _ in 0..20 {
        tokio::task::yield_now().await; // give a wrongly spawned run its chance
    }

    assert!(m.has(PIC1) && m.has(PIC2));
    assert!(cleanup::load(&m.dir).is_none());
    assert!(cleanup::load_session(&m.dir).is_none(), "no session file");
}

#[tokio::test]
async fn lanes_with_no_deep_session_start_nothing() {
    let m = Meeting::new();
    let mut lanes = AnswerLanes::new(answering(REMOVE_P1, None));
    lanes.close();

    start_beside_note(None, m.dir.clone(), lanes, "transcript".to_string());
    assert!(!cleanup::run_in_flight(&m.dir));
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    assert!(m.has(PIC1) && m.has(PIC2));
    assert!(cleanup::load(&m.dir).is_none());
}

#[tokio::test]
async fn a_failed_run_after_stop_writes_a_failed_record_and_moves_nothing() {
    let m = Meeting::new();
    let lanes = stopped_lanes(answering("", Some("boom")));

    start_beside_note(None, m.dir.clone(), lanes, "transcript".to_string());
    until_idle(&m.dir).await;

    assert!(m.has(PIC1) && m.has(PIC2), "a failed run moves nothing");
    let rec = cleanup::load(&m.dir).expect("a failed record should be written");
    assert!(matches!(rec.state, CleanupState::Failed { .. }), "got {:?}", rec.state);
    assert!(rec.removed.is_empty());
    assert!(!cleanup::run_in_flight(&m.dir));
}
