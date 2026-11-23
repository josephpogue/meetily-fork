// Locked tests for the "All pictures" job (assistant/cleanup.rs). Written by the
// tester from docs/superpowers/plans/2026-11-23-all-pictures-brief.md. Every
// expected value comes from that brief's states table, rules and worked examples.
// The meeting folder is the brief's: T1 200 bytes, Z 100, T3 300, T4 400.

use super::*;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier};

const DAY_MS: i64 = 86_400_000;
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

/// A hand-built entry for `summary` tests (the files need not exist).
fn rp(file: &str, reason: Reason, bytes: u64, restored: bool, delete_after_ms: i64) -> RemovedPicture {
    RemovedPicture {
        file: s(file),
        removed_file: file.replacen("-frames/", "-frames/removed/", 1),
        time: s("11:03:00"),
        reason,
        caption: None,
        repeat_of: None,
        bytes,
        restored,
        delete_after_ms,
    }
}

fn rec_of(state: CleanupState, ran_at_ms: i64, removed: Vec<RemovedPicture>) -> CleanupRecord {
    CleanupRecord { state, ran_at_ms, removed }
}

fn files_of(all: &AllPictures) -> Vec<String> {
    all.pictures.iter().map(|p| p.file.clone()).collect()
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    v.sort();
    v
}

// ---------------------------------------------------------------------------
// all_pictures
// ---------------------------------------------------------------------------

#[test]
fn all_pictures_with_no_record_lists_every_picture_as_kept_oldest_first() {
    let d = example();
    let all = all_pictures(d.path());

    assert_eq!(files_of(&all), vec![s(T1), s(Z), s(T3), s(T4)]);
    assert_eq!((all.kept, all.kept_bytes, all.removed), (4, 1000, 0));
    assert_eq!(all.title, "4 pictures kept, under 1 MB");

    let t1 = &all.pictures[0];
    assert_eq!(t1.shown_file, T1);
    assert_eq!(t1.time, "11:03:00");
    assert_eq!(t1.bytes, 200);
    assert!(!t1.removed);
    assert_eq!(t1.reason, None);
    assert_eq!(t1.caption, None);
    assert_eq!(t1.repeat_of, None);
    assert_eq!(t1.delete_after_ms, 0);
    let bytes: Vec<u64> = all.pictures.iter().map(|p| p.bytes).collect();
    assert_eq!(bytes, vec![200, 100, 300, 400]);
    assert!(all.pictures.iter().all(|p| !p.removed && p.shown_file == p.file));
}

#[test]
fn all_pictures_orders_by_the_time_in_the_file_name_across_both_folders() {
    let d = example();
    // A zoom picture older than every teams picture, and a teams one newer than all.
    write(d.path(), "zoom-frames/zoom-20261118-110200-000.jpg", 10);
    write(d.path(), "teams-frames/teams-20261118-110500-000.jpg", 10);
    let all = all_pictures(d.path());
    assert_eq!(
        files_of(&all),
        vec![
            s("zoom-frames/zoom-20261118-110200-000.jpg"),
            s(T1),
            s(Z),
            s(T3),
            s(T4),
            s("teams-frames/teams-20261118-110500-000.jpg"),
        ]
    );
}

#[test]
fn all_pictures_without_frames_folders_is_empty() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "audio.mp4", 50);
    let all = all_pictures(d.path());
    assert!(all.pictures.is_empty());
    assert_eq!((all.kept, all.kept_bytes, all.removed), (0, 0, 0));
    assert_eq!(all.title, "No pictures kept");
}

#[test]
fn all_pictures_after_remove_by_you_shows_the_removed_one_in_its_time_spot() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let all = all_pictures(d.path());

    assert_eq!(files_of(&all), vec![s(T1), s(Z), s(T3), s(T4)]);
    assert_eq!((all.kept, all.removed), (3, 1));
    assert_eq!(all.kept_bytes, 600);
    assert_eq!(all.title, "3 pictures kept, under 1 MB");

    let t4 = &all.pictures[3];
    assert_eq!(t4.file, T4);
    assert_eq!(t4.shown_file, "teams-frames/removed/teams-20261118-110433-637.jpg");
    assert!(t4.removed);
    assert_eq!(t4.reason, Some(Reason::ByYou));
    assert_eq!(t4.caption, None);
    assert_eq!(t4.repeat_of, None);
    assert_eq!(t4.bytes, 400);
    assert_eq!(t4.delete_after_ms, 432_001_000);
    assert_eq!(t4.time, "11:04:33");
    assert!(all.pictures[..3].iter().all(|p| !p.removed));
}

#[test]
fn all_pictures_lists_a_removed_picture_between_kept_ones_from_the_other_folder() {
    let d = example();
    remove_by_you(d.path(), &[s(Z)], 1000).unwrap();
    let all = all_pictures(d.path());
    assert_eq!(files_of(&all), vec![s(T1), s(Z), s(T3), s(T4)]);
    assert_eq!(all.pictures[1].shown_file, "zoom-frames/removed/zoom-20261118-110310-000.jpg");
    assert!(all.pictures[1].removed);
}

#[test]
fn all_pictures_shows_claudes_reason_caption_and_repeat_of() {
    let d = example();
    let pics = list_pictures(d.path());
    let removals = vec![
        removal("P1", Reason::Popup, Some("Save changes? box"), None),
        removal("P3", Reason::Repeat, Some("Same page"), Some("P4")),
    ];
    apply(d.path(), &pics, &removals, 5000).unwrap();
    let all = all_pictures(d.path());

    let t1 = all.pictures.iter().find(|p| p.file == T1).unwrap();
    assert!(t1.removed);
    assert_eq!(t1.reason, Some(Reason::Popup));
    assert_eq!(t1.caption.as_deref(), Some("Save changes? box"));
    assert_eq!(t1.delete_after_ms, 5000 + FIVE_DAYS);
    assert_eq!(t1.bytes, 200);
    let t3 = all.pictures.iter().find(|p| p.file == T3).unwrap();
    assert_eq!(t3.reason, Some(Reason::Repeat));
    assert_eq!(t3.repeat_of.as_deref(), Some(T4));
    assert_eq!(t3.shown_file, "teams-frames/removed/teams-20261118-110332-532.jpg");
    assert_eq!((all.kept, all.removed), (2, 2));
}

#[test]
fn all_pictures_counts_a_restored_picture_as_kept() {
    let d = example();
    let pics = list_pictures(d.path());
    apply(d.path(), &pics, &[removal("P1", Reason::Blank, None, None)], 5000).unwrap();
    restore(d.path(), &[s(T1)]).unwrap();
    let all = all_pictures(d.path());
    assert_eq!((all.kept, all.removed), (4, 0));
    let t1 = &all.pictures[0];
    assert_eq!(t1.file, T1);
    assert!(!t1.removed);
    assert_eq!(t1.reason, None);
    assert_eq!(t1.delete_after_ms, 0);
    assert_eq!(t1.shown_file, T1);
}

