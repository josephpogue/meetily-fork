// assistant/cleanup.rs
//
// Picture cleanup after a call. When Joseph presses Draft note, a second
// Claude run (a fork of the call's deep lane, beside the note) names the kept
// pictures that add nothing: loading screens, greyed-out pages, blank pages,
// popups, the gallery before anyone shared, and repeats of a kept picture.
// This module owns everything around that run except the run itself: the
// numbered picture list the run is given, parsing its reply, moving removed
// pictures into `removed/` inside their frames folder, restoring them, the
// record kept in the meeting folder, and deleting `removed/` for good once
// the restore window ends. Claude never writes; this code moves the files.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The cleanup run's system prompt.
pub const CLEANUP_PROMPT: &str = include_str!("prompts/cleanup.md");

const DAY_MS: i64 = 86_400_000;

/// Days a removed picture can be restored before it is deleted for good.
pub const RESTORE_DAYS: i64 = 5;
/// Folder, inside `zoom-frames/` or `teams-frames/`, that holds removed pictures.
pub const REMOVED_DIR: &str = "removed";
/// The cleanup record, in the meeting folder beside `assistant.json`.
pub const RECORD_FILE: &str = "picture-cleanup.json";
/// The deep-lane session id the cleanup forks, saved beside the record so Try
/// again can fork it after an app restart.
pub const SESSION_FILE: &str = "picture-cleanup-session.json";
/// The frames folders a meeting can have, in the order pictures are listed.
pub const FRAME_DIRS: [&str; 2] = ["zoom-frames", "teams-frames"];

/// Why a picture was removed. "Not useful" is every reason but `Repeat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// "Still loading": a spinner or "Loading content...".
    Loading,
    /// "Greyed out while saving or testing".
    Greyed,
    /// "Blank or no data".
    Blank,
    /// "Popup over the page".
    Popup,
    /// "Before anyone shared": the call's gallery or video view.
    BeforeShare,
    /// The same screen as a kept picture.
    Repeat,
}

/// One kept picture as the cleanup run sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// "P1", "P2", ... in capture order across both frames folders.
    pub id: String,
    /// Path relative to the meeting folder, e.g. "teams-frames/teams-20261118-110300-478.jpg".
    pub file: String,
    /// When it was captured, local time "HH:MM:SS", read from the file name.
    pub time: String,
}

/// One removal the run asked for, after parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub id: String,
    pub reason: Reason,
    /// A more specific caption than the reason, e.g. "Save changes? box".
    pub caption: Option<String>,
    /// For a repeat: the id of the kept picture it repeats.
    pub repeat_of: Option<String>,
}

/// One removed picture in the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovedPicture {
    /// Where it was before removal, relative to the meeting folder.
    pub file: String,
    /// Where it is now, relative to the meeting folder (inside `removed/`).
    pub removed_file: String,
    pub time: String,
    pub reason: Reason,
    pub caption: Option<String>,
    /// For a repeat: the kept picture's file, relative to the meeting folder.
    pub repeat_of: Option<String>,
    pub bytes: u64,
    /// True once Joseph restored it; the file is back at `file`.
    pub restored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum CleanupState {
    /// The run finished and its removals were applied.
    Done,
    /// The run could not finish; nothing was moved. Try again is offered.
    Failed { error: String },
    /// The restore window ended and `removed/` was deleted.
    Purged,
}

/// `picture-cleanup.json`: what the cleanup did to one meeting's pictures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRecord {
    #[serde(flatten)]
    pub state: CleanupState,
    pub ran_at_ms: i64,
    /// `ran_at_ms` plus RESTORE_DAYS; 0 for a failed run.
    pub delete_after_ms: i64,
    pub removed: Vec<RemovedPicture>,
}

