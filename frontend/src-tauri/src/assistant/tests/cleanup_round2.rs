// Round 2 tests for picture cleanup: the disk is the truth, one writer at a
// time, and Try again sees every picture. Expected values come from the
// brief's "Round 2: review findings" and its worked example folder.

use super::*;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier};

const T1: &str = "teams-frames/teams-20261118-110300-478.jpg"; // 200 bytes
const T2: &str = "teams-frames/teams-20261118-110332-532.jpg"; // 300 bytes
const T3: &str = "teams-frames/teams-20261118-110433-637.jpg"; // 400 bytes
const Z1: &str = "zoom-frames/zoom-20261118-110310-000.jpg"; // 100 bytes

fn write(root: &Path, rel: &str, bytes: usize) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![b'x'; bytes]).unwrap();
}

fn example() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), T1, 200);
    write(d.path(), T2, 300);
    write(d.path(), T3, 400);
    write(d.path(), Z1, 100);
    d
}

fn removal(id: &str, reason: Reason, repeat_of: Option<&str>) -> Removal {
    Removal {
        id: id.to_string(),
        reason,
        caption: None,
        repeat_of: repeat_of.map(|s| s.to_string()),
    }
}

/// The worked example after `apply` at now_ms 1_000: P1 (T1, before_share)
/// and P3 (T2, repeat of P4) are in `teams-frames/removed/`.
fn applied() -> tempfile::TempDir {
    let d = example();
    let pics = list_pictures(d.path());
    let removals = vec![
        removal("P1", Reason::BeforeShare, None),
        removal("P3", Reason::Repeat, Some("P4")),
    ];
    apply(d.path(), &pics, &removals, 1_000).unwrap();
    d
}

fn removed_of(file: &str) -> String {
    let (dir, name) = file.rsplit_once('/').unwrap();
    format!("{}/removed/{}", dir, name)
}

fn raw_record(folder: &Path) -> CleanupRecord {
    serde_json::from_str(&fs::read_to_string(folder.join(RECORD_FILE)).unwrap()).unwrap()
}

fn entry<'a>(rec: &'a CleanupRecord, file: &str) -> Option<&'a RemovedPicture> {
    rec.removed.iter().find(|m| m.file == file)
}

fn pic(id: &str, file: &str) -> Picture {
    Picture { id: id.to_string(), file: file.to_string(), time: "11:03:00".to_string() }
}

fn session(sent: Vec<&str>) -> SavedSession {
    SavedSession {
        session_id: "s1".to_string(),
        cwd: std::path::PathBuf::from("/tmp/meeting"),
        model: "opus".to_string(),
        effort: "high".to_string(),
        read_dirs: vec![],
        sent: sent.into_iter().map(|s| s.to_string()).collect(),
    }
}

// ---------------------------------------------------------------------------
// 1. The disk is the truth
// ---------------------------------------------------------------------------

#[test]
fn load_marks_an_entry_restored_when_its_picture_was_moved_back_by_hand() {
    let d = applied();
    fs::rename(d.path().join(removed_of(T1)), d.path().join(T1)).unwrap();

    let rec = load(d.path()).unwrap();

    assert!(entry(&rec, T1).unwrap().restored);
    assert!(!entry(&rec, T2).unwrap().restored);
    // Removed 2, one moved back by hand: 1 removed, 300 bytes (T2).
    let s = summary(Some(&rec), 2_000);
    assert_eq!(s.removed, 1);
    assert_eq!(s.bytes, 300);
}

#[test]
fn load_writes_the_corrected_record_so_a_second_load_reads_the_same_from_the_file() {
    let d = applied();
    fs::rename(d.path().join(removed_of(T1)), d.path().join(T1)).unwrap();

    let first = load(d.path()).unwrap();

    // The file itself now holds the correction, not only the returned value.
    let on_disk = raw_record(d.path());
    assert_eq!(on_disk, first);
    assert!(entry(&on_disk, T1).unwrap().restored);
    let second = load(d.path()).unwrap();
    assert_eq!(second, first);
}

