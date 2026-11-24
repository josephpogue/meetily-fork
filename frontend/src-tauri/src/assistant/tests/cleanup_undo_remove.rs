// Locked tests for `undo_remove` (assistant/cleanup.rs). Written by the tester
// from the "Round 2: Undo of a Remove leaves no trace" section of
// docs/superpowers/plans/2026-11-23-all-pictures-brief.md. Every expected value
// comes from that section's rules, state table and worked examples.
// The meeting folder is the brief's: T1 200 bytes, Z 100, T3 300, T4 400.

use super::*;
use std::fs;
use std::path::Path;

const FIVE_DAYS: i64 = 432_000_000;

const T1: &str = "teams-frames/teams-20261118-110300-478.jpg"; // 200 bytes
const Z: &str = "zoom-frames/zoom-20261118-110310-000.jpg"; // 100 bytes
const T3: &str = "teams-frames/teams-20261118-110332-532.jpg"; // 300 bytes
const T4: &str = "teams-frames/teams-20261118-110433-637.jpg"; // 400 bytes

fn write(root: &Path, rel: &str, bytes: usize) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![b'x'; bytes]).unwrap();
}

fn example() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), T1, 200);
    write(d.path(), Z, 100);
    write(d.path(), T3, 300);
    write(d.path(), T4, 400);
    d
}

fn removed_of(file: &str) -> String {
    let (dir, name) = file.rsplit_once('/').unwrap();
    format!("{}/removed/{}", dir, name)
}

fn s(x: &str) -> String {
    x.to_string()
}

fn removal(id: &str, reason: Reason, caption: Option<&str>, repeat_of: Option<&str>) -> Removal {
    Removal { id: s(id), reason, caption: caption.map(s), repeat_of: repeat_of.map(s) }
}

fn entry_of<'a>(rec: &'a CleanupRecord, file: &str) -> Option<&'a RemovedPicture> {
    rec.removed.iter().find(|m| m.file == file)
}

fn raw_record(folder: &Path) -> CleanupRecord {
    serde_json::from_str(&fs::read_to_string(folder.join(RECORD_FILE)).unwrap()).unwrap()
}

fn files_of(all: &AllPictures) -> Vec<String> {
    all.pictures.iter().map(|p| p.file.clone()).collect()
}

/// Claude removed T1 as a popup (Done, ran at 5000, clock 432_005_000).
fn claude_removed_t1(folder: &Path) -> CleanupRecord {
    let pics = list_pictures(folder);
    apply(folder, &pics, &[removal("P1", Reason::Popup, Some("Save changes? box"), None)], 5000).unwrap()
}

// ---------------------------------------------------------------------------
// The state table
// ---------------------------------------------------------------------------

#[test]
fn not_run_worked_example_leaves_no_trace() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let rec = undo_remove(d.path(), &[s(T4)]).unwrap();

    assert_eq!(fs::metadata(d.path().join(T4)).unwrap().len(), 400);
    assert!(!d.path().join(removed_of(T4)).exists());

    assert_eq!(rec.state, CleanupState::NotRun);
    assert_eq!(rec.ran_at_ms, 0);
    assert!(rec.removed.is_empty());
    assert_eq!(entry_of(&rec, T4), None);

    // The record is on disk and reads back the same.
    assert_eq!(raw_record(d.path()), rec);
    assert_eq!(load(d.path()), Some(rec.clone()));

    // Follow-on reads: summary and all_pictures.
    let sum = summary(Some(&rec), 2000);
    assert_eq!((sum.by_you, sum.removed, sum.bytes, sum.delete_after_ms), (0, 0, 0, 0));
    assert_eq!(sum.line, None);
    assert!(!sum.failed);

    let all = all_pictures(d.path());
    assert_eq!(files_of(&all), vec![s(T1), s(Z), s(T3), s(T4)]);
    assert_eq!((all.kept, all.removed), (4, 0));
    assert!(all.pictures.iter().all(|p| !p.removed && p.shown_file == p.file));
    assert_eq!(all.title, "4 pictures kept, under 1 MB");
}