/// What the meeting page header shows, worded here like `meeting_saved.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupSummary {
    /// "Claude removed 93 pictures to free 51 MB: 56 repeats, 37 not useful."
    /// or "Picture cleanup did not finish." None when there is nothing to show.
    pub line: Option<String>,
    pub failed: bool,
    /// Removed and not restored.
    pub removed: u64,
    pub repeats: u64,
    pub not_useful: u64,
    pub bytes: u64,
    /// When removed pictures are deleted for good; 0 when not applicable.
    pub delete_after_ms: i64,
}

/// Meeting folders with a cleanup run in flight, so a second press of Draft
/// note (a redraft) starts no second run. Lives outside the core lock on purpose.
static IN_FLIGHT: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// Marks a run in flight for the folder. False when one already is.
pub fn begin_run(meeting_folder: &Path) -> bool {
    IN_FLIGHT
        .lock()
        .unwrap()
        .get_or_insert_with(HashSet::new)
        .insert(meeting_folder.to_path_buf())
}

/// Ends the folder's run, however it ended.
pub fn end_run(meeting_folder: &Path) {
    if let Some(set) = IN_FLIGHT.lock().unwrap().as_mut() {
        set.remove(meeting_folder);
    }
}

pub fn run_in_flight(meeting_folder: &Path) -> bool {
    IN_FLIGHT.lock().unwrap().as_ref().map(|s| s.contains(meeting_folder)).unwrap_or(false)
}

/// What Try again needs to fork the call's deep session after a restart: the
/// session id and the folder and models it was opened with (a session is found
/// by the folder it ran in).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSession {
    pub session_id: String,
    pub cwd: PathBuf,
    pub model: String,
    pub effort: String,
    pub read_dirs: Vec<PathBuf>,
    /// Meeting-relative paths of the pictures this session was already sent
    /// (for example "teams-frames/teams-20261118-110300-478.jpg"). A file saved
    /// before this field existed loads with an empty list.
    #[serde(default)]
    pub sent: Vec<String>,
}

/// The pictures from `pictures` that the saved session was not sent, in the
/// same order: what a Try again must attach so Claude sees every picture.
pub fn unsent(pictures: &[Picture], session: &SavedSession) -> Vec<Picture> {
    let held: HashSet<&str> = session.sent.iter().map(|s| s.as_str()).collect();
    pictures.iter().filter(|p| !held.contains(p.file.as_str())).cloned().collect()
}

/// Saves the deep-lane session beside the record (best effort).
pub fn save_session(meeting_folder: &Path, session: &SavedSession) {
    let written = serde_json::to_string(session)
        .map_err(|e| e.to_string())
        .and_then(|t| std::fs::write(meeting_folder.join(SESSION_FILE), t).map_err(|e| e.to_string()));
    if let Err(e) = written {
        log::warn!("picture cleanup: could not save the session: {}", e);
    }
}

/// The saved deep-lane session, when the meeting has one.
pub fn load_session(meeting_folder: &Path) -> Option<SavedSession> {
    serde_json::from_str(&std::fs::read_to_string(meeting_folder.join(SESSION_FILE)).ok()?).ok()
}