#[test]
fn load_marks_a_restored_entry_not_restored_when_its_picture_is_back_in_removed() {
    let d = applied();
    restore(d.path(), &[T1.to_string()]).unwrap();
    // Undo moved it back into removed/ but the record still says restored.
    fs::rename(d.path().join(T1), d.path().join(removed_of(T1))).unwrap();
    assert!(entry(&raw_record(d.path()), T1).unwrap().restored);

    let rec = load(d.path()).unwrap();

    assert!(!entry(&rec, T1).unwrap().restored);
    assert_eq!(summary(Some(&rec), 2_000).removed, 2);
    assert!(!entry(&raw_record(d.path()), T1).unwrap().restored);
}

#[test]
fn load_drops_an_entry_whose_picture_is_in_neither_place() {
    let d = applied();
    fs::remove_file(d.path().join(removed_of(T1))).unwrap();

    let rec = load(d.path()).unwrap();

    assert!(entry(&rec, T1).is_none());
    assert_eq!(rec.removed.len(), 1);
    assert!(entry(&rec, T2).is_some());
    assert_eq!(summary(Some(&rec), 2_000).removed, 1);
    // And the dropped entry stays gone in the file.
    assert!(entry(&raw_record(d.path()), T1).is_none());
}

#[test]
fn load_drops_an_entry_when_its_whole_frames_folder_is_gone() {
    let d = applied();
    // Both removed entries (T1, T2) live in teams-frames; delete the folder.
    fs::remove_dir_all(d.path().join("teams-frames")).unwrap();

    let rec = load(d.path()).unwrap();

    assert!(entry(&rec, T1).is_none());
    assert!(entry(&rec, T2).is_none());
    assert!(rec.removed.is_empty());
    assert_eq!(summary(Some(&rec), 2_000).removed, 0);
    // The corrected record is in the file too.
    assert_eq!(raw_record(d.path()), rec);
}

#[test]
fn load_moves_an_unlisted_picture_in_removed_back_to_its_frames_folder() {
    let d = applied();
    write(d.path(), "teams-frames/removed/x.jpg", 10);

    let rec = load(d.path()).unwrap();

    assert!(d.path().join("teams-frames/x.jpg").exists());
    assert!(!d.path().join("teams-frames/removed/x.jpg").exists());
    // It is not listed in the record.
    assert!(rec.removed.iter().all(|m| !m.file.ends_with("x.jpg") && !m.removed_file.ends_with("x.jpg")));
    assert_eq!(rec.removed.len(), 2);
}

#[test]
fn an_unlisted_picture_moved_back_by_load_shows_in_list_pictures() {
    let d = applied();
    // A name the capture writes, so list_pictures can number it.
    let stray = "teams-20261118-110500-000.jpg";
    write(d.path(), &format!("teams-frames/removed/{}", stray), 10);

    let rec = load(d.path()).unwrap();

    let files: Vec<String> = list_pictures(d.path()).into_iter().map(|p| p.file).collect();
    assert!(files.contains(&format!("teams-frames/{}", stray)));
    assert!(rec.removed.iter().all(|m| !m.file.ends_with(stray)));
}

#[test]
fn load_moves_a_stray_removed_picture_back_and_returns_none_when_there_is_no_record() {
    let d = example();
    write(d.path(), "teams-frames/removed/x.jpg", 10);
    assert!(!d.path().join(RECORD_FILE).exists());

    let rec = load(d.path());

    assert!(rec.is_none());
    assert!(d.path().join("teams-frames/x.jpg").exists());
    assert!(!d.path().join("teams-frames/removed/x.jpg").exists());
}

#[test]
fn load_moves_a_stray_back_into_its_own_frames_folder_for_zoom_too() {
    let d = applied();
    write(d.path(), "zoom-frames/removed/y.jpg", 10);

    load(d.path()).unwrap();

    assert!(d.path().join("zoom-frames/y.jpg").exists());
    assert!(!d.path().join("zoom-frames/removed/y.jpg").exists());
}

// The all-pictures brief: load reconciles every record that has entries,
// whatever its state (NotRun and Failed too, not only Done).

