// Locked tests for assistant/cleanup.rs. Written by the tester, from the
// picture-cleanup brief (docs/superpowers/plans/2026-11-18-picture-cleanup-brief.md).
// Every expected value comes from the brief's tables, worked examples and answers.
#[allow(unused_imports)]
use super::*;
use std::fs;
use std::path::Path;

const DAY_MS: i64 = 86_400_000;
const T1: &str = "teams-frames/teams-20261118-110300-478.jpg"; // 200 bytes
const T2: &str = "teams-frames/teams-20261118-110332-532.jpg"; // 300 bytes
const T3: &str = "teams-frames/teams-20261118-110433-637.jpg"; // 400 bytes
const Z1: &str = "zoom-frames/zoom-20261118-110310-000.jpg"; // 100 bytes

fn write(root: &Path, rel: &str, bytes: usize) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![b'x'; bytes]).unwrap();
}

/// The brief's worked-example meeting folder.
fn example() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), T1, 200);
    write(d.path(), T2, 300);
    write(d.path(), T3, 400);
    write(d.path(), Z1, 100);
    d
}

fn pic(id: &str, file: &str, time: &str) -> Picture {
    Picture { id: id.to_string(), file: file.to_string(), time: time.to_string() }
}

/// P1 teams 11:03:00, P2 zoom 11:03:10, P3 teams 11:03:32, P4 teams 11:04:33.
fn example_pictures() -> Vec<Picture> {
    vec![
        pic("P1", T1, "11:03:00"),
        pic("P2", Z1, "11:03:10"),
        pic("P3", T2, "11:03:32"),
        pic("P4", T3, "11:04:33"),
    ]
}

fn removal(id: &str, reason: Reason, caption: Option<&str>, repeat_of: Option<&str>) -> Removal {
    Removal {
        id: id.to_string(),
        reason,
        caption: caption.map(str::to_string),
        repeat_of: repeat_of.map(str::to_string),
    }
}

/// P1 before_share, P3 repeat of P4 (the brief's example).
fn example_removals() -> Vec<Removal> {
    vec![
        removal("P1", Reason::BeforeShare, None, None),
        removal("P3", Reason::Repeat, Some("Same page, scrolled up"), Some("P4")),
    ]
}

fn entry(file: &str, reason: Reason, bytes: u64, restored: bool) -> RemovedPicture {
    RemovedPicture {
        file: file.to_string(),
        removed_file: file.replacen("-frames/", "-frames/removed/", 1),
        time: "11:03:00".to_string(),
        reason,
        caption: None,
        repeat_of: if reason == Reason::Repeat { Some(T3.to_string()) } else { None },
        bytes,
        restored,
        delete_after_ms: 1000 + 5 * DAY_MS,
    }
}

/// The record after the all-pictures change: no record-level delete_after_ms.
fn record(state: CleanupState, removed: Vec<RemovedPicture>) -> CleanupRecord {
    CleanupRecord { state, ran_at_ms: 1000, removed }
}

fn applied() -> tempfile::TempDir {
    let d = example();
    let pics = list_pictures(d.path());
    apply(d.path(), &pics, &example_removals(), 1_000).unwrap();
    d
}

// ---------- list_pictures ----------

#[test]
fn list_pictures_numbers_in_capture_order_across_both_folders() {
    let d = example();
    assert_eq!(list_pictures(d.path()), example_pictures());
}

#[test]
fn list_pictures_never_lists_the_removed_folder() {
    let d = example();
    write(d.path(), "teams-frames/removed/teams-20261118-110000-000.jpg", 10);
    write(d.path(), "zoom-frames/removed/zoom-20261118-110001-000.jpg", 10);
    assert_eq!(list_pictures(d.path()), example_pictures());
}

#[test]
fn list_pictures_is_empty_without_frames_folders() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "audio.mp4", 50);
    assert!(list_pictures(d.path()).is_empty());
}

// ---------- picture_list ----------

#[test]
fn picture_list_has_one_line_per_picture() {
    let text = picture_list(&example_pictures());
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines,
        vec![
            "P1 = Teams window at 11:03:00",
            "P2 = Zoom window at 11:03:10",
            "P3 = Teams window at 11:03:32",
            "P4 = Teams window at 11:04:33",
        ]
    );
}