#[test]
fn all_pictures_with_nothing_kept_says_no_pictures_kept() {
    let d = example();
    remove_by_you(d.path(), &[s(T1), s(Z), s(T3), s(T4)], 1000).unwrap();
    let all = all_pictures(d.path());
    assert_eq!(all.title, "No pictures kept");
    assert_eq!((all.kept, all.kept_bytes, all.removed), (0, 0, 4));
    assert_eq!(all.pictures.len(), 4);
    assert!(all.pictures.iter().all(|p| p.removed));
}

#[test]
fn all_pictures_reconciles_with_the_disk_like_load() {
    let d = example();
    remove_by_you(d.path(), &[s(T1), s(T3)], 1000).unwrap();
    fs::remove_file(d.path().join(removed_of(T3))).unwrap(); // gone from both places
    write(d.path(), "teams-frames/removed/teams-20261118-110500-000.jpg", 10); // stray

    let all = all_pictures(d.path());

    assert!(d.path().join("teams-frames/teams-20261118-110500-000.jpg").exists());
    // T3's entry is dropped (gone from both places); the stray is back and kept.
    assert_eq!(
        files_of(&all),
        vec![s(T1), s(Z), s(T4), s("teams-frames/teams-20261118-110500-000.jpg")]
    );
    assert_eq!(all.removed, 1);
    assert!(all.pictures[0].removed);
    assert!(all.pictures[1..].iter().all(|p| !p.removed));
}

// ---------------------------------------------------------------------------
// remove_by_you
// ---------------------------------------------------------------------------

#[test]
fn remove_by_you_worked_example_with_no_record() {
    let d = example();
    let rec = remove_by_you(d.path(), &[s(T4)], 1000).unwrap();

    assert!(!d.path().join(T4).exists());
    let moved = d.path().join("teams-frames/removed/teams-20261118-110433-637.jpg");
    assert_eq!(fs::metadata(&moved).unwrap().len(), 400);
    assert!(d.path().join(T1).exists() && d.path().join(Z).exists() && d.path().join(T3).exists());

    assert_eq!(rec.state, CleanupState::NotRun);
    assert_eq!(rec.ran_at_ms, 0);
    assert_eq!(rec.removed.len(), 1);
    let e = &rec.removed[0];
    assert_eq!(e.file, T4);
    assert_eq!(e.removed_file, "teams-frames/removed/teams-20261118-110433-637.jpg");
    assert_eq!(e.time, "11:04:33");
    assert_eq!(e.reason, Reason::ByYou);
    assert_eq!(e.caption, None);
    assert_eq!(e.repeat_of, None);
    assert_eq!(e.bytes, 400);
    assert!(!e.restored);
    assert_eq!(e.delete_after_ms, 432_001_000);

    // The record is on disk and reads back whole.
    assert_eq!(raw_record(d.path()), rec);
    assert_eq!(load(d.path()), Some(rec.clone()));

    assert!(should_start(Some(&rec), false));
    let sum = summary(Some(&rec), 2000);
    assert_eq!((sum.by_you, sum.removed, sum.bytes), (1, 0, 400));
    assert!(!sum.failed);
    assert_eq!(sum.delete_after_ms, 432_001_000);
    assert_eq!(sum.line.as_deref(), Some("You removed 1 picture to free under 1 MB."));
}

#[test]
fn remove_by_you_removes_several_across_both_folders_each_with_its_own_clock() {
    let d = example();
    let rec = remove_by_you(d.path(), &[s(T1), s(Z)], 7000).unwrap();
    assert_eq!(rec.removed.len(), 2);
    assert!(d.path().join("teams-frames/removed/teams-20261118-110300-478.jpg").exists());
    assert!(d.path().join("zoom-frames/removed/zoom-20261118-110310-000.jpg").exists());
    for f in [T1, Z] {
        let e = entry_of(&rec, f).unwrap();
        assert_eq!(e.reason, Reason::ByYou);
        assert_eq!(e.delete_after_ms, 7000 + FIVE_DAYS);
    }
    assert_eq!(entry_of(&rec, Z).unwrap().time, "11:03:10");
    // A later removal gets its own, later clock; the first keeps its own.
    let rec = remove_by_you(d.path(), &[s(T3)], 9000).unwrap();
    assert_eq!(entry_of(&rec, T1).unwrap().delete_after_ms, 7000 + FIVE_DAYS);
    assert_eq!(entry_of(&rec, T3).unwrap().delete_after_ms, 9000 + FIVE_DAYS);
    assert_eq!(rec.removed.len(), 3);
}

#[test]
fn remove_by_you_skips_a_name_that_is_not_a_kept_picture_and_writes_no_record() {
    let d = example();
    let rec = remove_by_you(d.path(), &[s("teams-frames/nope.jpg")], 1000).unwrap();
    assert!(rec.removed.is_empty());
    assert_eq!(rec.state, CleanupState::NotRun);
    assert!(!d.path().join(RECORD_FILE).exists());
    assert_eq!(list_pictures(d.path()).len(), 4);
    assert!(!d.path().join("teams-frames/removed").exists());
}

#[test]
fn remove_by_you_skips_a_picture_missing_from_disk_and_removes_the_rest() {
    let d = example();
    fs::remove_file(d.path().join(T1)).unwrap();
    let rec = remove_by_you(d.path(), &[s(T1), s(T4)], 1000).unwrap();
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].file, T4);
    assert!(!d.path().join("teams-frames/removed/teams-20261118-110300-478.jpg").exists());
}

#[test]
fn remove_by_you_with_an_empty_list_removes_nothing_and_writes_no_record() {
    let d = example();
    let rec = remove_by_you(d.path(), &[], 1000).unwrap();
    assert_eq!(rec.state, CleanupState::NotRun);
    assert!(rec.removed.is_empty());
    assert!(!d.path().join(RECORD_FILE).exists());
    assert_eq!(list_pictures(d.path()).len(), 4);
}

#[test]
fn remove_by_you_with_an_empty_list_leaves_an_existing_record_as_it_is() {
    let d = example();
    let before = remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let after = remove_by_you(d.path(), &[], 2000).unwrap();
    assert_eq!(after, before);
    assert_eq!(raw_record(d.path()), before);
}

#[test]
fn remove_by_you_ignores_a_picture_that_is_already_removed() {
    // T1 is in removed/ (Claude's, pending): not a kept picture, so skipped,
    // and the entry stays exactly as it was.
    let d = example();
    let pics = list_pictures(d.path());
    let first = apply(d.path(), &pics, &[removal("P1", Reason::Popup, Some("cap"), None)], 5000).unwrap();
    let after = remove_by_you(d.path(), &[s(T1)], 9000).unwrap();
    assert_eq!(after, first);
    assert!(d.path().join("teams-frames/removed/teams-20261118-110300-478.jpg").exists());
}