/// "20261118-110300-478" from "teams-20261118-110300-478.jpg", with its
/// "HH:MM:SS" form. None for a name the capture does not write.
fn stamp_of(name: &str) -> Option<(String, String)> {
    let stem = name.strip_suffix(".jpg").or_else(|| name.strip_suffix(".JPG"))?;
    let (_, rest) = stem.split_once('-')?;
    let mut parts = rest.split('-');
    let (date, hms, ms) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    if !(digits(date, 8) && digits(hms, 6) && !ms.is_empty() && ms.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some((rest.to_string(), format!("{}:{}:{}", &hms[0..2], &hms[2..4], &hms[4..6])))
}

/// When a kept picture was captured, in epoch milliseconds, read from its file
/// name (local time, "teams-20261118-110300-478.jpg"). None for another name.
pub fn captured_at_ms(file: &str) -> Option<i64> {
    use chrono::TimeZone;
    let name = file.rsplit('/').next()?;
    let (key, _) = stamp_of(name)?;
    let (stamp, ms) = key.rsplit_once('-')?;
    let naive = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S").ok()?;
    let local = chrono::Local.from_local_datetime(&naive).earliest()?;
    Some(local.timestamp_millis() + ms.parse::<i64>().ok()?.min(999))
}

/// The kept pictures in `meeting_folder` (top level of each frames folder,
/// never `removed/`), oldest first, numbered P1, P2, ...
pub fn list_pictures(meeting_folder: &Path) -> Vec<Picture> {
    let mut found: Vec<(String, String, String)> = Vec::new(); // (sort key, file, time)
    for dir in FRAME_DIRS {
        let Ok(entries) = std::fs::read_dir(meeting_folder.join(dir)) else { continue };
        for e in entries.flatten() {
            if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if let Some((key, time)) = stamp_of(&name) {
                found.push((key, format!("{}/{}", dir, name), time));
            }
        }
    }
    found.sort();
    found
        .into_iter()
        .enumerate()
        .map(|(i, (_, file, time))| Picture { id: format!("P{}", i + 1), file, time })
        .collect()
}

fn app_name(file: &str) -> &'static str {
    if file.starts_with("zoom-frames/") {
        "Zoom"
    } else {
        "Teams"
    }
}

/// The part of the cleanup run's message that lists the pictures, one line
/// each: "P1 = Teams window at 11:03:00".
pub fn picture_list(pictures: &[Picture]) -> String {
    pictures
        .iter()
        .map(|p| format!("{} = {} window at {}", p.id, app_name(&p.file), p.time))
        .collect::<Vec<_>>()
        .join("\n")
}

fn reason_of(s: &str) -> Option<Reason> {
    Some(match s {
        "loading" => Reason::Loading,
        "greyed" => Reason::Greyed,
        "blank" => Reason::Blank,
        "popup" => Reason::Popup,
        "before_share" => Reason::BeforeShare,
        "repeat" => Reason::Repeat,
        _ => return None,
    })
}

/// The first JSON object in `raw` that holds a `remove` array.
fn find_remove_list(raw: &str) -> Option<Vec<serde_json::Value>> {
    for (i, _) in raw.match_indices('{') {
        let mut stream = serde_json::Deserializer::from_str(&raw[i..]).into_iter::<serde_json::Value>();
        if let Some(Ok(serde_json::Value::Object(mut obj))) = stream.next() {
            if let Some(serde_json::Value::Array(list)) = obj.remove("remove") {
                return Some(list);
            }
        }
    }
    None
}

/// Parses the run's reply: a JSON object `{"remove":[{"id":"P3","reason":"loading",
/// "caption":"Loading content..."},{"id":"P7","reason":"repeat","repeat_of":"P8"}]}`,
/// possibly with text around it. Err when no such object can be read.
pub fn parse_reply(raw: &str, pictures: &[Picture]) -> Result<Vec<Removal>, String> {
    let list = find_remove_list(raw).ok_or("the reply holds no JSON object with a remove list")?;
    let known: HashSet<&str> = pictures.iter().map(|p| p.id.as_str()).collect();
    let mut removed: HashSet<String> = HashSet::new();
    // Kept pictures that an accepted repeat points at; they must stay kept.
    let mut protected: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for item in list {
        let text = |k: &str| {
            item.get(k)
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let Some(id) = text("id") else { continue };
        if !known.contains(id.as_str()) || removed.contains(&id) || protected.contains(&id) {
            continue;
        }
        let Some(reason) = text("reason").as_deref().and_then(reason_of) else { continue };
        let mut repeat_of = None;
        if reason == Reason::Repeat {
            let Some(target) = text("repeat_of") else { continue };
            if target == id || !known.contains(target.as_str()) || removed.contains(&target) {
                continue;
            }
            protected.insert(target.clone());
            repeat_of = Some(target);
        }
        removed.insert(id.clone());
        out.push(Removal { id, reason, caption: text("caption"), repeat_of });
    }
    Ok(out)
}

/// One lock per meeting folder: every writer of the record (and every move of
/// a picture in or out of `removed/`) holds it, so two of them never interleave.
static FOLDER_LOCKS: Mutex<Option<HashMap<PathBuf, Arc<Mutex<()>>>>> = Mutex::new(None);

fn folder_lock(meeting_folder: &Path) -> Arc<Mutex<()>> {
    FOLDER_LOCKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .entry(meeting_folder.to_path_buf())
        .or_default()
        .clone()
}

/// Writes the record to a temporary file of its own, then renames it into
/// place, so a reader never sees a half-written file.
fn write_record(meeting_folder: &Path, rec: &CleanupRecord) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let text = serde_json::to_string_pretty(rec).map_err(|e| e.to_string())?;
    let tmp = meeting_folder.join(format!(
        "{}.{}.{}.tmp",
        RECORD_FILE,
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let written = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, meeting_folder.join(RECORD_FILE)));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not write the cleanup record: {}", e)
    })
}