// ---------- parse_reply ----------

#[test]
fn parse_reply_worked_example_ignores_text_around_the_object() {
    let raw = r#"Here you go: {"remove":[{"id":"P1","reason":"before_share"},{"id":"P3","reason":"repeat","repeat_of":"P4","caption":"Same page, scrolled up"}]} done"#;
    assert_eq!(parse_reply(raw, &example_pictures()).unwrap(), example_removals());
}

#[test]
fn parse_reply_accepts_every_reason() {
    let raw = r#"{"remove":[
        {"id":"P1","reason":"loading"},
        {"id":"P2","reason":"greyed"},
        {"id":"P3","reason":"blank"},
        {"id":"P4","reason":"popup"}
    ]}"#;
    let got = parse_reply(raw, &example_pictures()).unwrap();
    let reasons: Vec<Reason> = got.iter().map(|r| r.reason).collect();
    assert_eq!(reasons, vec![Reason::Loading, Reason::Greyed, Reason::Blank, Reason::Popup]);

    let raw = r#"{"remove":[{"id":"P1","reason":"before_share"},{"id":"P2","reason":"repeat","repeat_of":"P4"}]}"#;
    let got = parse_reply(raw, &example_pictures()).unwrap();
    assert_eq!(got[0].reason, Reason::BeforeShare);
    assert_eq!(got[1].reason, Reason::Repeat);
}

#[test]
fn parse_reply_keeps_a_caption() {
    let raw = r#"{"remove":[{"id":"P2","reason":"popup","caption":"Save changes? box"}]}"#;
    let got = parse_reply(raw, &example_pictures()).unwrap();
    assert_eq!(got, vec![removal("P2", Reason::Popup, Some("Save changes? box"), None)]);
}