#[test]
fn remove_by_you_keeps_a_done_record_state_and_claudes_entries() {
    let d = example();
    let pics = list_pictures(d.path());
    apply(d.path(), &pics, &[removal("P1", Reason::BeforeShare, None, None)], 5000).unwrap();
    let rec = remove_by_you(d.path(), &[s(T4)], 6000).unwrap();
    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.ran_at_ms, 5000);
    assert_eq!(rec.removed.len(), 2);
    assert_eq!(entry_of(&rec, T1).unwrap().reason, Reason::BeforeShare);
    assert_eq!(entry_of(&rec, T1).unwrap().delete_after_ms, 5000 + FIVE_DAYS);
    assert_eq!(entry_of(&rec, T4).unwrap().reason, Reason::ByYou);
    assert_eq!(entry_of(&rec, T4).unwrap().delete_after_ms, 6000 + FIVE_DAYS);
    assert_eq!(load(d.path()), Some(rec));
}

#[test]
fn remove_by_you_keeps_a_failed_record_state() {
    let d = example();
    record_failure(d.path(), "boom", 2000).unwrap();
    let rec = remove_by_you(d.path(), &[s(T4)], 3000).unwrap();
    assert_eq!(rec.state, CleanupState::Failed { error: s("boom") });
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].reason, Reason::ByYou);
    assert_eq!(load(d.path()).unwrap().state, CleanupState::Failed { error: s("boom") });
}

#[test]
fn remove_by_you_replaces_an_earlier_entry_for_the_same_file() {
    // Claude removed T1 (Done, clock 432_005_000), Joseph restored it, then removes it himself.
    let d = example();
    let pics = list_pictures(d.path());
    let claude = vec![removal("P1", Reason::Popup, Some("Save changes? box"), None)];
    apply(d.path(), &pics, &claude, 5000).unwrap();
    restore(d.path(), &[s(T1)]).unwrap();

    let rec = remove_by_you(d.path(), &[s(T1)], 300_000_000).unwrap();

    let for_t1: Vec<&RemovedPicture> = rec.removed.iter().filter(|m| m.file == T1).collect();
    assert_eq!(for_t1.len(), 1);
    let e = for_t1[0];
    assert_eq!(e.reason, Reason::ByYou);
    assert_eq!(e.caption, None);
    assert_eq!(e.repeat_of, None);
    assert!(!e.restored);
    assert_eq!(e.delete_after_ms, 732_000_000);
    assert!(d.path().join("teams-frames/removed/teams-20261118-110300-478.jpg").exists());
    assert!(!d.path().join(T1).exists());
    let sum = summary(Some(&rec), 300_000_001);
    assert_eq!((sum.removed, sum.by_you), (0, 1));
    assert_eq!(load(d.path()), Some(rec));
}

#[test]
fn remove_by_you_when_one_move_fails_keeps_the_ones_that_moved() {
    let d = example();
    write(d.path(), "zoom-frames/removed", 1); // a plain file: moving Z into it fails
    let rec = remove_by_you(d.path(), &[s(Z), s(T4)], 1000).unwrap();
    assert!(d.path().join(Z).exists());
    assert!(d.path().join("teams-frames/removed/teams-20261118-110433-637.jpg").exists());
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].file, T4);
    assert_eq!(load(d.path()), Some(rec));
}

#[test]
fn remove_by_you_when_the_record_cannot_be_written_puts_everything_back_and_is_err() {
    let d = example();
    // A directory where the record file belongs: reading gives no record, and
    // the rename of the new record onto it fails.
    fs::create_dir(d.path().join(RECORD_FILE)).unwrap();
    let r = remove_by_you(d.path(), &[s(T1), s(T4)], 1000);
    assert!(r.is_err());
    for f in [T1, Z, T3, T4] {
        assert!(d.path().join(f).exists(), "{} should be back", f);
    }
    assert!(!d.path().join(removed_of(T1)).exists());
    assert!(!d.path().join(removed_of(T4)).exists());
}

#[cfg(unix)]
#[test]
fn remove_by_you_unwritable_record_with_an_earlier_entry_puts_back_only_this_calls_pictures() {
    use std::os::unix::fs::PermissionsExt;
    let d = example();
    remove_by_you(d.path(), &[s(T1)], 1000).unwrap(); // an earlier entry, stays put
    // A read-only meeting folder: the temporary record file cannot be created.
    fs::set_permissions(d.path(), fs::Permissions::from_mode(0o555)).unwrap();
    let r = remove_by_you(d.path(), &[s(T4)], 2000);
    fs::set_permissions(d.path(), fs::Permissions::from_mode(0o755)).unwrap();

    assert!(r.is_err());
    assert!(d.path().join(T4).exists(), "this call's picture goes back");
    assert!(!d.path().join(removed_of(T4)).exists());
    assert!(d.path().join(removed_of(T1)).exists(), "the earlier removal stays removed");
    let rec = load(d.path()).unwrap();
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].file, T1);
}

#[test]
fn remove_by_you_from_two_threads_records_both() {
    for round in 0..50 {
        let d = example();
        let folder = d.path().to_path_buf();
        let gate = Arc::new(Barrier::new(2));
        std::thread::scope(|sc| {
            for f in [T1, T4] {
                let (gate, folder) = (gate.clone(), folder.clone());
                sc.spawn(move || {
                    gate.wait();
                    remove_by_you(&folder, &[s(f)], 1000).unwrap();
                });
            }
        });
        let rec = raw_record(d.path());
        assert!(entry_of(&rec, T1).is_some(), "round {}: T1 lost", round);
        assert!(entry_of(&rec, T4).is_some(), "round {}: T4 lost", round);
        assert_eq!(
            names(d.path()),
            vec![s(RECORD_FILE), s("teams-frames"), s("zoom-frames")],
            "round {}: stray temporary file",
            round
        );
    }
}

#[test]
fn claudes_reply_can_never_name_by_you() {
    let pics = vec![
        Picture { id: s("P1"), file: s(T1), time: s("11:03:00") },
        Picture { id: s("P2"), file: s(Z), time: s("11:03:10") },
    ];
    let raw = r#"{"remove":[{"id":"P1","reason":"by_you"},{"id":"P2","reason":"blank"}]}"#;
    assert_eq!(parse_reply(raw, &pics).unwrap(), vec![removal("P2", Reason::Blank, None, None)]);
}

#[test]
fn by_you_is_written_to_the_record_as_by_you() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let text = fs::read_to_string(d.path().join(RECORD_FILE)).unwrap();
    assert!(text.contains("\"by_you\""), "record: {}", text);
}

// ---------------------------------------------------------------------------
// apply with Joseph's removals
// ---------------------------------------------------------------------------

#[test]
fn apply_after_remove_by_you_merges_and_the_worked_example_numbers_hold() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    // list_pictures now gives T1 P1, Z P2, T3 P3.
    let pics = list_pictures(d.path());
    assert_eq!(pics.len(), 3);
    let rec = apply(d.path(), &pics, &[removal("P1", Reason::BeforeShare, None, None)], 5000).unwrap();

    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.ran_at_ms, 5000);
    assert_eq!(rec.removed.len(), 2);
    let t4 = entry_of(&rec, T4).unwrap();
    assert_eq!(t4.reason, Reason::ByYou);
    assert_eq!(t4.delete_after_ms, 432_001_000);
    let t1 = entry_of(&rec, T1).unwrap();
    assert_eq!(t1.reason, Reason::BeforeShare);
    assert_eq!(t1.delete_after_ms, 432_005_000);
    assert_eq!(load(d.path()), Some(rec.clone()));

    let sum = summary(Some(&rec), 6000);
    assert_eq!(
        sum.line.as_deref(),
        Some("Claude removed 1 picture to free under 1 MB: 1 not useful. You removed 1.")
    );
    assert_eq!(sum.bytes, 600);
    assert_eq!(sum.delete_after_ms, 432_001_000);
    assert_eq!((sum.removed, sum.not_useful, sum.repeats, sum.by_you), (1, 1, 0, 1));
}

