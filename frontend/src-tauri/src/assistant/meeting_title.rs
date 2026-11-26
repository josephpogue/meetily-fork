// assistant/meeting_title.rs
//
// Names a meeting in Joseph's framework, "{CODE} - {Topic} {M/D/YY}", from the
// topic the note writer returns. Pure apart from reading the codes file.
// Design: docs/superpowers/specs/2026-11-25-meeting-title-design.md

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate};
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
    match std::env::var_os("PROJECT_CODES_FILE") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(".claude/routing/project-codes.json"),
    }
}

const TITLE_MARKER: &str = "=== TITLE ===";
const SECTION_MARKERS: [&str; 3] = ["=== PROJECT ===", "=== NOTE ===", "=== SLUG ==="];

/// Collapses every run of whitespace to one space.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The project codes by slug. Missing, unreadable or malformed means none.
fn read_codes(file: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return BTreeMap::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return BTreeMap::new();
    };
    json.get("codes")
        .and_then(|c| c.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|c| (k.clone(), c.trim().to_string())))
                .filter(|(_, c)| !c.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The slug's own code, else the code of the longest slug that is a prefix of it plus "-".
fn code_for<'a>(codes: &'a BTreeMap<String, String>, slug: &str) -> Option<&'a str> {
    if let Some(c) = codes.get(slug) {
        return Some(c);
    }
    codes
        .iter()
        .filter(|(k, _)| slug.strip_prefix(k.as_str()).is_some_and(|rest| rest.starts_with('-')))
        .max_by_key(|(k, _)| k.len())
        .map(|(_, c)| c.as_str())
}

/// The call-window name the app recorded in the meeting folder's `metadata.json`
/// at Stop, if there is one.
pub fn window_name_in(folder: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(folder.join("metadata.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    value.get("window_name")?.as_str().map(str::to_string).filter(|n| !n.is_empty())
}

/// The topic in the note writer's reply: the text of its `=== TITLE ===`
/// section, whitespace collapsed. None when the section is missing or empty.
pub fn topic_from_reply(raw: &str) -> Option<String> {
    let start = raw.find(TITLE_MARKER)? + TITLE_MARKER.len();
    let rest = &raw[start..];
    let end = SECTION_MARKERS.iter().filter_map(|m| rest.find(m)).min().unwrap_or(rest.len());
    Some(squash(&rest[..end])).filter(|t| !t.is_empty())
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
    let replaceable = super::meeting_name::is_automatic_name(current) || window_name == Some(current);
    let topic = topic.map(squash).filter(|t| !t.is_empty());
    let (Some(topic), true) = (topic, replaceable) else {
        return TitleSuggestions {
            by_project: projects.iter().map(|p| (p.clone(), current.to_string())).collect(),
            no_project: current.to_string(),
            current: current.to_string(),
            keep_current: true,
        };
    };
    let codes = read_codes(codes_file);
    let when = format!("{}/{}/{}", date.month(), date.day(), date.year() % 100);
    let title = |code: Option<&str>| match code {
        Some(c) => format!("{} - {} {}", c, topic, when),
        None => format!("{} {}", topic, when),
    };
    TitleSuggestions {
        by_project: projects.iter().map(|p| (p.clone(), title(code_for(&codes, p)))).collect(),
        no_project: title(None),
        current: current.to_string(),
        keep_current: false,
    }
}

/// The name Save or Discard gives the meeting, or None to leave it alone.
///
/// - `box_text`: what the Title box holds when Joseph presses the button.
/// - `edited`: whether Joseph changed the box himself.
/// - `current_now`: the meeting's name when he presses it (he may have
///   renamed it in the sidebar since the draft).
pub fn name_to_apply(s: &TitleSuggestions, box_text: &str, edited: bool, current_now: &str) -> Option<String> {
    let text = box_text.trim();
    if text.is_empty() {
        return None;
    }
    let unchanged = text == current_now.trim();
    if edited {
        return (!unchanged).then(|| text.to_string());
    }
    // Unedited: the suggestion applies only while the name is still the one
    // the draft saw (a sidebar rename since then stays).
    (!s.keep_current && current_now == s.current && !unchanged).then(|| text.to_string())
}

#[cfg(test)]
#[path = "tests/meeting_title.rs"]
mod tests;