#[test]
fn parse_reply_empty_remove_list_is_valid() {
    assert_eq!(parse_reply(r#"{"remove": []}"#, &example_pictures()).unwrap(), vec![]);
}

#[test]
fn parse_reply_without_an_object_or_remove_array_is_err() {
    let pics = example_pictures();
    assert!(parse_reply("I could not decide.", &pics).is_err());
    assert!(parse_reply("", &pics).is_err());
    assert!(parse_reply(r#"{"other":[]}"#, &pics).is_err());
}

#[test]
fn parse_reply_skips_an_id_not_in_the_list() {
    let raw = r#"{"remove":[{"id":"P9","reason":"loading"},{"id":"P2","reason":"blank"}]}"#;
    assert_eq!(
        parse_reply(raw, &example_pictures()).unwrap(),
        vec![removal("P2", Reason::Blank, None, None)]
    );
}

#[test]
fn parse_reply_an_id_listed_twice_is_removed_once_first_wins() {
    let raw = r#"{"remove":[{"id":"P1","reason":"loading"},{"id":"P1","reason":"blank"}]}"#;
    assert_eq!(
        parse_reply(raw, &example_pictures()).unwrap(),
        vec![removal("P1", Reason::Loading, None, None)]
    );
}

#[test]
fn parse_reply_skips_an_unknown_reason_only() {
    let raw = r#"{"remove":[{"id":"P1","reason":"boring"},{"id":"P2","reason":"blank"}]}"#;
    assert_eq!(
        parse_reply(raw, &example_pictures()).unwrap(),
        vec![removal("P2", Reason::Blank, None, None)]
    );
}

#[test]
fn parse_reply_repeat_without_repeat_of_is_skipped() {
    let raw = r#"{"remove":[{"id":"P1","reason":"repeat"},{"id":"P2","reason":"blank"}]}"#;
    assert_eq!(
        parse_reply(raw, &example_pictures()).unwrap(),
        vec![removal("P2", Reason::Blank, None, None)]
    );
}

#[test]
fn parse_reply_repeat_of_an_unknown_picture_is_skipped() {
    let raw = r#"{"remove":[{"id":"P1","reason":"repeat","repeat_of":"P9"}]}"#;
    assert_eq!(parse_reply(raw, &example_pictures()).unwrap(), vec![]);
}

#[test]
fn parse_reply_repeat_of_itself_is_skipped() {
    let raw = r#"{"remove":[{"id":"P1","reason":"repeat","repeat_of":"P1"}]}"#;
    assert_eq!(parse_reply(raw, &example_pictures()).unwrap(), vec![]);
}

#[test]
fn parse_reply_repeat_of_an_already_removed_picture_is_skipped() {
    // Reply order: P2 is removed first, so P1 "repeating P2" would point at nothing.
    let raw = r#"{"remove":[{"id":"P2","reason":"loading"},{"id":"P1","reason":"repeat","repeat_of":"P2"}]}"#;
    assert_eq!(
        parse_reply(raw, &example_pictures()).unwrap(),
        vec![removal("P2", Reason::Loading, None, None)]
    );
}

#[test]
fn parse_reply_a_later_removal_of_a_picture_an_accepted_repeat_names_is_skipped() {
    // P1 repeats P2 and is accepted (P2 is still kept then); removing P2 afterwards is skipped.
    let raw = r#"{"remove":[{"id":"P1","reason":"repeat","repeat_of":"P2"},{"id":"P2","reason":"loading"}]}"#;
    assert_eq!(
        parse_reply(raw, &example_pictures()).unwrap(),
        vec![removal("P1", Reason::Repeat, None, Some("P2"))]
    );
}

#[test]
fn parse_reply_two_repeats_naming_each_other_first_is_accepted_second_skipped() {
    let raw = r#"{"remove":[{"id":"P1","reason":"repeat","repeat_of":"P2"},{"id":"P2","reason":"repeat","repeat_of":"P1"}]}"#;
    assert_eq!(
        parse_reply(raw, &example_pictures()).unwrap(),
        vec![removal("P1", Reason::Repeat, None, Some("P2"))]
    );
}

// ---------- apply ----------

#[test]
fn apply_moves_pictures_into_removed_and_writes_the_record() {
    let d = example();
    let pics = list_pictures(d.path());
    let rec = apply(d.path(), &pics, &example_removals(), 1_000).unwrap();

    assert!(!d.path().join(T1).exists());
    assert!(!d.path().join(T2).exists());
    let m1 = d.path().join("teams-frames/removed/teams-20261118-110300-478.jpg");
    let m3 = d.path().join("teams-frames/removed/teams-20261118-110332-532.jpg");
    assert_eq!(fs::metadata(&m1).unwrap().len(), 200);
    assert_eq!(fs::metadata(&m3).unwrap().len(), 300);
    assert!(d.path().join(T3).exists());
    assert!(d.path().join(Z1).exists());

    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.ran_at_ms, 1000);
    assert_eq!(rec.removed.len(), 2);
    // Each entry carries its own clock: now_ms plus five days.
    assert!(rec.removed.iter().all(|r| r.delete_after_ms == 1000 + 432_000_000));

    let r1 = rec.removed.iter().find(|r| r.file == T1).unwrap();
    assert_eq!(r1.removed_file, "teams-frames/removed/teams-20261118-110300-478.jpg");
    assert_eq!(r1.time, "11:03:00");
    assert_eq!(r1.reason, Reason::BeforeShare);
    assert_eq!(r1.bytes, 200);
    assert_eq!(r1.repeat_of, None);
    assert!(!r1.restored);

    let r3 = rec.removed.iter().find(|r| r.file == T2).unwrap();
    assert_eq!(r3.removed_file, "teams-frames/removed/teams-20261118-110332-532.jpg");
    assert_eq!(r3.reason, Reason::Repeat);
    assert_eq!(r3.caption.as_deref(), Some("Same page, scrolled up"));
    assert_eq!(r3.repeat_of.as_deref(), Some(T3));
    assert_eq!(r3.bytes, 300);
    assert!(!r3.restored);
}

#[test]
fn apply_record_reads_back_every_field() {
    let d = example();
    let pics = list_pictures(d.path());
    let written = apply(d.path(), &pics, &example_removals(), 1_000).unwrap();
    assert!(d.path().join(RECORD_FILE).is_file());
    assert_eq!(load(d.path()), Some(written));
}

#[test]
fn restore_window_is_five_days() {
    assert_eq!(RESTORE_DAYS * DAY_MS, 432_000_000);
}

#[test]
fn list_pictures_after_apply_renumbers_the_kept_ones() {
    let d = applied();
    let got = list_pictures(d.path());
    assert_eq!(got, vec![pic("P1", Z1, "11:03:10"), pic("P2", T3, "11:04:33")]);
}

#[test]
fn apply_with_nothing_to_remove_writes_done_with_an_empty_list() {
    let d = example();
    let pics = list_pictures(d.path());
    let rec = apply(d.path(), &pics, &[], 1_000).unwrap();
    assert_eq!(rec.state, CleanupState::Done);
    assert!(rec.removed.is_empty());
    assert_eq!(list_pictures(d.path()), example_pictures());
    assert_eq!(load(d.path()), Some(rec.clone()));
    assert_eq!(summary(Some(&rec), 2_000).line, None);
}

#[test]
fn apply_skips_a_picture_that_no_longer_exists_and_applies_the_rest() {
    let d = example();
    let pics = list_pictures(d.path());
    fs::remove_file(d.path().join(T1)).unwrap(); // P1 vanished
    let rec = apply(d.path(), &pics, &example_removals(), 1_000).unwrap();
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].file, T2);
    assert!(d.path().join("teams-frames/removed/teams-20261118-110332-532.jpg").exists());
}

#[test]
fn apply_when_every_named_picture_is_gone_returns_ok_with_an_empty_done_record() {
    let d = example();
    let pics = list_pictures(d.path());
    fs::remove_file(d.path().join(T1)).unwrap();
    fs::remove_file(d.path().join(T2)).unwrap();
    let rec = apply(d.path(), &pics, &example_removals(), 1_000).unwrap();
    assert_eq!(rec.state, CleanupState::Done);
    assert!(rec.removed.is_empty());
}

#[test]
fn apply_when_one_move_fails_returns_ok_and_records_only_what_moved() {
    let d = example();
    let pics = list_pictures(d.path());
    // A plain file named `removed` in zoom-frames, so a move into it must fail.
    write(d.path(), "zoom-frames/removed", 1);
    let removals = vec![
        removal("P3", Reason::Blank, None, None), // teams: can move
        removal("P2", Reason::Popup, None, None), // zoom: cannot move
    ];
    let rec = apply(d.path(), &pics, &removals, 1_000).unwrap();

    // The failed picture stays where it was; the other moved.
    assert!(d.path().join(Z1).exists());
    assert!(!d.path().join(T2).exists());
    assert!(d.path().join("teams-frames/removed/teams-20261118-110332-532.jpg").exists());

    assert_eq!(rec.state, CleanupState::Done);
    assert_eq!(rec.removed.len(), 1);
    assert_eq!(rec.removed[0].file, T2);
    assert_eq!(load(d.path()), Some(rec));
}

// ---------- summary ----------

#[test]
fn summary_after_apply_matches_the_worked_example() {
    let d = applied();
    let rec = load(d.path()).unwrap();
    let s = summary(Some(&rec), 2_000);
    assert_eq!(s.removed, 2);
    assert_eq!(s.repeats, 1);
    assert_eq!(s.not_useful, 1);
    assert_eq!(s.bytes, 500);
    assert!(!s.failed);
    assert_eq!(s.delete_after_ms, 1000 + 432_000_000);
    assert_eq!(
        s.line.as_deref(),
        Some("Claude removed 2 pictures to free under 1 MB: 1 repeat, 1 not useful.")
    );
}

#[test]
fn summary_for_93_removed() {
    // 56 repeats + 37 not useful = 93; 53,477,376 bytes = 51 MB (53,477,376 / 1,048,576).
    let mut removed = Vec::new();
    for i in 0..56 {
        removed.push(entry(&format!("teams-frames/r{i}.jpg"), Reason::Repeat, if i == 0 { 53_477_376 } else { 0 }, false));
    }
    for i in 0..37 {
        removed.push(entry(&format!("zoom-frames/n{i}.jpg"), Reason::Loading, 0, false));
    }
    let s = summary(Some(&record(CleanupState::Done, removed)), 2_000);
    assert_eq!((s.removed, s.repeats, s.not_useful, s.bytes), (93, 56, 37, 53_477_376));
    assert_eq!(
        s.line.as_deref(),
        Some("Claude removed 93 pictures to free 51 MB: 56 repeats, 37 not useful.")
    );
}

#[test]
fn summary_with_only_repeats_leaves_out_the_not_useful_part() {
    let removed = (0..3).map(|i| entry(&format!("teams-frames/r{i}.jpg"), Reason::Repeat, 10, false)).collect();
    let s = summary(Some(&record(CleanupState::Done, removed)), 2_000);
    assert_eq!(s.line.as_deref(), Some("Claude removed 3 pictures to free under 1 MB: 3 repeats."));
}

#[test]
fn summary_with_only_not_useful_leaves_out_the_repeats_part() {
    let removed = vec![
        entry("teams-frames/a.jpg", Reason::Popup, 10, false),
        entry("teams-frames/b.jpg", Reason::Blank, 10, false),
    ];
    let s = summary(Some(&record(CleanupState::Done, removed)), 2_000);
    assert_eq!(s.line.as_deref(), Some("Claude removed 2 pictures to free under 1 MB: 2 not useful."));
}

#[test]
fn summary_uses_the_singular_for_one() {
    let removed = vec![entry("teams-frames/a.jpg", Reason::Repeat, 10, false)];
    let s = summary(Some(&record(CleanupState::Done, removed)), 2_000);
    assert_eq!(s.line.as_deref(), Some("Claude removed 1 picture to free under 1 MB: 1 repeat."));
}

#[test]
fn summary_with_no_record_shows_nothing() {
    let s = summary(None, 2_000);
    assert_eq!(s.line, None);
    assert!(!s.failed);
    assert_eq!((s.removed, s.repeats, s.not_useful, s.bytes), (0, 0, 0, 0));
}

#[test]
fn summary_done_with_nothing_removed_shows_nothing() {
    let s = summary(Some(&record(CleanupState::Done, vec![])), 2_000);
    assert_eq!(s.line, None);
    assert!(!s.failed);
    assert_eq!(s.removed, 0);
}

#[test]
fn summary_leaves_out_restored_pictures() {
    let removed = vec![
        entry("teams-frames/a.jpg", Reason::Repeat, 200, true),
        entry("teams-frames/b.jpg", Reason::Popup, 300, false),
    ];
    let s = summary(Some(&record(CleanupState::Done, removed)), 2_000);
    assert_eq!((s.removed, s.repeats, s.not_useful, s.bytes), (1, 0, 1, 300));
    assert_eq!(s.line.as_deref(), Some("Claude removed 1 picture to free under 1 MB: 1 not useful."));
}

#[test]
fn summary_all_restored_shows_no_line() {
    let removed = vec![
        entry("teams-frames/a.jpg", Reason::Repeat, 200, true),
        entry("teams-frames/b.jpg", Reason::Popup, 300, true),
    ];
    let s = summary(Some(&record(CleanupState::Done, removed)), 2_000);
    assert_eq!(s.line, None);
    assert_eq!(s.removed, 0);
}

#[test]
fn summary_failed_says_so() {
    let rec = record(CleanupState::Failed { error: "timed out".to_string() }, vec![]);
    let s = summary(Some(&rec), 2_000);
    assert_eq!(s.line.as_deref(), Some("Picture cleanup did not finish."));
    assert!(s.failed);
    assert_eq!(s.removed, 0);
}

#[test]
fn summary_done_with_an_emptied_list_shows_no_line_and_no_date() {
    // What a purge leaves behind: Done, no entries (Purged no longer exists).
    let s = summary(Some(&record(CleanupState::Done, vec![])), 2_000);
    assert_eq!(s.line, None);
    assert!(!s.failed);
    assert_eq!((s.removed, s.by_you, s.bytes, s.delete_after_ms), (0, 0, 0, 0));
}

// ---------- restore ----------

#[test]
fn restore_one_moves_it_back_and_marks_it_restored() {
    let d = applied();
    let rec = restore(d.path(), &[T1.to_string()]).unwrap();

    assert!(d.path().join(T1).exists());
    assert!(!d.path().join("teams-frames/removed/teams-20261118-110300-478.jpg").exists());
    assert!(!d.path().join(T2).exists());
    assert!(d.path().join("teams-frames/removed/teams-20261118-110332-532.jpg").exists());

    assert!(rec.removed.iter().find(|r| r.file == T1).unwrap().restored);
    assert!(!rec.removed.iter().find(|r| r.file == T2).unwrap().restored);
    assert_eq!(load(d.path()), Some(rec.clone()));

    let s = summary(Some(&rec), 2_000);
    assert_eq!(s.removed, 1);
    assert_eq!(s.bytes, 300);
}

#[test]
fn restored_picture_is_listed_again() {
    let d = applied();
    restore(d.path(), &[T1.to_string()]).unwrap();
    let files: Vec<String> = list_pictures(d.path()).into_iter().map(|p| p.file).collect();
    assert_eq!(files, vec![T1.to_string(), Z1.to_string(), T3.to_string()]);
}

#[test]
fn restore_with_an_empty_list_restores_everything() {
    let d = applied();
    let rec = restore(d.path(), &[]).unwrap();
    assert!(d.path().join(T1).exists());
    assert!(d.path().join(T2).exists());
    assert!(rec.removed.iter().all(|r| r.restored));
    let s = summary(Some(&rec), 2_000);
    assert_eq!(s.removed, 0);
    assert_eq!(s.line, None);
    assert_eq!(list_pictures(d.path()), example_pictures());
}

#[test]
fn restore_with_no_record_is_err_and_changes_nothing() {
    let d = example();
    assert!(restore(d.path(), &[]).is_err());
    assert_eq!(list_pictures(d.path()), example_pictures());
    assert_eq!(load(d.path()), None);
}

#[test]
fn restore_on_a_record_with_no_entries_is_err_and_changes_nothing() {
    // After a purge the record is still Done but holds no entries.
    let d = applied();
    assert!(purge_if_expired(d.path(), 1000 + 432_000_000));
    let before = load(d.path());
    assert!(restore(d.path(), &[]).is_err());
    assert!(restore(d.path(), &[T1.to_string()]).is_err());
    assert_eq!(load(d.path()), before);
    assert!(!d.path().join(T1).exists());
    assert!(!d.path().join("teams-frames/removed/teams-20261118-110300-478.jpg").exists());
}

// ---------- record_failure, load ----------

#[test]
fn record_failure_writes_a_failed_record_and_moves_nothing() {
    let d = example();
    let rec = record_failure(d.path(), "claude timed out", 5_000).unwrap();
    assert_eq!(rec.state, CleanupState::Failed { error: "claude timed out".to_string() });
    assert_eq!(rec.ran_at_ms, 5_000);
    assert!(rec.removed.is_empty());
    assert_eq!(load(d.path()), Some(rec.clone()));
    assert_eq!(list_pictures(d.path()), example_pictures());

    let s = summary(Some(&rec), 6_000);
    assert!(s.failed);
    assert_eq!(s.line.as_deref(), Some("Picture cleanup did not finish."));
}

#[test]
fn load_is_none_without_a_record_or_with_an_unreadable_one() {
    let d = example();
    assert_eq!(load(d.path()), None);
    fs::write(d.path().join(RECORD_FILE), "{not json").unwrap();
    assert_eq!(load(d.path()), None);
}

#[test]
fn load_reads_back_each_kind_of_record_from_its_file() {
    let mut full = entry(T2, Reason::Repeat, 300, true);
    full.caption = Some("Same page, scrolled up".to_string());
    let records = vec![
        record(CleanupState::Done, vec![full, entry(T1, Reason::Loading, 200, false)]),
        CleanupRecord { state: CleanupState::Failed { error: "boom".to_string() }, ran_at_ms: 7, removed: vec![] },
        // Joseph's removals only: NotRun, ran_at_ms 0.
        CleanupRecord {
            state: CleanupState::NotRun,
            ran_at_ms: 0,
            removed: vec![entry(T1, Reason::ByYou, 200, false)],
        },
    ];
    for rec in records {
        // A Done record's pictures are on disk where the record says they are
        // (restored: at `file`; not restored: at `removed_file`), so the disk
        // agrees with the record. A fresh folder per record keeps one record's
        // files from leaking into the next.
        let d = tempfile::tempdir().unwrap();
        {
            for m in &rec.removed {
                write(d.path(), if m.restored { &m.file } else { &m.removed_file }, m.bytes as usize);
            }
        }
        fs::write(d.path().join(RECORD_FILE), serde_json::to_string(&rec).unwrap()).unwrap();
        assert_eq!(load(d.path()), Some(rec));
    }
}

// ---------- purge_if_expired ----------

#[test]
fn purge_before_the_window_ends_deletes_nothing() {
    let d = applied();
    assert!(!purge_if_expired(d.path(), 1000 + 432_000_000 - 1));
    assert!(d.path().join("teams-frames/removed/teams-20261118-110300-478.jpg").exists());
    assert!(d.path().join("teams-frames/removed/teams-20261118-110332-532.jpg").exists());
    assert_eq!(load(d.path()).unwrap().state, CleanupState::Done);
}

#[test]
fn purge_at_the_end_of_the_window_deletes_the_files_and_drops_the_entries() {
    let d = applied();
    assert!(purge_if_expired(d.path(), 1000 + 432_000_000));
    assert!(!d.path().join("teams-frames/removed/teams-20261118-110300-478.jpg").exists());
    assert!(!d.path().join("teams-frames/removed/teams-20261118-110332-532.jpg").exists());
    assert!(d.path().join(T3).exists());
    assert!(d.path().join(Z1).exists());
    let rec = load(d.path()).unwrap();
    // The state is never changed by a purge (Purged is gone); the list is empty.
    assert_eq!(rec.state, CleanupState::Done);
    assert!(rec.removed.is_empty());
    assert_eq!(summary(Some(&rec), 1000 + 432_000_000).line, None);
    assert!(!purge_if_expired(d.path(), 1000 + 432_000_000 + 1));
}

#[test]
fn purge_deletes_removed_in_every_frames_folder() {
    let d = example();
    let pics = list_pictures(d.path());
    let removals = vec![
        removal("P1", Reason::Blank, None, None), // teams
        removal("P2", Reason::Popup, None, None), // zoom
    ];
    apply(d.path(), &pics, &removals, 1_000).unwrap();
    assert!(d.path().join("zoom-frames/removed").exists());
    assert!(d.path().join("teams-frames/removed").exists());
    assert!(purge_if_expired(d.path(), 1_000 + 432_000_000));
    assert!(!d.path().join("zoom-frames/removed/zoom-20261118-110310-000.jpg").exists());
    assert!(!d.path().join("teams-frames/removed/teams-20261118-110300-478.jpg").exists());
}

#[test]
fn purge_with_no_record_does_nothing() {
    let d = example();
    assert!(!purge_if_expired(d.path(), i64::MAX));
    assert_eq!(list_pictures(d.path()), example_pictures());
}

#[test]
fn purge_leaves_a_failed_record_with_no_entries_alone() {
    // A failed run with nothing pending: nothing to delete, the state stays Failed.
    let d = example();
    let rec = record_failure(d.path(), "boom", 5_000).unwrap();
    assert!(!purge_if_expired(d.path(), i64::MAX));
    assert_eq!(load(d.path()), Some(rec));
}

// ---------- should_start ----------

#[test]
fn should_start_rules() {
    assert!(should_start(None, false));
    assert!(!should_start(None, true));

    let failed = record(CleanupState::Failed { error: "x".to_string() }, vec![]);
    assert!(should_start(Some(&failed), false));

    let done = record(CleanupState::Done, vec![entry(T1, Reason::Blank, 1, false)]);
    assert!(!should_start(Some(&done), false));

    let done_empty = record(CleanupState::Done, vec![]);
    assert!(!should_start(Some(&done_empty), false));

    // Joseph's removals only: Claude has not run, so Draft note starts it.
    let not_run = record(CleanupState::NotRun, vec![entry(T1, Reason::ByYou, 1, false)]);
    assert!(should_start(Some(&not_run), false));
    let not_run_empty = record(CleanupState::NotRun, vec![]);
    assert!(should_start(Some(&not_run_empty), false));
    assert!(!should_start(Some(&not_run), true));
}

#[test]
fn should_start_a_run_in_flight_blocks_a_second_one_even_after_a_failure() {
    let failed = record(CleanupState::Failed { error: "x".to_string() }, vec![]);
    assert!(!should_start(Some(&failed), true));
}

// ---------- absolute ----------

#[test]
fn absolute_joins_the_meeting_folder_and_the_relative_file() {
    let p = absolute(Path::new("/meetings/m1"), T1);
    assert_eq!(p, Path::new("/meetings/m1").join(T1));
}
