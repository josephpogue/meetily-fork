// assistant/meeting_title.rs
//
// Names a meeting in Joseph's framework, "{CODE} - {Topic} {M/D/YY}", from the
// topic the note writer returns. Pure apart from reading the codes file.
// Design: docs/superpowers/specs/2026-11-25-meeting-title-design.md

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde::Serialize;

/// What the note bar's Title box can show for one drafted note.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleSuggestions {
    /// The framework title for each listed project slug.
    pub by_project: BTreeMap<String, String>,
    /// The title when No project is picked.
    pub no_project: String,
    /// The meeting's name when the note was drafted.
    pub current: String,
    /// True when `current` stays: Joseph typed it, or the writer gave no
    /// topic. Every value above is then `current`.
    pub keep_current: bool,
}

/// `$PROJECT_CODES_FILE`, else `~/.claude/routing/project-codes.json`.
pub fn default_codes_file() -> PathBuf {
    todo!("not implemented")
}

/// The topic in the note writer's reply: the text of its `=== TITLE ===`
/// section, whitespace collapsed. None when the section is missing or empty.
pub fn topic_from_reply(raw: &str) -> Option<String> {
    let _ = raw;
    todo!("not implemented")
}

/// The Title box suggestions for a drafted note.
///
/// - `current`: the meeting's name now.
/// - `window_name`: the name the app took from the call window at Stop, if any.
/// - `topic`: from `topic_from_reply`.
/// - `date`: the meeting's local start date.
/// - `projects`: the slugs the note bar lists.
/// - `codes_file`: the project codes file; missing or unreadable means no codes.
pub fn suggest(
    current: &str,
    window_name: Option<&str>,
    topic: Option<&str>,
    date: NaiveDate,
    projects: &[String],
    codes_file: &Path,
) -> TitleSuggestions {
    let _ = (current, window_name, topic, date, projects, codes_file);
    todo!("not implemented")
}

/// The name Save or Discard gives the meeting, or None to leave it alone.
///
/// - `box_text`: what the Title box holds when Joseph presses the button.
/// - `edited`: whether Joseph changed the box himself.
/// - `current_now`: the meeting's name when he presses it (he may have
///   renamed it in the sidebar since the draft).
pub fn name_to_apply(s: &TitleSuggestions, box_text: &str, edited: bool, current_now: &str) -> Option<String> {
    let _ = (s, box_text, edited, current_now);
    todo!("not implemented")
}

#[cfg(test)]
#[path = "tests/meeting_title.rs"]
mod tests;