#[test]
fn load_reconciles_a_not_run_record_with_the_disk() {
    let d = example();
    remove_by_you(d.path(), &[T1.to_string(), T2.to_string()], 1_000).unwrap();
    // T1 moved back by hand; T2's picture deleted by hand.
    fs::rename(d.path().join(removed_of(T1)), d.path().join(T1)).unwrap();
    fs::remove_file(d.path().join(removed_of(T2))).unwrap();
    write(d.path(), "teams-frames/removed/x.jpg", 10);

    let rec = load(d.path()).unwrap();

    assert_eq!(rec.state, CleanupState::NotRun);
    assert!(entry(&rec, T1).unwrap().restored);
    assert!(entry(&rec, T2).is_none());
    assert!(d.path().join("teams-frames/x.jpg").exists());
    assert!(!d.path().join("teams-frames/removed/x.jpg").exists());
    assert_eq!(raw_record(d.path()), rec);
}

#[test]
fn load_reconciles_a_failed_record_that_has_entries() {
    let d = example();
    remove_by_you(d.path(), &[T1.to_string(), T2.to_string()], 1_000).unwrap();
    record_failure(d.path(), "boom", 2_000).unwrap();
    fs::rename(d.path().join(removed_of(T1)), d.path().join(T1)).unwrap();
    fs::remove_file(d.path().join(removed_of(T2))).unwrap();

    let rec = load(d.path()).unwrap();

    assert_eq!(rec.state, CleanupState::Failed { error: "boom".to_string() });
    assert!(entry(&rec, T1).unwrap().restored);
    assert!(entry(&rec, T2).is_none());
}

#[test]
fn load_marks_a_restored_by_you_entry_not_restored_when_its_picture_is_back_in_removed() {
    let d = example();
    remove_by_you(d.path(), &[T1.to_string()], 1_000).unwrap();
    restore(d.path(), &[T1.to_string()]).unwrap();
    fs::rename(d.path().join(T1), d.path().join(removed_of(T1))).unwrap();

    let rec = load(d.path()).unwrap();

    assert!(!entry(&rec, T1).unwrap().restored);
    assert_eq!(summary(Some(&rec), 2_000).by_you, 1);
}

// ---------------------------------------------------------------------------
// 2. One writer at a time
// ---------------------------------------------------------------------------

