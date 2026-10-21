// "What was saved" per meeting: times, length, pictures kept and disk space of
// the meeting folder, plus the meeting's Brief and the list totals. Read-only:
// nothing here writes to a folder or to the database. All wording lives in this
// file so the page line and the list header cannot drift apart.

use crate::database::repositories::meeting::MeetingsRepository;
use chrono::{DateTime, FixedOffset, Local, TimeZone, Utc};
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::path::Path;

/// What the folder on disk says about one recording.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FolderScan {
    pub pictures: u64,
    pub bytes: u64,
    pub created_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSaved {
    /// First non-empty line of the Brief, trimmed; absent when there is none.
    pub brief: Option<String>,
    /// "10:04 to 10:47, 43 min, 181 pictures, 165 MB" or "Recording folder not found".
    pub line: String,
    pub duration_seconds: f64,
    pub pictures: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedOverview {
    pub meeting_count: usize,
    /// "10 meetings, 7.4 hours, 1,204 pictures, 2.1 GB"
    pub totals: String,
    pub meetings: HashMap<String, MeetingSaved>,
}

fn with_commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn format_pictures(n: u64) -> String {
    format!("{} {}", with_commas(n), if n == 1 { "picture" } else { "pictures" })
}

pub fn format_size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb < 1.0 {
        return "under 1 MB".to_string();
    }
    let whole = mb.round();
    if whole < 1024.0 {
        format!("{} MB", whole as u64)
    } else {
        format!("{:.1} GB", mb / 1024.0)
    }
}

pub fn format_length(seconds: f64) -> String {
    if seconds < 30.0 {
        return "under 1 min".to_string();
    }
    let minutes = (seconds / 60.0).round() as u64;
    let (h, m) = (minutes / 60, minutes % 60);
    match (h, m) {
        (0, m) => format!("{} min", m),
        (h, 0) => format!("{} h", h),
        (h, m) => format!("{} h {} min", h, m),
    }
}

fn clock<Tz: TimeZone>(t: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    t.with_timezone(tz).format("%H:%M").to_string()
}

/// The recording length in seconds: `duration_seconds`, else end minus start.
/// None without a `completed_at`. The page line and the list totals both use this.
fn length_seconds(scan: &FolderScan, meeting_created: DateTime<Utc>) -> Option<f64> {
    let end = scan.completed_at?;
    let start = scan.created_at.unwrap_or(meeting_created);
    Some(
        scan.duration_seconds
            .unwrap_or_else(|| (end - start).num_milliseconds().max(0) as f64 / 1000.0),
    )
}

/// The page line (without the "What was saved: " lead) for one meeting.
/// `meeting_created` is the database row's time, used when metadata.json has none.
pub fn format_line<Tz: TimeZone>(
    scan: &FolderScan,
    meeting_created: DateTime<Utc>,
    tz: &Tz,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let start = scan.created_at.unwrap_or(meeting_created);
    let mut parts = Vec::new();
    match (scan.completed_at, length_seconds(scan, meeting_created)) {
        (Some(end), Some(secs)) => {
            parts.push(format!("{} to {}", clock(start, tz), clock(end, tz)));
            parts.push(format_length(secs));
        }
        _ => parts.push(clock(start, tz)),
    }
    parts.push(format_pictures(scan.pictures));
    parts.push(format_size(scan.bytes));
    parts.join(", ")
}

/// "0 hours", "1 hour" (exactly 1.0 at one decimal), else "7.4 hours".
fn format_hours(hours: f64) -> String {
    let tenths = (hours * 10.0).round() as i64;
    match tenths {
        0 => "0 hours".to_string(),
        10 => "1 hour".to_string(),
        t => format!("{:.1} hours", t as f64 / 10.0),
    }
}

pub fn format_totals(meetings: usize, hours: f64, pictures: u64, bytes: u64) -> String {
    format!(
        "{} {}, {}, {}, {}",
        with_commas(meetings as u64),
        if meetings == 1 { "meeting" } else { "meetings" },
        format_hours(hours),
        format_pictures(pictures),
        format_size(bytes),
    )
}