#[test]
fn a_record_left_with_no_entries_is_still_written_and_should_start_stays_true() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let rec = undo_remove(d.path(), &[s(T4)]).unwrap();

    assert!(d.path().join(RECORD_FILE).exists(), "the record file is still there");
    assert_eq!(raw_record(d.path()), rec);
    assert_eq!(rec.state, CleanupState::NotRun);
    assert!(should_start(Some(&rec), false));
    assert!(should_start(load(d.path()).as_ref(), false));
    assert!(!should_start(Some(&rec), true));
}

#[test]
fn done_with_claudes_pending_entry_kept_and_a_by_you_entry_undone() {
    let d = example();
    let before = claude_removed_t1(d.path());
    let t1_before = entry_of(&before, T1).unwrap().clone();
    assert_eq!(t1_before.delete_after_ms, 432_005_000);
    remove_by_you(d.path(), &[s(T4)], 6000).unwrap();

    let rec = undo_remove(d.path(), &[s(T4)]).unwrap();

    assert!(d.path().join(T4).exists());
    assert!(!d.path().join(removed_of(T4)).exists());
    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.ran_at_ms, 5000);
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(entry_of(&rec, T4), None);
    assert_eq!(entry_of(&rec, T1).unwrap(), &t1_before);
    assert!(d.path().join(removed_of(T1)).exists(), "Claude's picture stays removed");
    assert_eq!(load(d.path()), Some(rec.clone()));
    assert_eq!(raw_record(d.path()), rec);

    let sum = summary(Some(&rec), 7000);
    assert_eq!(sum.line.as_deref(), Some("Claude removed 1 picture to free under 1 MB: 1 not useful."));
    assert!(!sum.line.unwrap().contains("You removed"));
    assert_eq!((sum.removed, sum.not_useful, sum.by_you), (1, 1, 0));
}

#[test]
fn failed_with_a_by_you_entry_keeps_failed_and_drops_the_entry() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    record_failure(d.path(), "boom", 2000).unwrap();

    let rec = undo_remove(d.path(), &[s(T4)]).unwrap();

    assert!(d.path().join(T4).exists());
    assert!(!d.path().join(removed_of(T4)).exists());
    assert_eq!(rec.state, CleanupState::Failed { error: s("boom") });
    assert_eq!(rec.ran_at_ms, 2000);
    assert_eq!(entry_of(&rec, T4), None);
    assert!(rec.removed.is_empty());
    assert_eq!(load(d.path()), Some(rec.clone()));
    assert_eq!(raw_record(d.path()), rec);
    assert!(should_start(Some(&rec), false));
}

#[test]
fn an_already_restored_by_you_entry_is_skipped_and_the_record_is_unchanged() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let restored = restore(d.path(), &[s(T4)]).unwrap();
    assert!(entry_of(&restored, T4).unwrap().restored);

    let rec = undo_remove(d.path(), &[s(T4)]).unwrap();

    assert_eq!(rec, restored);
    assert!(entry_of(&rec, T4).unwrap().restored, "it keeps its Restored tag");
    assert_eq!(entry_of(&rec, T4).unwrap().reason, Reason::ByYou);
    assert!(d.path().join(T4).exists());
    assert!(!d.path().join(removed_of(T4)).exists());
    assert_eq!(load(d.path()), Some(rec));
}

#[test]
fn a_claude_pending_entry_is_skipped_and_stays_removed() {
    let d = example();
    let before = claude_removed_t1(d.path());

    let rec = undo_remove(d.path(), &[s(T1)]).unwrap();

    assert_eq!(rec, before);
    assert!(d.path().join(removed_of(T1)).exists());
    assert!(!d.path().join(T1).exists());
    let e = entry_of(&rec, T1).unwrap();
    assert_eq!((e.reason, e.restored), (Reason::Popup, false));
    assert_eq!(load(d.path()), Some(before));
}