/// `dir/name.jpg` -> `dir/removed/name.jpg`.
fn removed_path(file: &str) -> Option<String> {
    let (dir, name) = file.rsplit_once('/')?;
    Some(format!("{}/{}/{}", dir, REMOVED_DIR, name))
}

/// Moves each removal's picture into `removed/` in its own frames folder and
/// writes the record. Returns the record written.
pub fn apply(meeting_folder: &Path, pictures: &[Picture], removals: &[Removal], now_ms: i64) -> Result<CleanupRecord, String> {
    let lock = folder_lock(meeting_folder);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let file_of = |id: &str| pictures.iter().find(|p| p.id == id).map(|p| p.file.clone());
    let mut moved: Vec<RemovedPicture> = Vec::new();
    for r in removals {
        let Some(pic) = pictures.iter().find(|p| p.id == r.id) else { continue };
        let Some(dest_rel) = removed_path(&pic.file) else { continue };
        let src = absolute(meeting_folder, &pic.file);
        let dest = absolute(meeting_folder, &dest_rel);
        let Ok(meta) = std::fs::metadata(&src) else { continue }; // gone: skipped
        let outcome = match dest.parent() {
            Some(d) => std::fs::create_dir_all(d).and_then(|_| std::fs::rename(&src, &dest)),
            None => Ok(()),
        };
        if let Err(e) = outcome {
            log::warn!("picture cleanup: could not move {}: {}", pic.file, e);
            continue;
        }
        moved.push(RemovedPicture {
            file: pic.file.clone(),
            removed_file: dest_rel,
            time: pic.time.clone(),
            reason: r.reason,
            caption: r.caption.clone(),
            repeat_of: r.repeat_of.as_deref().and_then(file_of),
            bytes: meta.len(),
            restored: false,
        });
    }
    let rec = CleanupRecord {
        state: CleanupState::Done,
        ran_at_ms: now_ms,
        delete_after_ms: now_ms + RESTORE_DAYS * DAY_MS,
        removed: moved,
    };
    if let Err(e) = write_record(meeting_folder, &rec) {
        // Nothing may be lost without a record: put every picture back.
        for m in &rec.removed {
            let _ = std::fs::rename(absolute(meeting_folder, &m.removed_file), absolute(meeting_folder, &m.file));
        }
        return Err(e);
    }
    Ok(rec)
}

/// Moves the named pictures (their `file` values) back and updates the record.
/// An empty list restores every removed picture.
pub fn restore(meeting_folder: &Path, files: &[String]) -> Result<CleanupRecord, String> {
    let lock = folder_lock(meeting_folder);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut rec = load_locked(meeting_folder).ok_or("there is no picture cleanup to restore from")?;
    if rec.state != CleanupState::Done {
        return Err("there are no removed pictures to restore".to_string());
    }
    for m in rec.removed.iter_mut() {
        if m.restored || !(files.is_empty() || files.contains(&m.file)) {
            continue;
        }
        let from = absolute(meeting_folder, &m.removed_file);
        let to = absolute(meeting_folder, &m.file);
        match std::fs::rename(&from, &to) {
            Ok(()) => m.restored = true,
            Err(e) => log::warn!("picture cleanup: could not restore {}: {}", m.file, e),
        }
    }
    write_record(meeting_folder, &rec)?;
    Ok(rec)
}