#[test]
fn purge_after_the_merge_deletes_only_josephs_picture_when_its_clock_ends() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let pics = list_pictures(d.path());
    apply(d.path(), &pics, &[removal("P1", Reason::BeforeShare, None, None)], 5000).unwrap();

    assert!(purge_if_expired(d.path(), 432_001_000));

    assert!(!d.path().join(removed_of(T4)).exists());
    assert!(!d.path().join(T4).exists());
    assert!(d.path().join(removed_of(T1)).exists(), "Claude's picture stays in removed/");
    let rec = load(d.path()).unwrap();
    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].file, T1);
    assert_eq!(
        summary(Some(&rec), 432_001_000).line.as_deref(),
        Some("Claude removed 1 picture to free under 1 MB: 1 not useful.")
    );
}

#[test]
fn apply_while_josephs_removal_happened_in_flight_skips_it_and_keeps_his_entry() {
    let d = example();
    let pics = list_pictures(d.path()); // T1..T4 as P1..P4, before Joseph removes
    remove_by_you(d.path(), &[s(T3)], 2000).unwrap();
    let removals = vec![
        removal("P3", Reason::Repeat, None, Some("P4")),
        removal("P1", Reason::Blank, None, None),
    ];
    let rec = apply(d.path(), &pics, &removals, 5000).unwrap();

    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.removed.len(), 2);
    let t3 = entry_of(&rec, T3).unwrap();
    assert_eq!(t3.reason, Reason::ByYou, "T3 stays Joseph's");
    assert_eq!(t3.repeat_of, None);
    assert_eq!(t3.delete_after_ms, 2000 + FIVE_DAYS);
    let t1 = entry_of(&rec, T1).unwrap();
    assert_eq!(t1.reason, Reason::Blank);
    assert_eq!(t1.delete_after_ms, 5000 + FIVE_DAYS);
    assert!(d.path().join(T4).exists());
    assert!(d.path().join(removed_of(T3)).exists());
    assert!(d.path().join(removed_of(T1)).exists());
}

#[test]
fn apply_on_a_failed_record_keeps_josephs_entries_and_becomes_done() {
    // Try again after a failure that left Joseph's removal in the record.
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    record_failure(d.path(), "boom", 2000).unwrap();
    let pics = list_pictures(d.path());
    let rec = apply(d.path(), &pics, &[removal("P1", Reason::Blank, None, None)], 5000).unwrap();
    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.ran_at_ms, 5000);
    assert_eq!(rec.removed.len(), 2);
    assert_eq!(entry_of(&rec, T4).unwrap().reason, Reason::ByYou);
    assert_eq!(entry_of(&rec, T4).unwrap().delete_after_ms, 432_001_000);
}

#[cfg(unix)]
#[test]
fn apply_unwritable_record_puts_back_only_the_pictures_this_run_moved() {
    use std::os::unix::fs::PermissionsExt;
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let pics = list_pictures(d.path());
    fs::set_permissions(d.path(), fs::Permissions::from_mode(0o555)).unwrap();
    let r = apply(d.path(), &pics, &[removal("P1", Reason::Blank, None, None)], 5000);
    fs::set_permissions(d.path(), fs::Permissions::from_mode(0o755)).unwrap();

    assert!(r.is_err());
    assert!(d.path().join(T1).exists(), "the run's picture goes back");
    assert!(!d.path().join(removed_of(T1)).exists());
    assert!(d.path().join(removed_of(T4)).exists(), "Joseph's removal stays removed");
    let rec = load(d.path()).unwrap();
    assert_eq!(rec.state, CleanupState::NotRun);
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].file, T4);
}

// ---------------------------------------------------------------------------
// record_failure
// ---------------------------------------------------------------------------

#[test]
fn record_failure_keeps_josephs_entry_and_the_worked_example_holds() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let rec = record_failure(d.path(), "boom", 2000).unwrap();

    assert_eq!(rec.state, CleanupState::Failed { error: s("boom") });
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].file, T4);
    assert_eq!(rec.removed[0].reason, Reason::ByYou);
    assert!(d.path().join(removed_of(T4)).exists());
    assert_eq!(load(d.path()), Some(rec.clone()));

    let sum = summary(Some(&rec), 3000);
    assert!(sum.failed);
    assert_eq!(sum.by_you, 1);
    assert_eq!(sum.line.as_deref(), Some("Picture cleanup did not finish."));
    assert!(should_start(Some(&rec), false));
    assert!(!should_start(Some(&rec), true));

    // Joseph's picture purges on its own clock even though the run failed.
    assert!(purge_if_expired(d.path(), 432_001_000));
    assert!(!d.path().join(removed_of(T4)).exists());
    let after = load(d.path()).unwrap();
    assert_eq!(after.state, CleanupState::Failed { error: s("boom") });
    assert!(after.removed.is_empty());
}

#[test]
fn record_failure_after_a_done_run_keeps_every_entry() {
    let d = example();
    let pics = list_pictures(d.path());
    let done = apply(d.path(), &pics, &[removal("P1", Reason::Blank, None, None)], 5000).unwrap();
    let rec = record_failure(d.path(), "again", 6000).unwrap();
    assert_eq!(rec.state, CleanupState::Failed { error: s("again") });
    assert_eq!(rec.removed, done.removed);
    assert!(d.path().join(removed_of(T1)).exists());
}

#[test]
fn record_failure_fields_not_touching_entries_leave_the_pictures_alone() {
    let d = example();
    record_failure(d.path(), "boom", 2000).unwrap();
    assert_eq!(list_pictures(d.path()).len(), 4);
}

// ---------------------------------------------------------------------------
// should_start
// ---------------------------------------------------------------------------

#[test]
fn should_start_covers_every_state() {
    assert!(should_start(None, false));
    assert!(!should_start(None, true));
    let not_run = rec_of(CleanupState::NotRun, 0, vec![rp(T4, Reason::ByYou, 400, false, 1)]);
    assert!(should_start(Some(&not_run), false));
    assert!(!should_start(Some(&not_run), true));
    let failed = rec_of(CleanupState::Failed { error: s("x") }, 5, vec![]);
    assert!(should_start(Some(&failed), false));
    assert!(!should_start(Some(&failed), true));
    let done = rec_of(CleanupState::Done, 5, vec![]);
    assert!(!should_start(Some(&done), false));
    assert!(!should_start(Some(&done), true));
}