/// First non-empty line of a Brief, trimmed.
pub fn first_brief_line(brief: &str) -> Option<String> {
    brief
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

fn count_jpgs(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .filter(|e| {
            e.file_type().map(|t| t.is_file()).unwrap_or(false)
                && e.path()
                    .extension()
                    .map(|x| x.eq_ignore_ascii_case("jpg"))
                    .unwrap_or(false)
        })
        .count() as u64
}

fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_bytes(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

fn parse_time(v: &serde_json::Value, key: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.get(key)?.as_str()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Reads the folder. None when it does not exist. Blocking: call off the runtime.
pub fn scan_folder(folder: &Path) -> Option<FolderScan> {
    if !folder.is_dir() {
        return None;
    }
    let meta: Option<serde_json::Value> = std::fs::read_to_string(folder.join("metadata.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    Some(FolderScan {
        pictures: count_jpgs(&folder.join("zoom-frames")) + count_jpgs(&folder.join("teams-frames")),
        bytes: dir_bytes(folder),
        created_at: meta.as_ref().and_then(|m| parse_time(m, "created_at")),
        completed_at: meta.as_ref().and_then(|m| parse_time(m, "completed_at")),
        duration_seconds: meta
            .as_ref()
            .and_then(|m| m.get("duration_seconds"))
            .and_then(|d| d.as_f64()),
    })
}

struct Row_ {
    id: String,
    created: DateTime<Utc>,
    folder: Option<String>,
    brief: Option<String>,
}

fn build<Tz: TimeZone>(rows: Vec<Row_>, tz: &Tz) -> SavedOverview
where
    Tz::Offset: std::fmt::Display,
{
    let (mut hours, mut pictures, mut bytes) = (0.0, 0u64, 0u64);
    let mut meetings = HashMap::new();
    let count = rows.len();
    for r in rows {
        let scan = r.folder.as_deref().and_then(|f| scan_folder(Path::new(f)));
        let saved = match &scan {
            Some(s) => MeetingSaved {
                brief: r.brief,
                line: format_line(s, r.created, tz),
                duration_seconds: length_seconds(s, r.created).unwrap_or(0.0),
                pictures: s.pictures,
                bytes: s.bytes,
            },
            None => MeetingSaved {
                brief: r.brief,
                line: "Recording folder not found".to_string(),
                duration_seconds: 0.0,
                pictures: 0,
                bytes: 0,
            },
        };
        hours += saved.duration_seconds / 3600.0;
        pictures += saved.pictures;
        bytes += saved.bytes;
        meetings.insert(r.id, saved);
    }
    SavedOverview {
        meeting_count: count,
        totals: format_totals(count, hours, pictures, bytes),
        meetings,
    }
}

async fn load_rows(pool: &SqlitePool, only: Option<&str>) -> Result<Vec<Row_>, String> {
    let briefs: HashMap<String, String> = sqlx::query("SELECT meeting_id, brief FROM meeting_assistant_meta")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|r| (r.get("meeting_id"), r.get("brief")))
        .collect();
    let meetings = MeetingsRepository::get_meetings(pool).await.map_err(|e| e.to_string())?;
    Ok(meetings
        .into_iter()
        .filter(|m| only.map_or(true, |id| m.id == id))
        .map(|m| Row_ {
            brief: briefs.get(&m.id).and_then(|b| first_brief_line(b)),
            id: m.id,
            created: m.created_at.0,
            folder: m.folder_path,
        })
        .collect())
}

/// Numbers for the whole list (or one meeting), in the Mac's local time zone.
pub async fn overview(pool: &SqlitePool, only: Option<&str>) -> Result<SavedOverview, String> {
    let rows = load_rows(pool, only).await?;
    tokio::task::spawn_blocking(move || build(rows, &Local))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_get_meetings_saved(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<SavedOverview, String> {
    overview(state.db_manager.pool(), None).await
}

#[tauri::command]
pub async fn api_get_meeting_saved(
    state: tauri::State<'_, crate::state::AppState>,
    meeting_id: String,
) -> Result<Option<MeetingSaved>, String> {
    Ok(overview(state.db_manager.pool(), Some(&meeting_id)).await?.meetings.remove(&meeting_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn est() -> FixedOffset {
        FixedOffset::west_opt(4 * 3600).unwrap()
    }
    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }
    fn touch(p: &Path, bytes: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn length_rules() {
        assert_eq!(format_length(2572.24), "43 min");
        assert_eq!(format_length(29.9), "under 1 min");
        assert_eq!(format_length(30.0), "1 min");
        assert_eq!(format_length(4320.0), "1 h 12 min");
        assert_eq!(format_length(7200.0), "2 h");
        assert_eq!(format_length(3600.0), "1 h");
        assert_eq!(format_length(3580.0), "1 h"); // rounds to 60 min
    }

    #[test]
    fn picture_rules() {
        assert_eq!(format_pictures(0), "0 pictures");
        assert_eq!(format_pictures(1), "1 picture");
        assert_eq!(format_pictures(181), "181 pictures");
        assert_eq!(format_pictures(1204), "1,204 pictures");
        assert_eq!(format_pictures(1_234_567), "1,234,567 pictures");
    }

    #[test]
    fn size_rules() {
        assert_eq!(format_size(0), "under 1 MB");
        assert_eq!(format_size(1_048_575), "under 1 MB");
        assert_eq!(format_size(1_048_576), "1 MB");
        assert_eq!(format_size(172_782_838), "165 MB");
        assert_eq!(format_size(1024 * 1_048_576 - 1_048_576), "1023 MB");
        assert_eq!(format_size(1024 * 1_048_576), "1.0 GB");
        assert_eq!(format_size((1.2 * 1024.0 * 1_048_576.0) as u64), "1.2 GB");
    }

    #[test]
    fn totals_rules() {
        let gb2_1 = (2.1 * 1024.0 * 1_048_576.0) as u64;
        assert_eq!(format_totals(10, 7.4, 1204, gb2_1), "10 meetings, 7.4 hours, 1,204 pictures, 2.1 GB");
        assert_eq!(format_hours(1.0), "1 hour");
        assert_eq!(format_hours(1.04), "1 hour");
        assert_eq!(format_hours(1.06), "1.1 hours");
        assert_eq!(format_hours(2.0), "2.0 hours");
        assert_eq!(format_totals(1, 0.0, 0, 0), "1 meeting, 0 hours, 0 pictures, under 1 MB");
    }

    #[test]
    fn brief_first_nonempty_line() {
        assert_eq!(first_brief_line("\n  \n  Plan the sale \nsecond"), Some("Plan the sale".to_string()));
        assert_eq!(first_brief_line("  \n\n"), None);
        assert_eq!(first_brief_line(""), None);
    }

    #[test]
    fn line_for_the_real_folder_numbers() {
        let scan = FolderScan {
            pictures: 181,
            bytes: 172_782_838,
            created_at: Some(utc("2026-10-21T14:04:32.763214+00:00")),
            completed_at: Some(utc("2026-10-21T14:47:05.305813+00:00")),
            duration_seconds: Some(2572.24),
        };
        assert_eq!(
            format_line(&scan, utc("2026-10-21T14:04:31+00:00"), &est()),
            "10:04 to 10:47, 43 min, 181 pictures, 165 MB"
        );
    }

    #[test]
    fn line_without_end_uses_meeting_row_time() {
        let scan = FolderScan { pictures: 181, bytes: 172_782_838, ..Default::default() };
        assert_eq!(
            format_line(&scan, utc("2026-10-21T14:04:31+00:00"), &est()),
            "10:04, 181 pictures, 165 MB"
        );
    }

    #[test]
    fn counts_zoom_frames_only() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..3 {
            touch(&d.path().join(format!("zoom-frames/f{i}.jpg")), 10);
        }
        touch(&d.path().join("zoom-frames/notes.txt"), 10);
        assert_eq!(scan_folder(d.path()).unwrap().pictures, 3);
    }

    #[test]
    fn counts_both_frame_folders() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("zoom-frames/a.jpg"), 1);
        touch(&d.path().join("teams-frames/b.jpg"), 1);
        touch(&d.path().join("teams-frames/c.JPG"), 1);
        touch(&d.path().join("other/d.jpg"), 1);
        assert_eq!(scan_folder(d.path()).unwrap().pictures, 3);
    }

    #[test]
    fn counts_neither_folder() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("audio.mp4"), 1);
        assert_eq!(scan_folder(d.path()).unwrap().pictures, 0);
    }

    #[test]
    fn size_is_recursive_total() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("a.bin"), 100);
        touch(&d.path().join("zoom-frames/b.jpg"), 200);
        touch(&d.path().join("deep/er/c.bin"), 300);
        assert_eq!(scan_folder(d.path()).unwrap().bytes, 600);
    }

    #[test]
    fn reads_metadata_json() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("metadata.json"),
            r#"{"created_at":"2026-10-21T14:04:32.763214+00:00","completed_at":"2026-10-21T14:47:05.305813+00:00","duration_seconds":2572.24}"#,
        )
        .unwrap();
        let s = scan_folder(d.path()).unwrap();
        assert_eq!(s.created_at, Some(utc("2026-10-21T14:04:32.763214+00:00")));
        assert_eq!(s.completed_at, Some(utc("2026-10-21T14:47:05.305813+00:00")));
        assert_eq!(s.duration_seconds, Some(2572.24));
    }

    #[test]
    fn metadata_missing_or_broken_falls_back() {
        let d = tempfile::tempdir().unwrap();
        let s = scan_folder(d.path()).unwrap();
        assert_eq!((s.created_at, s.completed_at, s.duration_seconds), (None, None, None));
        fs::write(d.path().join("metadata.json"), "{not json").unwrap();
        assert_eq!(scan_folder(d.path()).unwrap().completed_at, None);
        // created_at but no completed_at (recording never finished)
        fs::write(d.path().join("metadata.json"), r#"{"created_at":"2026-10-21T14:04:32+00:00"}"#).unwrap();
        let s = scan_folder(d.path()).unwrap();
        assert!(s.created_at.is_some() && s.completed_at.is_none());
    }

    #[test]
    fn missing_folder_is_none() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(scan_folder(&d.path().join("gone")), None);
    }

    fn rows_for(dirs: &[(&str, Option<&Path>, Option<&str>)]) -> Vec<Row_> {
        dirs.iter()
            .map(|(id, p, b)| Row_ {
                id: id.to_string(),
                created: utc("2026-10-21T14:04:31+00:00"),
                folder: p.map(|p| p.to_string_lossy().to_string()),
                brief: b.map(str::to_string),
            })
            .collect()
    }

    #[test]
    fn totals_use_the_same_length_as_the_line() {
        // completed_at but no duration_seconds: 90 minutes by end minus start.
        let a = tempfile::tempdir().unwrap();
        fs::write(
            a.path().join("metadata.json"),
            r#"{"created_at":"2026-10-21T14:00:00+00:00","completed_at":"2026-10-21T15:30:00+00:00"}"#,
        )
        .unwrap();
        let out = build(rows_for(&[("a", Some(a.path()), None)]), &est());
        assert!(out.meetings["a"].line.starts_with("10:00 to 11:30, 1 h 30 min"));
        assert_eq!(out.meetings["a"].duration_seconds, 5400.0);
        assert!(out.totals.starts_with("1 meeting, 1.5 hours,"));
    }

    #[test]
    fn overview_totals_and_missing_folder() {
        let a = tempfile::tempdir().unwrap();
        fs::write(
            a.path().join("metadata.json"),
            r#"{"created_at":"2026-10-21T14:04:32+00:00","completed_at":"2026-10-21T15:04:32+00:00","duration_seconds":3600.0}"#,
        )
        .unwrap();
        touch(&a.path().join("zoom-frames/1.jpg"), 1);
        let b = tempfile::tempdir().unwrap(); // no metadata.json: adds 0 hours
        touch(&b.path().join("teams-frames/1.jpg"), 1);
        touch(&b.path().join("teams-frames/2.jpg"), 1);
        let gone = b.path().join("gone");
        let out = build(
            rows_for(&[("a", Some(a.path()), Some("Topic")), ("b", Some(b.path()), None), ("c", Some(&gone), None), ("d", None, None)]),
            &est(),
        );
        assert_eq!(out.meeting_count, 4);
        assert_eq!(out.totals, "4 meetings, 1 hour, 3 pictures, under 1 MB");
        assert_eq!(out.meetings["a"].brief.as_deref(), Some("Topic"));
        assert!(out.meetings["a"].line.starts_with("10:04 to 11:04, 1 h, 1 picture"));
        assert!(out.meetings["b"].line.starts_with("10:04, 2 pictures")); // from the row time
        assert_eq!(out.meetings["c"].line, "Recording folder not found");
        assert_eq!(out.meetings["d"].line, "Recording folder not found");
    }

    #[tokio::test]
    async fn overview_reads_briefs_from_the_database() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE meetings (id TEXT PRIMARY KEY, title TEXT, created_at TEXT, updated_at TEXT, folder_path TEXT)")
            .execute(&pool).await.unwrap();
        sqlx::query(include_str!("../migrations/20261022120000_meeting_assistant_meta.sql"))
            .execute(&pool).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_string_lossy().to_string();
        for id in ["m1", "m2", "m3"] {
            sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, folder_path) VALUES (?, 't', '2026-10-21T14:04:31+00:00', '2026-10-21T14:04:31+00:00', ?)")
                .bind(id).bind(&folder).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO meeting_assistant_meta (meeting_id, brief) VALUES ('m1', '\n Plan the sale \nmore'), ('m2', '  ')")
            .execute(&pool).await.unwrap();
        let all = overview(&pool, None).await.unwrap();
        assert_eq!(all.meetings["m1"].brief.as_deref(), Some("Plan the sale"));
        assert_eq!(all.meetings["m2"].brief, None);
        assert_eq!(all.meetings["m3"].brief, None);
        let one = overview(&pool, Some("m1")).await.unwrap();
        assert_eq!(one.meeting_count, 1);
    }
}
