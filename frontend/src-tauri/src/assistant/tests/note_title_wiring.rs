// Locked tests for two meeting-title fixes:
//   1. Save renames the meeting only once the vault note has been filed, and the
//      filed note and the project page use the title being applied.
//   2. The Title box appears once the meeting row lands (note_titles_after_save).
// Child of `core`, so core's private items are reached through `super::`.

use super::tests::{capture_notes, git_vault, note_runner, settle, test_lane_cfg};
use super::*;
use crate::database::repositories::assistant_items::tests::{add_meeting_at, test_pool};
use std::collections::BTreeMap;

const TIMESTAMP_NAME: &str = "Meeting 25_11_26_11_03_31";

async fn db_title(pool: &SqlitePool, id: &str) -> String {
    sqlx::query_scalar::<_, String>("SELECT title FROM meetings WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Opens the assistant the way the existing save tests do and returns the
/// history collector, closed and not yet saved to any meeting.
async fn open_history(handle: &AssistantHandle) -> super::super::history::HistoryHandle {
    let history = super::tests::collect_asks(handle).await;
    let calls = Arc::new(StdMutex::new(Vec::new()));
    open_lanes_with(handle, test_lane_cfg(), "seed".to_string(), String::new(), note_runner(calls)).await;
    refresh_asks(handle).await;
    settle().await;
    refresh_asks(handle).await;
    settle().await;
    close_session(handle).await;
    history
}

fn add_project_page(vault: &std::path::Path, slug: &str) {
    std::fs::write(vault.join("Wiki/Projects").join(format!("{slug}.md")), format!("# {slug}\n")).unwrap();
}

fn suggestions(current: &str) -> TitleSuggestions {
    let mut by_project = BTreeMap::new();
    by_project.insert("proj-a".to_string(), "AAA - Topic 11/25/26".to_string());
    by_project.insert("proj-b".to_string(), "BBB - Topic 11/25/26".to_string());
    TitleSuggestions {
        by_project,
        no_project: "Topic 11/25/26".to_string(),
        current: current.to_string(),
        keep_current: false,
    }
}

/// A saved meeting m1, a drafted (Ready) note, a vault with two project pages
/// and the Title suggestions set. Returns (pool, handle, vault, meeting folder).
async fn ready_to_save() -> (SqlitePool, AssistantHandle, tempfile::TempDir, tempfile::TempDir) {
    let pool = test_pool().await;
    let folder = tempfile::tempdir().unwrap();
    add_meeting_at(&pool, "m1", TIMESTAMP_NAME, Some(folder.path().to_str().unwrap()), "2026-11-25T15:00:00Z").await;
    let vault = git_vault();
    add_project_page(vault.path(), "proj-a");
    add_project_page(vault.path(), "proj-b");
    let handle = AssistantHandle::new();
    let history = open_history(&handle).await;
    super::super::history::save_for_meeting(&pool, &history, "m1").await.unwrap();
    handle.0.lock().await.settings.vault_root = vault.path().to_string_lossy().into_owned();
    assert!(draft_note(&handle).await.is_ok());
    {
        let mut core = handle.0.lock().await;
        core.note_titles = Some(suggestions(TIMESTAMP_NAME));
        core.note_state = NoteState::Ready;
    }
    (pool, handle, vault, folder)
}

// ---- Fix 1 ----

#[tokio::test]
async fn a_save_that_fails_to_file_the_note_does_not_rename_and_the_retry_names_the_filed_note() {
    let (pool, handle, vault, _folder) = ready_to_save().await;

    // Break filing after the project check passes: _sources/meetings is a file.
    std::fs::create_dir_all(vault.path().join("_sources")).unwrap();
    std::fs::write(vault.path().join("_sources/meetings"), "in the way").unwrap();

    let first = save_note(
        &handle,
        &pool,
        Some("proj-a".to_string()),
        Some(TitleChoice { text: "AAA - Topic 11/25/26".to_string(), edited: false }),
    )
    .await;
    assert!(first.is_err(), "filing failed, so Save fails: {first:?}");
    assert_eq!(db_title(&pool, "m1").await, TIMESTAMP_NAME, "no rename before the note is filed");

    // Repair the vault and try again with the other project.
    std::fs::remove_file(vault.path().join("_sources/meetings")).unwrap();
    handle.0.lock().await.note_state = NoteState::Ready;

    let saved = save_note(
        &handle,
        &pool,
        Some("proj-b".to_string()),
        Some(TitleChoice { text: "BBB - Topic 11/25/26".to_string(), edited: false }),
    )
    .await
    .expect("the repaired vault files the note");
    assert_eq!(db_title(&pool, "m1").await, "BBB - Topic 11/25/26");
    let note = std::fs::read_to_string(&saved.note_path).unwrap();
    assert_eq!(note.lines().next(), Some("# BBB - Topic 11/25/26"), "{note}");
    assert!(
        saved.note_path.contains("_sources/meetings"),
        "the note is filed under _sources/meetings: {}",
        saved.note_path
    );
    let page = std::fs::read_to_string(vault.path().join("Wiki/Projects/proj-b.md")).unwrap();
    let meetings_line = page
        .split("## Meetings")
        .nth(1)
        .and_then(|rest| rest.lines().find(|l| l.contains("BBB - Topic 11/25/26")));
    assert!(meetings_line.is_some(), "a line under ## Meetings names the new title: {page}");
}

#[tokio::test]
async fn a_successful_save_with_an_edited_title_renames_the_meeting_and_heads_the_filed_note() {
    let (pool, handle, vault, _folder) = ready_to_save().await;

    let saved = save_note(
        &handle,
        &pool,
        Some("proj-a".to_string()),
        Some(TitleChoice { text: "PNC - My Own Title 11/25/26".to_string(), edited: true }),
    )
    .await
    .expect("save");
    assert_eq!(db_title(&pool, "m1").await, "PNC - My Own Title 11/25/26");
    let note = std::fs::read_to_string(&saved.note_path).unwrap();
    assert_eq!(note.lines().next(), Some("# PNC - My Own Title 11/25/26"), "{note}");
    let page = std::fs::read_to_string(vault.path().join("Wiki/Projects/proj-a.md")).unwrap();
    assert!(page.contains("PNC - My Own Title 11/25/26"), "the project page line uses the title: {page}");
}

// ---- Fix 2 ----

const TOPIC: &str = "VCF Design Workshop";

fn codes_file(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("project-codes.json");
    std::fs::write(&p, r#"{"codes": {"proj-a": "PNC"}}"#).unwrap();
    p
}

/// A Ready note that landed with no titles, and the sources to build them from.
async fn ready_without_titles(handle: &AssistantHandle) {
    let mut core = handle.0.lock().await;
    core.note_state = NoteState::Ready;
    core.note_titles = None;
    core.note_title_src = Some((Some(TOPIC.to_string()), vec!["proj-a".to_string()]));
}

async fn saved_meeting(pool: &SqlitePool, history: &super::super::history::HistoryHandle) {
    add_meeting_at(pool, "m1", TIMESTAMP_NAME, None, "2026-10-26T15:00:00Z").await;
    super::super::history::save_for_meeting(pool, history, "m1").await.unwrap();
}

#[tokio::test]
async fn without_a_saved_meeting_the_titles_wait() {
    let pool = test_pool().await;
    let dir = tempfile::tempdir().unwrap();
    let handle = AssistantHandle::new();
    let _history = open_history(&handle).await;
    ready_without_titles(&handle).await;
    let sent = capture_notes(&handle).await;

    note_titles_after_save(&handle, &pool, &codes_file(dir.path())).await;

    assert!(handle.0.lock().await.note_titles.is_none());
    assert!(sent.lock().unwrap().is_empty(), "nothing is emitted");
}

#[tokio::test]
async fn once_the_row_lands_the_titles_are_built_and_the_panel_is_told_once() {
    let pool = test_pool().await;
    let dir = tempfile::tempdir().unwrap();
    let handle = AssistantHandle::new();
    let history = open_history(&handle).await;
    ready_without_titles(&handle).await;
    saved_meeting(&pool, &history).await;
    let sent = capture_notes(&handle).await;

    note_titles_after_save(&handle, &pool, &codes_file(dir.path())).await;

    let titles = handle.0.lock().await.note_titles.clone().expect("titles are set");
    assert_eq!(titles.by_project.get("proj-a").map(String::as_str), Some("PNC - VCF Design Workshop 10/26/26"));
    assert_eq!(titles.no_project, "VCF Design Workshop 10/26/26");
    assert!(!titles.keep_current);
    assert_eq!(titles.current, TIMESTAMP_NAME);
    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "exactly one note is emitted");
    assert_eq!(sent[0].titles.as_ref(), Some(&titles));
    assert_eq!(sent[0].state, "ready");
}

#[tokio::test]
async fn a_missing_codes_file_gives_titles_without_a_code() {
    let pool = test_pool().await;
    let dir = tempfile::tempdir().unwrap();
    let handle = AssistantHandle::new();
    let history = open_history(&handle).await;
    ready_without_titles(&handle).await;
    saved_meeting(&pool, &history).await;

    note_titles_after_save(&handle, &pool, &dir.path().join("no-such-codes.json")).await;

    let titles = handle.0.lock().await.note_titles.clone().expect("titles are set");
    assert_eq!(titles.by_project.get("proj-a").map(String::as_str), Some("VCF Design Workshop 10/26/26"));
}

#[tokio::test]
async fn a_note_that_is_not_ready_is_left_alone() {
    for state in [NoteState::Saved, NoteState::Idle, NoteState::Failed] {
        let pool = test_pool().await;
        let dir = tempfile::tempdir().unwrap();
        let handle = AssistantHandle::new();
        let history = open_history(&handle).await;
        ready_without_titles(&handle).await;
        handle.0.lock().await.note_state = state;
        saved_meeting(&pool, &history).await;
        let sent = capture_notes(&handle).await;

        note_titles_after_save(&handle, &pool, &codes_file(dir.path())).await;

        let core = handle.0.lock().await;
        assert!(core.note_titles.is_none(), "{state:?}: no titles");
        assert_eq!(core.note_state, state, "{state:?}: state unchanged");
        assert!(sent.lock().unwrap().is_empty(), "{state:?}: nothing emitted");
    }
}

#[tokio::test]
async fn titles_already_there_are_kept_and_nothing_is_emitted() {
    let pool = test_pool().await;
    let dir = tempfile::tempdir().unwrap();
    let handle = AssistantHandle::new();
    let history = open_history(&handle).await;
    ready_without_titles(&handle).await;
    let existing = suggestions(TIMESTAMP_NAME);
    handle.0.lock().await.note_titles = Some(existing.clone());
    saved_meeting(&pool, &history).await;
    let sent = capture_notes(&handle).await;

    note_titles_after_save(&handle, &pool, &codes_file(dir.path())).await;

    assert_eq!(handle.0.lock().await.note_titles, Some(existing));
    assert!(sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn without_title_sources_nothing_happens() {
    let pool = test_pool().await;
    let dir = tempfile::tempdir().unwrap();
    let handle = AssistantHandle::new();
    let history = open_history(&handle).await;
    ready_without_titles(&handle).await;
    handle.0.lock().await.note_title_src = None;
    saved_meeting(&pool, &history).await;
    let sent = capture_notes(&handle).await;

    note_titles_after_save(&handle, &pool, &codes_file(dir.path())).await;

    assert!(handle.0.lock().await.note_titles.is_none());
    assert!(sent.lock().unwrap().is_empty());
}