// ---------------------------------------------------------------------------
// restore and remove_again
// ---------------------------------------------------------------------------

#[test]
fn restore_works_on_a_not_run_record_and_keeps_its_state() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let rec = restore(d.path(), &[s(T4)]).unwrap();
    assert!(d.path().join(T4).exists());
    assert!(!d.path().join(removed_of(T4)).exists());
    assert_eq!(rec.state, CleanupState::NotRun);
    assert!(entry_of(&rec, T4).unwrap().restored);
    assert_eq!(load(d.path()), Some(rec.clone()));
    let sum = summary(Some(&rec), 2000);
    assert_eq!((sum.by_you, sum.bytes, sum.delete_after_ms), (0, 0, 0));
    assert_eq!(sum.line, None);
}

#[test]
fn undo_of_a_remove_is_restore_of_that_file_and_every_picture_is_kept_again() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    restore(d.path(), &[s(T4)]).unwrap();
    let all = all_pictures(d.path());
    assert_eq!((all.kept, all.removed), (4, 0));
    assert_eq!(all.title, "4 pictures kept, under 1 MB");
}

#[test]
fn restore_works_on_a_failed_record_that_has_entries() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    record_failure(d.path(), "boom", 2000).unwrap();
    let rec = restore(d.path(), &[s(T4)]).unwrap();
    assert!(d.path().join(T4).exists());
    assert_eq!(rec.state, CleanupState::Failed { error: s("boom") });
    assert!(entry_of(&rec, T4).unwrap().restored);
}

#[test]
fn restore_with_an_empty_list_restores_claudes_and_josephs_pending_pictures() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let pics = list_pictures(d.path());
    apply(d.path(), &pics, &[removal("P1", Reason::Blank, None, None)], 5000).unwrap();
    let rec = restore(d.path(), &[]).unwrap();
    assert!(rec.removed.iter().all(|m| m.restored));
    assert_eq!(rec.state, CleanupState::Done);
    for f in [T1, Z, T3, T4] {
        assert!(d.path().join(f).exists());
    }
    let sum = summary(Some(&rec), 6000);
    assert_eq!((sum.removed, sum.by_you, sum.bytes, sum.delete_after_ms), (0, 0, 0, 0));
    assert_eq!(sum.line, None);
}

#[test]
fn restore_of_one_by_you_picture_leaves_claudes_pending() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let pics = list_pictures(d.path());
    apply(d.path(), &pics, &[removal("P1", Reason::Blank, None, None)], 5000).unwrap();
    let rec = restore(d.path(), &[s(T4)]).unwrap();
    assert!(entry_of(&rec, T4).unwrap().restored);
    assert!(!entry_of(&rec, T1).unwrap().restored);
    assert_eq!(
        summary(Some(&rec), 6000).line.as_deref(),
        Some("Claude removed 1 picture to free under 1 MB: 1 not useful.")
    );
}

#[test]
fn restore_with_no_record_is_err() {
    let d = example();
    assert!(restore(d.path(), &[]).is_err());
    assert!(restore(d.path(), &[s(T1)]).is_err());
    assert!(!d.path().join(RECORD_FILE).exists());
}

#[test]
fn restore_with_a_record_that_has_no_entries_is_err() {
    let d = example();
    record_failure(d.path(), "boom", 2000).unwrap();
    assert!(restore(d.path(), &[]).is_err());
    let d2 = example();
    let pics = list_pictures(d2.path());
    apply(d2.path(), &pics, &[], 5000).unwrap();
    assert!(restore(d2.path(), &[]).is_err());
}

#[test]
fn restore_with_only_restored_entries_is_ok_and_changes_nothing() {
    let d = example();
    let pics = list_pictures(d.path());
    apply(d.path(), &pics, &[removal("P1", Reason::Blank, None, None)], 5000).unwrap();
    let all_back = restore(d.path(), &[]).unwrap();
    let again = restore(d.path(), &[]).unwrap();
    assert_eq!(again, all_back);
    assert_eq!(load(d.path()), Some(all_back));
    assert_eq!(list_pictures(d.path()).len(), 4);
}

#[test]
fn remove_again_puts_a_claude_removal_back_exactly_as_it_was() {
    let d = example();
    let pics = list_pictures(d.path());
    let claude = vec![removal("P1", Reason::Popup, Some("Save changes? box"), None)];
    let before = apply(d.path(), &pics, &claude, 5000).unwrap();
    let original = entry_of(&before, T1).unwrap().clone();
    assert_eq!(original.delete_after_ms, 432_005_000);

    restore(d.path(), &[s(T1)]).unwrap();
    let rec = remove_again(d.path(), &[s(T1)]).unwrap();

    let e = entry_of(&rec, T1).unwrap();
    assert_eq!(e, &original);
    assert_eq!(e.reason, Reason::Popup);
    assert_eq!(e.caption.as_deref(), Some("Save changes? box"));
    assert_eq!(e.delete_after_ms, 432_005_000);
    assert!(!e.restored);
    assert!(d.path().join(removed_of(T1)).exists());
    assert!(!d.path().join(T1).exists());
}

#[test]
fn remove_again_keeps_repeat_of() {
    let d = example();
    let pics = list_pictures(d.path());
    let before = apply(d.path(), &pics, &[removal("P3", Reason::Repeat, None, Some("P4"))], 5000).unwrap();
    let original = entry_of(&before, T3).unwrap().clone();
    assert_eq!(original.repeat_of.as_deref(), Some(T4));
    restore(d.path(), &[s(T3)]).unwrap();
    let rec = remove_again(d.path(), &[s(T3)]).unwrap();
    assert_eq!(entry_of(&rec, T3).unwrap(), &original);
}

#[test]
fn remove_again_works_on_a_not_run_record_and_keeps_josephs_clock() {
    let d = example();
    let first = remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    restore(d.path(), &[s(T4)]).unwrap();
    let rec = remove_again(d.path(), &[s(T4)]).unwrap();
    assert_eq!(rec.state, CleanupState::NotRun);
    assert_eq!(entry_of(&rec, T4).unwrap(), entry_of(&first, T4).unwrap());
    assert_eq!(entry_of(&rec, T4).unwrap().delete_after_ms, 432_001_000);
    assert!(d.path().join(removed_of(T4)).exists());
}

#[test]
fn remove_again_works_on_a_failed_record_with_entries() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    record_failure(d.path(), "boom", 2000).unwrap();
    restore(d.path(), &[s(T4)]).unwrap();
    let rec = remove_again(d.path(), &[s(T4)]).unwrap();
    assert_eq!(rec.state, CleanupState::Failed { error: s("boom") });
    assert!(!entry_of(&rec, T4).unwrap().restored);
    assert!(d.path().join(removed_of(T4)).exists());
}