#[test]
fn a_name_no_entry_lists_is_skipped_and_is_ok() {
    let d = example();
    let before = remove_by_you(d.path(), &[s(T4)], 1000).unwrap();

    let rec = undo_remove(d.path(), &[s("teams-frames/nope.jpg")]).unwrap();

    assert_eq!(rec, before);
    assert!(d.path().join(removed_of(T4)).exists());
    assert!(!d.path().join(T4).exists());
    assert_eq!(load(d.path()), Some(before));
}

#[test]
fn a_kept_picture_with_no_entry_is_skipped_and_stays_put() {
    // Z was never removed: it is kept, no entry lists it.
    let d = example();
    let before = remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let rec = undo_remove(d.path(), &[s(Z)]).unwrap();
    assert_eq!(rec, before);
    assert_eq!(fs::metadata(d.path().join(Z)).unwrap().len(), 100);
    assert!(d.path().join(removed_of(T4)).exists());
}

#[test]
fn no_record_is_err_and_nothing_is_written() {
    let d = example();
    assert!(undo_remove(d.path(), &[s(T4)]).is_err());
    assert!(undo_remove(d.path(), &[]).is_err());
    assert!(!d.path().join(RECORD_FILE).exists());
    assert!(!d.path().join("teams-frames/removed").exists());
    assert!(!d.path().join("zoom-frames/removed").exists());
    for f in [T1, Z, T3, T4] {
        assert!(d.path().join(f).exists());
    }
}

// ---------------------------------------------------------------------------
// Worked examples
// ---------------------------------------------------------------------------

#[test]
fn claude_removed_t1_then_joseph_removed_t3_then_undo_of_t3_and_t1_only_brings_t3_back() {
    let d = example();
    claude_removed_t1(d.path());
    remove_by_you(d.path(), &[s(T3)], 6000).unwrap();

    let rec = undo_remove(d.path(), &[s(T3), s(T1)]).unwrap();

    assert!(d.path().join(T3).exists());
    assert!(!d.path().join(removed_of(T3)).exists());
    assert_eq!(entry_of(&rec, T3), None);
    assert!(d.path().join(removed_of(T1)).exists(), "T1 is skipped and still removed");
    assert!(!d.path().join(T1).exists());

    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.ran_at_ms, 5000);
    assert_eq!(rec.removed.len(), 1);
    let t1 = entry_of(&rec, T1).unwrap();
    assert_eq!((t1.reason, t1.restored, t1.delete_after_ms), (Reason::Popup, false, 432_005_000));
    assert_eq!(load(d.path()), Some(rec.clone()));

    assert_eq!(
        summary(Some(&rec), 7000).line.as_deref(),
        Some("Claude removed 1 picture to free under 1 MB: 1 not useful.")
    );
}

#[test]
fn with_two_pending_only_the_named_by_you_picture_comes_back() {
    let d = example();
    remove_by_you(d.path(), &[s(T4), s(T3)], 1000).unwrap();

    let rec = undo_remove(d.path(), &[s(T4)]).unwrap();

    assert!(d.path().join(T4).exists());
    assert!(!d.path().join(removed_of(T4)).exists());
    assert!(!d.path().join(T3).exists());
    assert!(d.path().join(removed_of(T3)).exists());
    assert_eq!(rec.removed.len(), 1);
    let t3 = entry_of(&rec, T3).unwrap();
    assert_eq!((t3.reason, t3.restored, t3.delete_after_ms), (Reason::ByYou, false, 1000 + FIVE_DAYS));
    assert_eq!(entry_of(&rec, T4), None);

    let sum = summary(Some(&rec), 2000);
    assert_eq!(sum.by_you, 1);
    assert_eq!(sum.removed, 0);
    assert_eq!(sum.bytes, 300);
    assert_eq!(sum.line.as_deref(), Some("You removed 1 picture to free under 1 MB."));

    let all = all_pictures(d.path());
    assert_eq!((all.kept, all.removed), (3, 1));
    assert_eq!(all.title, "3 pictures kept, under 1 MB");
}