/// Undo for `restore`: moves the named restored pictures (their `file`
/// values) back into `removed/` and marks them removed again.
pub fn remove_again(meeting_folder: &Path, files: &[String]) -> Result<CleanupRecord, String> {
    let lock = folder_lock(meeting_folder);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut rec = load_locked(meeting_folder).ok_or("there is no picture cleanup to undo")?;
    if rec.state != CleanupState::Done {
        return Err("there are no removed pictures to undo".to_string());
    }
    for m in rec.removed.iter_mut() {
        if !m.restored || !files.contains(&m.file) {
            continue;
        }
        let from = absolute(meeting_folder, &m.file);
        let to = absolute(meeting_folder, &m.removed_file);
        let moved = match to.parent() {
            Some(d) => std::fs::create_dir_all(d).and_then(|_| std::fs::rename(&from, &to)),
            None => Ok(()),
        };
        match moved {
            Ok(()) => m.restored = false,
            Err(e) => log::warn!("picture cleanup: could not undo the restore of {}: {}", m.file, e),
        }
    }
    write_record(meeting_folder, &rec)?;
    Ok(rec)
}

/// Writes a failed record. Moves nothing.
pub fn record_failure(meeting_folder: &Path, error: &str, now_ms: i64) -> Result<CleanupRecord, String> {
    let lock = folder_lock(meeting_folder);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let rec = CleanupRecord {
        state: CleanupState::Failed { error: error.to_string() },
        ran_at_ms: now_ms,
        delete_after_ms: 0,
        removed: Vec::new(),
    };
    write_record(meeting_folder, &rec)?;
    Ok(rec)
}

/// Reads the record, first making it agree with the disk (the disk is the
/// truth): see `load_locked`. None when there is no readable record.
pub fn load(meeting_folder: &Path) -> Option<CleanupRecord> {
    let lock = folder_lock(meeting_folder);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    load_locked(meeting_folder)
}

/// `load` for a caller that already holds the folder's lock.
///
/// An entry whose picture sits at `removed_file` is not restored; one whose
/// picture is back at `file` is restored; one found in neither place is
/// dropped. A picture in a `removed/` folder that no entry lists goes back to
/// its frames folder, so nothing is stranded. A corrected record is written.
fn load_locked(meeting_folder: &Path) -> Option<CleanupRecord> {
    let mut rec: Option<CleanupRecord> =
        std::fs::read_to_string(meeting_folder.join(RECORD_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok());
    let mut changed = false;
    let mut listed: HashSet<String> = HashSet::new();
    if let Some(r) = rec.as_mut().filter(|r| r.state == CleanupState::Done) {
        let before = r.removed.clone();
        r.removed.retain_mut(|m| {
            let in_removed = absolute(meeting_folder, &m.removed_file).is_file();
            let in_place = absolute(meeting_folder, &m.file).is_file();
            match (in_removed, in_place) {
                (true, false) => m.restored = false,
                (false, true) => m.restored = true,
                // Gone from both places: drop it. With the frames folder itself
                // missing (a folder that moved or is not mounted) nothing can be
                // judged, so the entry stays.
                (false, false) => {
                    return m.file.rsplit_once('/').map(|(dir, _)| !meeting_folder.join(dir).is_dir()).unwrap_or(true)
                }
                (true, true) => {}
            }
            true
        });
        listed = r.removed.iter().map(|m| m.removed_file.clone()).collect();
        changed = r.removed != before;
    }
    for dir in FRAME_DIRS {
        let Ok(entries) = std::fs::read_dir(meeting_folder.join(dir).join(REMOVED_DIR)) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !e.file_type().map(|t| t.is_file()).unwrap_or(false) || listed.contains(&format!("{}/{}/{}", dir, REMOVED_DIR, name)) {
                continue;
            }
            let back = meeting_folder.join(dir).join(&name);
            if back.exists() {
                continue; // never overwrite a kept picture
            }
            if let Err(err) = std::fs::rename(e.path(), &back) {
                log::warn!("picture cleanup: could not move {} back: {}", name, err);
            }
        }
    }
    if changed {
        if let Some(r) = rec.as_ref() {
            if let Err(e) = write_record(meeting_folder, r) {
                log::warn!("picture cleanup: {}", e);
            }
        }
    }
    rec
}

