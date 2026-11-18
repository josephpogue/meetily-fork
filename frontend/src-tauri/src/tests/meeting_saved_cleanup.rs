// Locked tests: "What was saved" leaves out pictures the cleanup removed. Written by the tester.
// Expected values come from the picture-cleanup brief's worked example:
// 200 bytes in teams-frames/a.jpg, 300 bytes in teams-frames/removed/b.jpg and
// 50 bytes audio.mp4 give 1 picture and 250 bytes.
#[allow(unused_imports)]
use super::*;
use std::fs;
use std::path::Path;

fn put(root: &Path, rel: &str, bytes: usize) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![b'x'; bytes]).unwrap();
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

#[test]
fn removed_folder_is_left_out_of_pictures_and_bytes() {
    let d = tempfile::tempdir().unwrap();
    put(d.path(), "teams-frames/a.jpg", 200);
    put(d.path(), "teams-frames/removed/b.jpg", 300);
    put(d.path(), "audio.mp4", 50);
    let s = scan_folder(d.path()).unwrap();
    assert_eq!(s.pictures, 1);
    assert_eq!(s.bytes, 250); // 200 + 50; the 300 in removed/ is not counted
}

#[test]
fn removed_folder_in_the_zoom_frames_folder_is_left_out_too() {
    let d = tempfile::tempdir().unwrap();
    put(d.path(), "zoom-frames/a.jpg", 100);
    put(d.path(), "zoom-frames/removed/b.jpg", 700);
    let s = scan_folder(d.path()).unwrap();
    assert_eq!(s.pictures, 1);
    assert_eq!(s.bytes, 100);
}

#[test]
fn a_frames_folder_holding_only_removed_pictures_counts_nothing() {
    let d = tempfile::tempdir().unwrap();
    put(d.path(), "teams-frames/removed/a.jpg", 200);
    put(d.path(), "teams-frames/removed/b.jpg", 300);
    let s = scan_folder(d.path()).unwrap();
    assert_eq!(s.pictures, 0);
    assert_eq!(s.bytes, 0);
}

#[test]
fn a_removed_folder_elsewhere_in_the_meeting_folder_still_counts() {
    let d = tempfile::tempdir().unwrap();
    put(d.path(), "teams-frames/a.jpg", 200);
    put(d.path(), "removed/x.bin", 60);
    put(d.path(), "deep/removed/y.bin", 40);
    let s = scan_folder(d.path()).unwrap();
    assert_eq!(s.pictures, 1);
    assert_eq!(s.bytes, 300); // 200 + 60 + 40
}

#[test]
fn a_restored_picture_counts_again() {
    let d = tempfile::tempdir().unwrap();
    put(d.path(), "teams-frames/a.jpg", 200);
    put(d.path(), "teams-frames/removed/b.jpg", 300);
    put(d.path(), "audio.mp4", 50);
    // Restore b.jpg the way the cleanup does: move it back up one level.
    fs::rename(d.path().join("teams-frames/removed/b.jpg"), d.path().join("teams-frames/b.jpg")).unwrap();
    let s = scan_folder(d.path()).unwrap();
    assert_eq!(s.pictures, 2);
    assert_eq!(s.bytes, 550); // 200 + 300 + 50
}

#[test]
fn a_meeting_never_cleaned_counts_as_today() {
    let d = tempfile::tempdir().unwrap();
    put(d.path(), "zoom-frames/a.jpg", 10);
    put(d.path(), "teams-frames/b.jpg", 20);
    put(d.path(), "audio.mp4", 30);
    let s = scan_folder(d.path()).unwrap();
    assert_eq!(s.pictures, 2);
    assert_eq!(s.bytes, 60);
}

#[test]
fn the_page_line_shows_only_kept_pictures_and_size() {
    let d = tempfile::tempdir().unwrap();
    put(d.path(), "teams-frames/a.jpg", 200);
    // 2,097,152 bytes in removed/ would read "2 MB" if counted.
    put(d.path(), "teams-frames/removed/b.jpg", 2_097_152);
    let s = scan_folder(d.path()).unwrap();
    let est = FixedOffset::west_opt(4 * 3600).unwrap();
    assert_eq!(
        format_line(&s, utc("2026-10-21T14:04:31+00:00"), &est),
        "10:04, 1 picture, under 1 MB"
    );
    assert_eq!(format_size(s.bytes), "under 1 MB");
}

#[test]
fn the_list_totals_leave_out_removed_pictures() {
    let a = tempfile::tempdir().unwrap();
    put(a.path(), "teams-frames/a.jpg", 200);
    put(a.path(), "teams-frames/removed/b.jpg", 300);
    put(a.path(), "audio.mp4", 50);
    let rows = vec![Row_ {
        id: "a".to_string(),
        created: utc("2026-10-21T14:04:31+00:00"),
        folder: Some(a.path().to_string_lossy().to_string()),
        brief: None,
    }];
    let est = FixedOffset::west_opt(4 * 3600).unwrap();
    let out = build(rows, &est);
    assert_eq!(out.meetings["a"].pictures, 1);
    assert_eq!(out.meetings["a"].bytes, 250);
    assert_eq!(out.totals, "1 meeting, 0 hours, 1 picture, under 1 MB");
}