#[test]
fn restore_then_remove_by_you_leaves_one_by_you_entry_and_no_claude_count() {
    // The brief's "Restore and remove again" example.
    let d = example();
    let pics = list_pictures(d.path());
    apply(d.path(), &pics, &[removal("P1", Reason::BeforeShare, None, None)], 5000).unwrap();
    restore(d.path(), &[s(T1)]).unwrap();
    let rec = remove_by_you(d.path(), &[s(T1)], 300_000_000).unwrap();
    assert_eq!(rec.removed.iter().filter(|m| m.file == T1).count(), 1);
    let e = entry_of(&rec, T1).unwrap();
    assert_eq!(e.reason, Reason::ByYou);
    assert_eq!(e.delete_after_ms, 732_000_000);
    let sum = summary(Some(&rec), 300_000_001);
    assert_eq!(sum.removed, 0);
    assert_eq!(sum.by_you, 1);
}

// ---------------------------------------------------------------------------
// load: PR #30 records
// ---------------------------------------------------------------------------

fn pr30_entry(file: &str, bytes: u64, restored: bool) -> String {
    let (dir, name) = file.rsplit_once('/').unwrap();
    let time = {
        let hms = name.split('-').nth(2).unwrap();
        format!("{}:{}:{}", &hms[0..2], &hms[2..4], &hms[4..6])
    };
    format!(
        r#"{{"file":"{file}","removedFile":"{dir}/removed/{name}","time":"{time}","reason":"before_share","caption":null,"repeatOf":null,"bytes":{bytes},"restored":{restored}}}"#
    )
}

#[test]
fn a_pr30_done_record_loads_as_done_and_each_entry_takes_the_old_record_date() {
    let d = example();
    write(d.path(), &removed_of(T1), 200);
    fs::remove_file(d.path().join(T1)).unwrap();
    write(d.path(), &removed_of(Z), 100);
    fs::remove_file(d.path().join(Z)).unwrap();
    let json = format!(
        r#"{{"state":"done","ranAtMs":1000,"deleteAfterMs":432001000,"removed":[{},{}]}}"#,
        pr30_entry(T1, 200, false),
        pr30_entry(Z, 100, false)
    );
    fs::write(d.path().join(RECORD_FILE), json).unwrap();

    let rec = load(d.path()).unwrap();

    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.ran_at_ms, 1000);
    assert_eq!(rec.removed.len(), 2);
    for f in [T1, Z] {
        let e = entry_of(&rec, f).unwrap();
        assert_eq!(e.delete_after_ms, 432_001_000);
        assert_eq!(e.reason, Reason::BeforeShare);
        assert!(!e.restored);
    }
    let sum = summary(Some(&rec), 2000);
    assert_eq!((sum.removed, sum.bytes, sum.delete_after_ms), (2, 300, 432_001_000));
    // Loading again gives the same thing (whatever the builder rewrote).
    assert_eq!(load(d.path()), Some(rec));
}

#[test]
fn a_pr30_record_lists_in_all_pictures_with_the_old_date_and_purges_on_it() {
    let d = example();
    write(d.path(), &removed_of(T1), 200);
    fs::remove_file(d.path().join(T1)).unwrap();
    let json = format!(
        r#"{{"state":"done","ranAtMs":1000,"deleteAfterMs":432001000,"removed":[{}]}}"#,
        pr30_entry(T1, 200, false)
    );
    fs::write(d.path().join(RECORD_FILE), json).unwrap();

    let rec = load(d.path()).unwrap();
    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(entry_of(&rec, T1).unwrap().delete_after_ms, 432_001_000);

    let all = all_pictures(d.path());
    let t1 = all.pictures.iter().find(|p| p.file == T1).unwrap();
    assert!(t1.removed);
    assert_eq!(t1.reason, Some(Reason::BeforeShare));
    assert_eq!(t1.delete_after_ms, 432_001_000);
    assert_eq!(t1.shown_file, removed_of(T1));
    assert_eq!((all.kept, all.removed), (3, 1));

    assert!(!purge_if_expired(d.path(), 432_000_999));
    assert!(purge_if_expired(d.path(), 432_001_000));
    assert!(!d.path().join(removed_of(T1)).exists());
}

#[test]
fn a_pr30_purged_record_loads_as_done_with_an_empty_list() {
    let d = example();
    let json = r#"{"state":"purged","ranAtMs":1000,"deleteAfterMs":432001000,"removed":[]}"#;
    fs::write(d.path().join(RECORD_FILE), json).unwrap();
    let rec = load(d.path()).unwrap();
    assert_eq!(rec.state, CleanupState::Done);
    assert!(rec.removed.is_empty());
    assert_eq!(rec.ran_at_ms, 1000);
    assert!(!should_start(Some(&rec), false));
    let sum = summary(Some(&rec), 999_999_999_999);
    assert_eq!(sum.line, None);
    assert!(!sum.failed);
}

#[test]
fn a_pr30_purged_record_that_still_lists_deleted_pictures_loads_as_done_with_an_empty_list() {
    let d = example(); // T1 is deleted for good: in neither place after the purge
    fs::remove_file(d.path().join(T1)).unwrap();
    let json = format!(
        r#"{{"state":"purged","ranAtMs":1000,"deleteAfterMs":432001000,"removed":[{}]}}"#,
        pr30_entry(T1, 200, false)
    );
    fs::write(d.path().join(RECORD_FILE), json).unwrap();
    let rec = load(d.path()).unwrap();
    assert_eq!(rec.state, CleanupState::Done);
    assert!(rec.removed.is_empty());
    assert_eq!(all_pictures(d.path()).removed, 0);
}

#[test]
fn a_pr30_failed_record_loads_as_failed() {
    let d = example();
    let json = r#"{"state":"failed","error":"boom","ranAtMs":7,"deleteAfterMs":0,"removed":[]}"#;
    fs::write(d.path().join(RECORD_FILE), json).unwrap();
    let rec = load(d.path()).unwrap();
    assert_eq!(rec.state, CleanupState::Failed { error: s("boom") });
    assert_eq!(rec.ran_at_ms, 7);
    assert!(rec.removed.is_empty());
    assert!(should_start(Some(&rec), false));
    assert_eq!(summary(Some(&rec), 8).line.as_deref(), Some("Picture cleanup did not finish."));
}

#[test]
fn a_pr30_record_takes_josephs_removals_afterwards_without_losing_anything() {
    let d = example();
    write(d.path(), &removed_of(T1), 200);
    fs::remove_file(d.path().join(T1)).unwrap();
    let json = format!(
        r#"{{"state":"done","ranAtMs":1000,"deleteAfterMs":432001000,"removed":[{}]}}"#,
        pr30_entry(T1, 200, false)
    );
    fs::write(d.path().join(RECORD_FILE), json).unwrap();
    let rec = remove_by_you(d.path(), &[s(T4)], 2000).unwrap();
    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(entry_of(&rec, T1).unwrap().delete_after_ms, 432_001_000);
    assert_eq!(entry_of(&rec, T4).unwrap().delete_after_ms, 2000 + FIVE_DAYS);
}

// ---------------------------------------------------------------------------
// purge_if_expired
// ---------------------------------------------------------------------------