#[test]
fn two_threads_restoring_different_pictures_at_once_both_take_effect() {
    // A lost write is a race, so run it many times.
    for round in 0..100 {
        let d = applied();
        let folder = d.path().to_path_buf();
        let gate = Arc::new(Barrier::new(2));

        let results: Vec<Result<CleanupRecord, String>> = std::thread::scope(|s| {
            let handles: Vec<_> = [T1, T2]
                .into_iter()
                .map(|file| {
                    let gate = gate.clone();
                    let folder = folder.clone();
                    s.spawn(move || {
                        gate.wait();
                        restore(&folder, &[file.to_string()])
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        for r in &results {
            assert!(r.is_ok(), "round {}: a restore failed: {:?}", round, r);
        }
        // Both files are back.
        assert!(d.path().join(T1).exists(), "round {}: T1 not back", round);
        assert!(d.path().join(T2).exists(), "round {}: T2 not back", round);
        assert!(!d.path().join(removed_of(T1)).exists());
        assert!(!d.path().join(removed_of(T2)).exists());
        // The record FILE (read raw, so load's reconcile cannot hide a lost
        // write) marks both restored.
        let on_disk = raw_record(d.path());
        assert!(entry(&on_disk, T1).unwrap().restored, "round {}: T1 not marked", round);
        assert!(entry(&on_disk, T2).unwrap().restored, "round {}: T2 not marked", round);
        // No leftover temporary file from the two writers.
        let mut names: Vec<String> = fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![RECORD_FILE.to_string(), "teams-frames".to_string(), "zoom-frames".to_string()],
            "round {}",
            round
        );
    }
}

#[test]
fn a_record_write_leaves_no_temporary_file_in_the_meeting_folder() {
    let only = |d: &Path| {
        let mut names: Vec<String> = fs::read_dir(d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    };
    let expected = vec![RECORD_FILE.to_string(), "teams-frames".to_string(), "zoom-frames".to_string()];

    // apply
    let d = applied();
    assert_eq!(only(d.path()), expected);
    // restore
    restore(d.path(), &[]).unwrap();
    assert_eq!(only(d.path()), expected);
    // remove_again
    remove_again(d.path(), &[T1.to_string()]).unwrap();
    assert_eq!(only(d.path()), expected);
    // purge_if_expired
    assert!(purge_if_expired(d.path(), 1_000 + 432_000_000));
    assert_eq!(only(d.path()), expected);
    // record_failure
    let f = example();
    record_failure(f.path(), "boom", 5).unwrap();
    assert_eq!(only(f.path()), expected);
}

#[test]
fn restore_and_purge_on_one_folder_leave_a_readable_record() {
    // Mixed writers from several threads must never leave a half-written file.
    for _ in 0..30 {
        let d = applied();
        let folder = d.path().to_path_buf();
        let gate = Arc::new(Barrier::new(3));
        std::thread::scope(|s| {
            let (g1, f1) = (gate.clone(), folder.clone());
            s.spawn(move || {
                g1.wait();
                let _ = restore(&f1, &[T1.to_string()]);
            });
            let (g2, f2) = (gate.clone(), folder.clone());
            s.spawn(move || {
                g2.wait();
                let _ = restore(&f2, &[T2.to_string()]);
            });
            let (g3, f3) = (gate.clone(), folder.clone());
            s.spawn(move || {
                g3.wait();
                let _ = purge_if_expired(&f3, 1_000 + 432_000_000 - 1); // not yet due
            });
        });
        // The file parses as a whole record.
        let text = fs::read_to_string(d.path().join(RECORD_FILE)).unwrap();
        assert!(serde_json::from_str::<CleanupRecord>(&text).is_ok());
    }
}

// ---------------------------------------------------------------------------
// 3. Try again sees every picture
// ---------------------------------------------------------------------------

#[test]
fn unsent_returns_the_pictures_the_session_was_not_sent_in_order() {
    let pics = vec![
        pic("P1", "teams-frames/a.jpg"),
        pic("P2", "teams-frames/b.jpg"),
        pic("P3", "zoom-frames/c.jpg"),
    ];
    let s = session(vec!["teams-frames/a.jpg"]);

    let out = unsent(&pics, &s);

    assert_eq!(out, vec![pics[1].clone(), pics[2].clone()]);
}

#[test]
fn unsent_returns_every_picture_when_the_session_was_sent_none() {
    let pics = vec![pic("P1", "teams-frames/a.jpg"), pic("P2", "zoom-frames/c.jpg")];

    assert_eq!(unsent(&pics, &session(vec![])), pics);
}

#[test]
fn unsent_returns_nothing_when_the_session_holds_every_picture() {
    let pics = vec![pic("P1", "teams-frames/a.jpg"), pic("P2", "zoom-frames/c.jpg")];

    let out = unsent(&pics, &session(vec!["zoom-frames/c.jpg", "teams-frames/a.jpg"]));

    assert!(out.is_empty());
}

#[test]
fn unsent_ignores_sent_paths_that_are_not_in_the_picture_list() {
    let pics = vec![pic("P1", "teams-frames/a.jpg"), pic("P2", "teams-frames/b.jpg")];

    let out = unsent(&pics, &session(vec!["teams-frames/gone.jpg", "teams-frames/b.jpg"]));

    assert_eq!(out, vec![pics[0].clone()]);
}

#[test]
fn a_session_file_saved_without_sent_loads_and_unsent_gives_every_picture() {
    let d = tempfile::tempdir().unwrap();
    fs::write(
        d.path().join(SESSION_FILE),
        r#"{"sessionId":"s9","cwd":"/tmp/meeting","model":"opus","effort":"high","readDirs":[]}"#,
    )
    .unwrap();

    let loaded = load_session(d.path()).expect("an older session file must still load");

    assert_eq!(loaded.session_id, "s9");
    assert!(loaded.sent.is_empty());
    let pics = vec![pic("P1", "teams-frames/a.jpg"), pic("P2", "zoom-frames/c.jpg")];
    assert_eq!(unsent(&pics, &loaded), pics);
}

#[test]
fn the_sent_list_survives_save_session_and_load_session() {
    let d = tempfile::tempdir().unwrap();
    let s = session(vec!["teams-frames/a.jpg", "zoom-frames/c.jpg"]);

    save_session(d.path(), &s);
    let loaded = load_session(d.path()).unwrap();

    assert_eq!(loaded, s);
    let pics = vec![
        pic("P1", "teams-frames/a.jpg"),
        pic("P2", "teams-frames/b.jpg"),
        pic("P3", "zoom-frames/c.jpg"),
    ];
    assert_eq!(unsent(&pics, &loaded), vec![pics[1].clone()]);
}