/// Deletes `removed/` in each frames folder when the window has ended, and
/// marks the record Purged. True when it deleted something.
pub fn purge_if_expired(meeting_folder: &Path, now_ms: i64) -> bool {
    let lock = folder_lock(meeting_folder);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut rec) = load_locked(meeting_folder) else { return false };
    if rec.state != CleanupState::Done || now_ms < rec.delete_after_ms {
        return false;
    }
    for dir in FRAME_DIRS {
        let removed = meeting_folder.join(dir).join(REMOVED_DIR);
        if let Err(e) = std::fs::remove_dir_all(&removed) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("picture cleanup: could not delete {}: {}", removed.display(), e);
                return false; // keep the record Done so the next sweep retries
            }
        }
    }
    rec.state = CleanupState::Purged;
    if let Err(e) = write_record(meeting_folder, &rec) {
        log::warn!("picture cleanup: {}", e);
    }
    true
}

/// Whether Draft note should start a cleanup run for this meeting.
pub fn should_start(record: Option<&CleanupRecord>, run_in_flight: bool) -> bool {
    if run_in_flight {
        return false;
    }
    match record {
        None => true,
        Some(r) => matches!(r.state, CleanupState::Failed { .. }),
    }
}

/// The header numbers and line for the meeting page.
pub fn summary(record: Option<&CleanupRecord>, now_ms: i64) -> CleanupSummary {
    let _ = now_ms; // a Done record shows until the purge sweep marks it Purged
    let mut out = CleanupSummary {
        line: None,
        failed: false,
        removed: 0,
        repeats: 0,
        not_useful: 0,
        bytes: 0,
        delete_after_ms: 0,
    };
    let Some(rec) = record else { return out };
    match &rec.state {
        CleanupState::Failed { .. } => {
            out.failed = true;
            out.line = Some("Picture cleanup did not finish.".to_string());
        }
        CleanupState::Purged => {}
        CleanupState::Done => {
            for m in rec.removed.iter().filter(|m| !m.restored) {
                out.removed += 1;
                out.bytes += m.bytes;
                if m.reason == Reason::Repeat {
                    out.repeats += 1;
                } else {
                    out.not_useful += 1;
                }
            }
            if out.removed > 0 {
                out.delete_after_ms = rec.delete_after_ms;
                let mut parts = Vec::new();
                if out.repeats > 0 {
                    parts.push(format!("{} {}", out.repeats, if out.repeats == 1 { "repeat" } else { "repeats" }));
                }
                if out.not_useful > 0 {
                    parts.push(format!("{} not useful", out.not_useful));
                }
                out.line = Some(format!(
                    "Claude removed {} to free {}: {}.",
                    crate::meeting_saved::format_pictures(out.removed),
                    crate::meeting_saved::format_size(out.bytes),
                    parts.join(", ")
                ));
            }
        }
    }
    out
}

/// The absolute path of a record-relative file.
pub fn absolute(meeting_folder: &Path, file: &str) -> PathBuf {
    meeting_folder.join(file)
}

#[cfg(test)]
#[path = "tests/cleanup.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/cleanup_round2.rs"]
mod round2_tests;