#[test]
fn purge_deletes_only_the_pending_pictures_past_their_date() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap(); // clock 432_001_000
    remove_by_you(d.path(), &[s(T1)], 5000).unwrap(); // clock 432_005_000

    assert!(!purge_if_expired(d.path(), 432_000_999));
    assert!(purge_if_expired(d.path(), 432_001_000));

    assert!(!d.path().join(removed_of(T4)).exists());
    assert!(d.path().join(removed_of(T1)).exists());
    let rec = load(d.path()).unwrap();
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].file, T1);
    // The rest stays restorable.
    restore(d.path(), &[s(T1)]).unwrap();
    assert!(d.path().join(T1).exists());
}

#[test]
fn purge_never_changes_the_state_of_any_record() {
    // NotRun stays NotRun.
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    assert!(purge_if_expired(d.path(), 432_001_000));
    let rec = load(d.path()).unwrap();
    assert_eq!(rec.state, CleanupState::NotRun);
    assert!(rec.removed.is_empty());
    assert_eq!(rec.ran_at_ms, 0);

    // Done stays Done.
    let d = example();
    let pics = list_pictures(d.path());
    apply(d.path(), &pics, &[removal("P1", Reason::Blank, None, None)], 1000).unwrap();
    assert!(purge_if_expired(d.path(), 432_001_000));
    assert_eq!(load(d.path()).unwrap().state, CleanupState::Done);
}

#[test]
fn purge_drops_an_expired_restored_entry_but_keeps_its_picture() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    restore(d.path(), &[s(T4)]).unwrap();

    // No file is deleted, so the answer is false; the entry is dropped.
    assert!(!purge_if_expired(d.path(), 432_001_000));
    assert!(d.path().join(T4).exists(), "a restored picture is a kept picture");
    let rec = load(d.path()).unwrap();
    assert!(rec.removed.is_empty());
    assert_eq!(all_pictures(d.path()).kept, 4);
}

#[test]
fn purge_keeps_a_restored_entry_that_is_not_yet_due() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    restore(d.path(), &[s(T4)]).unwrap();
    assert!(!purge_if_expired(d.path(), 432_000_999));
    let rec = load(d.path()).unwrap();
    assert_eq!(rec.removed.len(), 1);
    assert!(rec.removed[0].restored);
}

#[test]
fn purge_returns_true_when_it_deleted_at_least_one_file_even_if_others_were_only_dropped() {
    let d = example();
    remove_by_you(d.path(), &[s(T4), s(T1)], 1000).unwrap();
    restore(d.path(), &[s(T1)]).unwrap();
    assert!(purge_if_expired(d.path(), 432_001_000));
    assert!(!d.path().join(removed_of(T4)).exists());
    assert!(d.path().join(T1).exists());
    assert!(load(d.path()).unwrap().removed.is_empty());
}

#[test]
fn purge_with_nothing_pending_past_its_date_does_nothing() {
    let d = example();
    let first = remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    assert!(!purge_if_expired(d.path(), 0));
    assert_eq!(load(d.path()), Some(first));
}

#[test]
fn purge_on_a_failed_record_deletes_expired_pictures_and_keeps_the_state() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    record_failure(d.path(), "boom", 2000).unwrap();
    assert!(purge_if_expired(d.path(), 432_001_000));
    assert_eq!(load(d.path()).unwrap().state, CleanupState::Failed { error: s("boom") });
}

#[cfg(unix)]
#[test]
fn a_delete_that_fails_keeps_that_entry_and_the_next_sweep_tries_again() {
    use std::os::unix::fs::PermissionsExt;
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let removed_dir = d.path().join("teams-frames/removed");
    fs::set_permissions(&removed_dir, fs::Permissions::from_mode(0o555)).unwrap();
    let first = purge_if_expired(d.path(), 432_001_000);
    fs::set_permissions(&removed_dir, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(!first);
    assert!(d.path().join(removed_of(T4)).exists());
    let rec = load(d.path()).unwrap();
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].file, T4);

    assert!(purge_if_expired(d.path(), 432_001_001));
    assert!(!d.path().join(removed_of(T4)).exists());
    assert!(load(d.path()).unwrap().removed.is_empty());
}

#[test]
fn removed_on_day_4_is_restorable_on_day_8_and_deleted_on_day_9() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 4 * DAY_MS).unwrap();
    assert!(!purge_if_expired(d.path(), 8 * DAY_MS));
    assert!(d.path().join(removed_of(T4)).exists());
    restore(d.path(), &[s(T4)]).unwrap();
    assert!(d.path().join(T4).exists());

    let d2 = example();
    remove_by_you(d2.path(), &[s(T4)], 4 * DAY_MS).unwrap();
    assert!(purge_if_expired(d2.path(), 9 * DAY_MS));
    assert!(!d2.path().join(removed_of(T4)).exists());
    assert!(!d2.path().join(T4).exists());
}

// ---------------------------------------------------------------------------
// summary
// ---------------------------------------------------------------------------

#[test]
fn summary_not_run_with_josephs_pictures_only() {
    // 4 pictures, 2 MB (2,097,152 bytes) in all.
    let removed = vec![
        rp(T1, Reason::ByYou, 1_048_576, false, 500),
        rp(Z, Reason::ByYou, 524_288, false, 300),
        rp(T3, Reason::ByYou, 262_144, false, 900),
        rp(T4, Reason::ByYou, 262_144, false, 700),
    ];
    let sum = summary(Some(&rec_of(CleanupState::NotRun, 0, removed)), 100);
    assert_eq!((sum.removed, sum.repeats, sum.not_useful, sum.by_you), (0, 0, 0, 4));
    assert_eq!(sum.bytes, 2_097_152);
    assert!(!sum.failed);
    assert_eq!(sum.delete_after_ms, 300, "the soonest pending date");
    assert_eq!(sum.line.as_deref(), Some("You removed 4 pictures to free 2 MB."));
}

#[test]
fn summary_done_with_both_the_worked_93_and_4() {
    let mut removed = Vec::new();
    for i in 0..56 {
        let mut e = rp(&format!("teams-frames/r{i}.jpg"), Reason::Repeat, if i == 0 { 53_477_376 } else { 0 }, false, 9000);
        e.repeat_of = Some(s(T4));
        removed.push(e);
    }
    for i in 0..37 {
        removed.push(rp(&format!("zoom-frames/n{i}.jpg"), Reason::Loading, 0, false, 9000));
    }
    for i in 0..4 {
        removed.push(rp(&format!("teams-frames/y{i}.jpg"), Reason::ByYou, if i == 0 { 2_097_152 } else { 0 }, false, 8000));
    }
    let sum = summary(Some(&rec_of(CleanupState::Done, 1000, removed)), 100);
    assert_eq!((sum.removed, sum.repeats, sum.not_useful, sum.by_you), (93, 56, 37, 4));
    // bytes counts every pending entry; the line uses only Claude's.
    assert_eq!(sum.bytes, 53_477_376 + 2_097_152);
    assert_eq!(sum.delete_after_ms, 8000);
    assert_eq!(
        sum.line.as_deref(),
        Some("Claude removed 93 pictures to free 51 MB: 56 repeats, 37 not useful. You removed 4.")
    );
}

