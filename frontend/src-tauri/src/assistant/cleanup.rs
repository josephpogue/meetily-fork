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
//
// Entry points only: every function body is still to be written.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Days a removed picture can be restored before it is deleted for good.
pub const RESTORE_DAYS: i64 = 5;
/// Folder, inside `zoom-frames/` or `teams-frames/`, that holds removed pictures.
pub const REMOVED_DIR: &str = "removed";
/// The cleanup record, in the meeting folder beside `assistant.json`.
pub const RECORD_FILE: &str = "picture-cleanup.json";
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

/// The kept pictures in `meeting_folder` (top level of each frames folder,
/// never `removed/`), oldest first, numbered P1, P2, ...
pub fn list_pictures(meeting_folder: &Path) -> Vec<Picture> {
    let _ = meeting_folder;
    unimplemented!("not implemented")
}

/// The part of the cleanup run's message that lists the pictures, one line
/// each: "P1 = Teams window at 11:03:00".
pub fn picture_list(pictures: &[Picture]) -> String {
    let _ = pictures;
    unimplemented!("not implemented")
}

/// Parses the run's reply: a JSON object `{"remove":[{"id":"P3","reason":"loading",
/// "caption":"Loading content..."},{"id":"P7","reason":"repeat","repeat_of":"P8"}]}`,
/// possibly with text around it. Err when no such object can be read.
pub fn parse_reply(raw: &str, pictures: &[Picture]) -> Result<Vec<Removal>, String> {
    let _ = (raw, pictures);
    unimplemented!("not implemented")
}

/// Moves each removal's picture into `removed/` in its own frames folder and
/// writes the record. Returns the record written.
pub fn apply(meeting_folder: &Path, pictures: &[Picture], removals: &[Removal], now_ms: i64) -> Result<CleanupRecord, String> {
    let _ = (meeting_folder, pictures, removals, now_ms);
    unimplemented!("not implemented")
}

/// Moves the named pictures (their `file` values) back and updates the record.
/// An empty list restores every removed picture.
pub fn restore(meeting_folder: &Path, files: &[String]) -> Result<CleanupRecord, String> {
    let _ = (meeting_folder, files);
    unimplemented!("not implemented")
}

/// Writes a failed record. Moves nothing.
pub fn record_failure(meeting_folder: &Path, error: &str, now_ms: i64) -> Result<CleanupRecord, String> {
    let _ = (meeting_folder, error, now_ms);
    unimplemented!("not implemented")
}

/// Reads the record. None when there is none or it cannot be read.
pub fn load(meeting_folder: &Path) -> Option<CleanupRecord> {
    let _ = meeting_folder;
    unimplemented!("not implemented")
}

/// Deletes `removed/` in each frames folder when the window has ended, and
/// marks the record Purged. True when it deleted something.
pub fn purge_if_expired(meeting_folder: &Path, now_ms: i64) -> bool {
    let _ = (meeting_folder, now_ms);
    unimplemented!("not implemented")
}

/// Whether Draft note should start a cleanup run for this meeting.
pub fn should_start(record: Option<&CleanupRecord>, run_in_flight: bool) -> bool {
    let _ = (record, run_in_flight);
    unimplemented!("not implemented")
}

/// The header numbers and line for the meeting page.
pub fn summary(record: Option<&CleanupRecord>, now_ms: i64) -> CleanupSummary {
    let _ = (record, now_ms);
    unimplemented!("not implemented")
}

/// The absolute path of a record-relative file.
pub fn absolute(meeting_folder: &Path, file: &str) -> PathBuf {
    meeting_folder.join(file)
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