#[test]
fn undo_of_several_names_brings_every_named_by_you_picture_back() {
    let d = example();
    remove_by_you(d.path(), &[s(T4), s(Z)], 1000).unwrap();
    let rec = undo_remove(d.path(), &[s(T4), s(Z)]).unwrap();
    assert!(rec.removed.is_empty());
    assert!(d.path().join(T4).exists() && d.path().join(Z).exists());
    assert_eq!(all_pictures(d.path()).kept, 4);
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

#[test]
fn a_kept_picture_already_at_the_place_is_never_overwritten() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    // A different picture now sits at T4's place (77 bytes, not the removed 400).
    write(d.path(), T4, 77);

    let rec = undo_remove(d.path(), &[s(T4)]).unwrap();

    assert_eq!(fs::metadata(d.path().join(T4)).unwrap().len(), 77, "the kept picture is untouched");
    assert_eq!(fs::metadata(d.path().join(removed_of(T4))).unwrap().len(), 400, "the removed copy stays");
    assert_eq!(entry_of(&rec, T4), None, "the entry is dropped");
    assert!(rec.removed.is_empty());
    assert_eq!(rec.state, CleanupState::NotRun);
}

#[test]
fn a_claude_entry_that_remove_by_you_replaced_is_not_brought_back() {
    // Claude removed T1, Joseph restored it, then removed it himself, then undid that.
    let d = example();
    claude_removed_t1(d.path());
    restore(d.path(), &[s(T1)]).unwrap();
    remove_by_you(d.path(), &[s(T1)], 300_000_000).unwrap();

    let rec = undo_remove(d.path(), &[s(T1)]).unwrap();

    assert!(d.path().join(T1).exists(), "the picture is simply kept");
    assert!(!d.path().join(removed_of(T1)).exists());
    assert_eq!(entry_of(&rec, T1), None);
    assert!(rec.removed.is_empty());
    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.ran_at_ms, 5000);
    let sum = summary(Some(&rec), 300_000_001);
    assert_eq!((sum.removed, sum.by_you), (0, 0));
    assert_eq!(sum.line, None);
}

#[test]
fn state_and_ran_at_ms_do_not_change() {
    // Done, ran at 5000.
    let d = example();
    claude_removed_t1(d.path());
    remove_by_you(d.path(), &[s(T4)], 6000).unwrap();
    let rec = undo_remove(d.path(), &[s(T4)]).unwrap();
    assert_eq!((rec.state.clone(), rec.ran_at_ms), (CleanupState::Done, 5000));

    // Failed, ran at 2000.
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    record_failure(d.path(), "boom", 2000).unwrap();
    let rec = undo_remove(d.path(), &[s(T4)]).unwrap();
    assert_eq!((rec.state.clone(), rec.ran_at_ms), (CleanupState::Failed { error: s("boom") }, 2000));

    // NotRun, ran at 0.
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let rec = undo_remove(d.path(), &[s(T4)]).unwrap();
    assert_eq!((rec.state.clone(), rec.ran_at_ms), (CleanupState::NotRun, 0));
}

#[test]
fn undo_remove_with_an_empty_list_changes_nothing() {
    let d = example();
    let before = remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let rec = undo_remove(d.path(), &[]).unwrap();
    assert_eq!(rec, before);
    assert!(d.path().join(removed_of(T4)).exists());
}

#[test]
fn restore_then_remove_again_still_works_after_an_undo_remove_of_another_picture() {
    // "restore then Undo still use restore and remove_again as before."
    let d = example();
    remove_by_you(d.path(), &[s(T4), s(T3)], 1000).unwrap();
    restore(d.path(), &[s(T4)]).unwrap();
    let again = remove_again(d.path(), &[s(T4)]).unwrap();
    assert!(!entry_of(&again, T4).unwrap().restored);
    let rec = undo_remove(d.path(), &[s(T3)]).unwrap();
    assert_eq!(entry_of(&rec, T3), None);
    assert!(!entry_of(&rec, T4).unwrap().restored);
    assert!(d.path().join(removed_of(T4)).exists());
}