#[test]
fn summary_done_with_claudes_all_restored_and_josephs_pending_says_you_removed() {
    let removed = vec![
        rp(T1, Reason::Blank, 200, true, 432_005_000),
        rp(Z, Reason::Repeat, 100, true, 432_005_000),
        rp(T3, Reason::ByYou, 2_097_152, false, 432_006_000),
    ];
    let sum = summary(Some(&rec_of(CleanupState::Done, 5000, removed)), 6000);
    assert_eq!((sum.removed, sum.by_you, sum.bytes), (0, 1, 2_097_152));
    assert_eq!(sum.delete_after_ms, 432_006_000);
    assert_eq!(sum.line.as_deref(), Some("You removed 1 picture to free 2 MB."));
}

#[test]
fn summary_done_with_claudes_removals_and_none_by_joseph_is_as_before() {
    let removed = vec![rp(T1, Reason::Repeat, 10, false, 77), rp(Z, Reason::Popup, 10, false, 66)];
    let sum = summary(Some(&rec_of(CleanupState::Done, 5000, removed)), 6000);
    assert_eq!(sum.by_you, 0);
    assert_eq!(sum.delete_after_ms, 66);
    assert_eq!(
        sum.line.as_deref(),
        Some("Claude removed 2 pictures to free under 1 MB: 1 repeat, 1 not useful.")
    );
}

#[test]
fn summary_by_you_is_not_counted_as_not_useful() {
    let removed = vec![rp(T1, Reason::ByYou, 10, false, 77), rp(Z, Reason::Blank, 10, false, 77)];
    let sum = summary(Some(&rec_of(CleanupState::Done, 5000, removed)), 6000);
    assert_eq!((sum.removed, sum.not_useful, sum.by_you), (1, 1, 1));
}

#[test]
fn summary_leaves_out_josephs_restored_pictures() {
    let removed = vec![rp(T1, Reason::ByYou, 100, true, 77), rp(Z, Reason::ByYou, 300, false, 88)];
    let sum = summary(Some(&rec_of(CleanupState::NotRun, 0, removed)), 6000);
    assert_eq!((sum.by_you, sum.bytes, sum.delete_after_ms), (1, 300, 88));
    assert_eq!(sum.line.as_deref(), Some("You removed 1 picture to free under 1 MB."));
}

#[test]
fn summary_not_run_with_nothing_pending_shows_nothing() {
    for removed in [vec![], vec![rp(T1, Reason::ByYou, 100, true, 77)]] {
        let sum = summary(Some(&rec_of(CleanupState::NotRun, 0, removed)), 6000);
        assert_eq!(sum.line, None);
        assert!(!sum.failed);
        assert_eq!((sum.removed, sum.by_you, sum.bytes, sum.delete_after_ms), (0, 0, 0, 0));
    }
}

#[test]
fn summary_failed_with_josephs_pending_still_says_did_not_finish_and_fills_the_counts() {
    let removed = vec![rp(T4, Reason::ByYou, 400, false, 432_001_000)];
    let sum = summary(Some(&rec_of(CleanupState::Failed { error: s("boom") }, 2000, removed)), 3000);
    assert!(sum.failed);
    assert_eq!(sum.line.as_deref(), Some("Picture cleanup did not finish."));
    assert_eq!((sum.by_you, sum.bytes), (1, 400));
}

// Answers to the tester's questions (end of the brief).

#[test]
fn summary_failed_delete_after_ms_is_the_soonest_pending_date() {
    let removed = vec![
        rp(T1, Reason::ByYou, 10, false, 900),
        rp(Z, Reason::ByYou, 10, false, 700),
        rp(T3, Reason::ByYou, 10, true, 100),
    ];
    let sum = summary(Some(&rec_of(CleanupState::Failed { error: s("boom") }, 2000, removed)), 3000);
    assert!(sum.failed);
    assert_eq!(sum.delete_after_ms, 700);
    assert_eq!(sum.by_you, 2);
}

#[test]
fn record_failure_on_an_existing_record_sets_ran_at_to_now_and_keeps_every_entry() {
    // After a Done run.
    let d = example();
    let pics = list_pictures(d.path());
    let done = apply(d.path(), &pics, &[removal("P1", Reason::Blank, None, None)], 5000).unwrap();
    let rec = record_failure(d.path(), "again", 6000).unwrap();
    assert_eq!(rec.ran_at_ms, 6000);
    assert_eq!(rec.removed, done.removed);
    assert_eq!(raw_record(d.path()).ran_at_ms, 6000);

    // After Joseph's removals only (NotRun, ran_at 0).
    let d = example();
    let first = remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let rec = record_failure(d.path(), "boom", 2000).unwrap();
    assert_eq!(rec.ran_at_ms, 2000);
    assert_eq!(rec.removed, first.removed);
}

#[test]
fn remove_again_after_the_date_has_passed_keeps_the_same_date_and_the_next_purge_deletes_it() {
    let d = example();
    remove_by_you(d.path(), &[s(T4)], 1000).unwrap(); // clock 432_001_000
    restore(d.path(), &[s(T4)]).unwrap();
    // Time has moved past the date, but no sweep ran in between.
    let rec = remove_again(d.path(), &[s(T4)]).unwrap();
    let e = entry_of(&rec, T4).unwrap();
    assert!(!e.restored);
    assert_eq!(e.delete_after_ms, 432_001_000);
    assert!(d.path().join(removed_of(T4)).exists());

    assert!(purge_if_expired(d.path(), 500_000_000));
    assert!(!d.path().join(removed_of(T4)).exists());
    assert!(load(d.path()).unwrap().removed.is_empty());
}

#[test]
fn restore_skips_a_name_no_entry_lists_restores_the_rest_and_is_ok() {
    let d = example();
    remove_by_you(d.path(), &[s(T1), s(T4)], 1000).unwrap();
    let rec = restore(d.path(), &[s("teams-frames/nope.jpg"), s(T4)]).unwrap();
    assert!(entry_of(&rec, T4).unwrap().restored);
    assert!(!entry_of(&rec, T1).unwrap().restored);
    assert!(d.path().join(T4).exists());
    assert!(d.path().join(removed_of(T1)).exists());
    assert_eq!(rec.removed.len(), 2);
    assert_eq!(load(d.path()), Some(rec));
}

#[test]
fn restore_with_only_unknown_names_is_ok_and_changes_nothing() {
    let d = example();
    let before = remove_by_you(d.path(), &[s(T4)], 1000).unwrap();
    let rec = restore(d.path(), &[s("teams-frames/nope.jpg")]).unwrap();
    assert_eq!(rec, before);
    assert!(d.path().join(removed_of(T4)).exists());
}

#[test]
fn summary_delete_after_ms_is_the_soonest_pending_and_ignores_restored() {
    let removed = vec![
        rp(T1, Reason::Blank, 10, true, 100),
        rp(Z, Reason::Blank, 10, false, 900),
        rp(T3, Reason::ByYou, 10, false, 700),
    ];
    let sum = summary(Some(&rec_of(CleanupState::Done, 5000, removed)), 6000);
    assert_eq!(sum.delete_after_ms, 700);
}
