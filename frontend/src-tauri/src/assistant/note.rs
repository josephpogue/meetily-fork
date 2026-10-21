// assistant/note.rs
//
// Note drafting parse and vault save. `AnswerLanes::draft_note` (Task 7)
// runs the deep lane once at end of meeting with `NOTE_PROMPT` appended as
// its system prompt; the raw reply lands here to parse and, once Joseph
// presses Save, to write.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io};

use super::ask_list::{AnswerHow, AnsweredItem};
use crate::database::repositories::assistant_items::AssistantItem;

/// System prompt for `AnswerLanes::draft_note`. Populates `LaneConfig`'s
/// `note_prompt` field so `lanes.rs` never depends on this file directly.
pub const NOTE_PROMPT: &str = include_str!("prompts/note.md");

#[derive(Debug, Clone)]
pub struct NoteDraft {
    pub slug: String,
    pub markdown: String,
}

const SLUG_MARKER: &str = "=== SLUG ===";
const NOTE_MARKER: &str = "=== NOTE ===";

/// Parses the deep lane's raw reply into a slug and a note body.
pub fn parse_note(raw: &str) -> Result<NoteDraft, String> {
    let slug_pos = raw
        .find(SLUG_MARKER)
        .ok_or_else(|| format!("missing {} marker", SLUG_MARKER))?;
    let note_pos = raw
        .find(NOTE_MARKER)
        .ok_or_else(|| format!("missing {} marker", NOTE_MARKER))?;
    if note_pos <= slug_pos {
        return Err(format!("{} must come after {}", NOTE_MARKER, SLUG_MARKER));
    }

    let slug = raw[slug_pos + SLUG_MARKER.len()..note_pos].trim();
    let markdown = raw[note_pos + NOTE_MARKER.len()..].trim();

    if slug.is_empty() {
        return Err("empty slug".to_string());
    }
    if markdown.is_empty() {
        return Err("empty note body".to_string());
    }

    Ok(NoteDraft {
        slug: slug.to_string(),
        markdown: markdown.to_string(),
    })
}

/// The "Answered questions" section, built in code from the assistant's
/// answered list (oldest first), never by the model. Empty for an empty list.
pub fn answered_section(items: &[AnsweredItem]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = items
        .iter()
        .map(|a| {
            let how = match a.how {
                AnswerHow::Call => "On the call",
                AnswerHow::Chat => "Worked out in Ask",
                AnswerHow::You => "You answered",
            };
            format!("- **{}** {} ({})", super::ask_list::collapse(&a.question), super::ask_list::collapse(&a.answer), how)
        })
        .collect();
    format!("## Answered questions\n\n{}", lines.join("\n"))
}

/// The note body with the answered section appended; unchanged when nothing
/// was answered.
pub fn with_answered(markdown: &str, items: &[AnsweredItem]) -> String {
    let section = answered_section(items);
    if section.is_empty() {
        markdown.to_string()
    } else {
        format!("{}\n\n{}", markdown.trim_end(), section)
    }
}

/// The model's slug made safe for a file name and a commit message: lowercase
/// `a-z0-9-`, other runs become one `-`, trimmed, at most 60 characters,
/// `meeting` when nothing is left.
fn sanitize_slug(slug: &str) -> String {
    let mut out = String::new();
    for c in slug.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(60);
    let out = out.trim_matches('-');
    if out.is_empty() { "meeting".to_string() } else { out.to_string() }
}

/// "team-standup" -> "Team standup".
fn title_from_slug(slug: &str) -> String {
    let spaced = slug.replace('-', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}

// ---------------------------------------------------------------------
// Vault projects
// ---------------------------------------------------------------------

fn projects_dir(vault_root: &Path) -> PathBuf {
    vault_root.join("Wiki").join("Projects")
}

fn is_project_slug(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Slugs of `<vault_root>/Wiki/Projects/*.md`, sorted, kebab-case names only.
/// A missing folder gives an empty list.
pub fn list_projects(vault_root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(projects_dir(vault_root)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".md")).map(str::to_string))
        .filter(|slug| is_project_slug(slug))
        .collect();
    out.sort();
    out
}

/// Lowercase alphanumeric words of `text`.
fn words(text: &str) -> std::collections::HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The words of the first `# ` line of a project page; empty when unreadable.
fn page_h1_words(vault_root: &Path, slug: &str) -> std::collections::HashSet<String> {
    let Ok(text) = fs::read_to_string(projects_dir(vault_root).join(format!("{slug}.md"))) else {
        return Default::default();
    };
    text.lines().find_map(|l| l.strip_prefix("# ")).map(words).unwrap_or_default()
}

/// The project the meeting is about, scored over the `Wiki/Projects` pages.
/// A slug's tokens are its `-` parts (3+ characters, or a word of the page's
/// H1) plus the H1's 3+ character words. A token found in 3+ slugs is common
/// and scores 1, any other 3. Title and Brief weigh 2, the transcript 1; a
/// token scores once per place it appears. Best total wins, ties go to the slug
/// with fewer `-` parts then the alphabet, and under 3 gives None.
pub fn suggest_project(vault_root: &Path, title: &str, brief: &str, transcript: &str) -> Option<String> {
    let head = words(&format!("{title} {brief}"));
    let body = words(transcript);
    let slugs: Vec<(String, usize, std::collections::HashSet<String>)> = list_projects(vault_root)
        .into_iter()
        .map(|slug| {
            let h1 = page_h1_words(vault_root, &slug);
            let parts: Vec<&str> = slug.split('-').filter(|p| !p.is_empty()).collect();
            let mut tokens: std::collections::HashSet<String> = parts
                .iter()
                .filter(|p| p.len() >= 3 || h1.contains(**p))
                .map(|p| p.to_string())
                .collect();
            tokens.extend(h1.into_iter().filter(|w| w.chars().count() >= 3));
            let n = parts.len();
            (slug, n, tokens)
        })
        .collect();
    let owners = |token: &str| slugs.iter().filter(|(_, _, t)| t.contains(token)).count();
    let mut best: Option<(u32, usize, &String)> = None;
    for (slug, parts, tokens) in &slugs {
        let score: u32 = tokens
            .iter()
            .map(|t| {
                let value = if owners(t) >= 3 { 1 } else { 3 };
                value * (2 * head.contains(t) as u32 + body.contains(t) as u32)
            })
            .sum();
        // Higher score, then fewer parts, then alphabetical (the list is sorted, so first wins).
        if best.map_or(true, |(s, n, _)| score > s || (score == s && *parts < n)) {
            best = Some((score, *parts, slug));
        }
    }
    best.filter(|(score, _, _)| *score >= 3).map(|(_, _, slug)| slug.clone())
}

pub fn check_project(vault_root: &Path, project: Option<&str>) -> Result<(), String> {
    match project {
        Some(p) if !list_projects(vault_root).iter().any(|s| s == p) => {
            Err(format!("\"{}\" is not a project in the vault", p))
        }
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------
// Sections built in code from the saved rows
// ---------------------------------------------------------------------

const ANSWERED_HEADING: &str = "## Answered questions";
const QA_HEADING: &str = "## Q&A log";

/// Rows of kind question or for_you that hold an answer, oldest answer first.
fn answered_from_rows(rows: &[AssistantItem]) -> Vec<AnsweredItem> {
    let answered_at = |r: &AssistantItem| r.answered_call_time.unwrap_or(r.call_time);
    let mut kept: Vec<&AssistantItem> = rows
        .iter()
        .filter(|r| matches!(r.kind.as_str(), "question" | "for_you") && !r.answer.trim().is_empty())
        .collect();
    kept.sort_by(|a, b| answered_at(a).total_cmp(&answered_at(b)));
    kept.into_iter()
        .map(|r| AnsweredItem {
            id: r.id.clone(),
            question: r.question.clone(),
            answer: r.answer.clone(),
            how: match r.answered_how.as_str() {
                "chat" => AnswerHow::Chat,
                "you" => AnswerHow::You,
                _ => AnswerHow::Call,
            },
            at: (answered_at(r) * 1000.0) as i64,
        })
        .collect()
}

/// The Q&A log: settled answer and ask cards, `Q: ...` then `A: ...`.
fn qa_log_from_rows(rows: &[AssistantItem]) -> String {
    rows.iter()
        .filter(|r| matches!(r.kind.as_str(), "answer" | "ask") && matches!(r.phase.as_str(), "checked" | "corrected"))
        .map(|r| format!("Q: {}\nA: {}", r.question, r.lead))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `## Answered questions` then `## Q&A log`, each only when it has content,
/// built from the meeting's rows (to-dos never appear). Empty when neither has.
pub fn code_sections(rows: &[AssistantItem]) -> String {
    let mut parts = Vec::new();
    let answered = answered_section(&answered_from_rows(rows));
    if !answered.is_empty() {
        parts.push(answered);
    }
    let log = qa_log_from_rows(rows);
    if !log.is_empty() {
        parts.push(format!("{}\n\n{}", QA_HEADING, log));
    }
    parts.join("\n\n")
}

// ---------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------

/// Everything `save_note` writes from. `rows` are the meeting's saved items
/// (deleted ones already gone).
pub struct SaveInput<'a> {
    pub vault_root: &'a Path,
    pub date: &'a str,
    pub draft: &'a NoteDraft,
    pub title: Option<&'a str>,
    pub brief: &'a str,
    pub rows: &'a [AssistantItem],
    pub transcript: &'a str,
    pub project: Option<&'a str>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Filed {
    pub note_path: PathBuf,
    pub transcript_path: PathBuf,
    pub project: Option<String>,
}

/// The first `<date>-<slug>` stem whose note and transcript files are both
/// free: `-2`, `-3` ... when it is taken.
fn free_stem(meetings_dir: &Path, date: &str, slug: &str) -> String {
    let taken = |stem: &str| {
        meetings_dir.join(format!("{stem}.md")).exists() || meetings_dir.join(format!("{stem}_transcript.md")).exists()
    };
    let base = format!("{date}-{slug}");
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|stem| !taken(stem))
        .expect("an unbounded range yields a free stem")
}

fn compose_note(input: &SaveInput, title: &str, stem: &str) -> String {
    let mut out = format!(
        "# {}\n\n- Date: {}\n- Source: meetily-fork recording, saved from the app on {}.\n- Full transcript: [[{}_transcript]]",
        title, input.date, input.date, stem
    );
    if let Some(p) = input.project {
        out.push_str(&format!("\n- Linked projects: [[{}]]", p));
    }
    if !input.brief.trim().is_empty() {
        out.push_str(&format!("\n\n## Brief\n\n{}", input.brief.trim()));
    }
    out.push_str(&format!("\n\n{}", input.draft.markdown.trim()));
    let sections = code_sections(input.rows);
    if !sections.is_empty() {
        out.push_str(&format!("\n\n{}", sections));
    }
    out.push('\n');
    out
}

/// Adds `- <date>: <title>: [[<stem>]]` as the last line of the page's
/// `## Meetings` section, creating the section at the end when missing. None
/// when the page already links `[[<stem>]]`. Nothing else changes.
fn with_meeting_link(page: &str, date: &str, title: &str, stem: &str) -> Option<String> {
    if page.contains(&format!("[[{}]]", stem)) {
        return None;
    }
    let line = format!("- {}: {}: [[{}]]", date, title, stem);
    let mut lines: Vec<&str> = page.split('\n').collect();
    let Some(heading) = lines.iter().position(|l| l.trim_end() == "## Meetings") else {
        return Some(format!("{}\n\n## Meetings\n\n{}\n", page.trim_end(), line));
    };
    let end = lines[heading + 1..]
        .iter()
        .position(|l| l.starts_with("# ") || l.starts_with("## "))
        .map_or(lines.len(), |i| heading + 1 + i);
    let last = (heading..end).rev().find(|&i| !lines[i].trim().is_empty()).unwrap_or(heading);
    if last == heading {
        lines.splice(heading + 1..heading + 1, ["", line.as_str()]);
    } else {
        lines.insert(last + 1, line.as_str());
    }
    Some(lines.join("\n"))
}

/// Files the meeting in the vault: `_sources/meetings/<date>-<slug>.md` and its
/// `_transcript.md`, a `## Meetings` link on the chosen project's page, and one
/// local git commit of exactly those files. A project that is not a page in
/// `Wiki/Projects/` is refused before anything is written. `dry_run` writes and
/// commits nothing and returns the paths it would have written. A git failure
/// comes back as `Err`, never a panic.
pub fn save_note(input: SaveInput) -> Result<Filed, String> {
    check_project(input.vault_root, input.project)?;
    let meetings_dir = input.vault_root.join("_sources").join("meetings");
    let slug = sanitize_slug(&input.draft.slug);
    let stem = free_stem(&meetings_dir, input.date, &slug);
    let note_path = meetings_dir.join(format!("{stem}.md"));
    let transcript_path = meetings_dir.join(format!("{stem}_transcript.md"));
    let filed = Filed {
        note_path: note_path.clone(),
        transcript_path: transcript_path.clone(),
        project: input.project.map(str::to_string),
    };
    if input.dry_run {
        return Ok(filed);
    }

    let title = input
        .title
        .map(super::ask_list::collapse)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| title_from_slug(&slug));
    // Read the page first: an unreadable page stops the save before any write.
    let page = match input.project {
        Some(p) => {
            let path = projects_dir(input.vault_root).join(format!("{p}.md"));
            let text = fs::read_to_string(&path).map_err(|e| format!("could not read {}: {}", path.display(), e))?;
            Some((path, text))
        }
        None => None,
    };

    fs::create_dir_all(&meetings_dir)
        .map_err(|e| format!("could not create {}: {}", meetings_dir.display(), e))?;
    fs::write(&note_path, compose_note(&input, &title, &stem))
        .map_err(|e| format!("could not write {}: {}", note_path.display(), e))?;
    fs::write(&transcript_path, input.transcript)
        .map_err(|e| format!("could not write {}: {}", transcript_path.display(), e))?;

    let mut paths = vec![note_path, transcript_path];
    if let Some((path, text)) = page {
        if let Some(updated) = with_meeting_link(&text, input.date, &title, &stem) {
            fs::write(&path, updated).map_err(|e| format!("could not write {}: {}", path.display(), e))?;
            paths.push(path);
        }
    }
    commit_paths(input.vault_root, &paths, &format!("Meeting note: {}", slug))?;
    Ok(filed)
}

// ---------------------------------------------------------------------
// Refiling after a delete
// ---------------------------------------------------------------------

/// The note text with its two code-built sections (`## Answered questions`,
/// `## Q&A log`) rebuilt from `remaining` in place, each only up to the next
/// heading, so every other section stays byte for byte; and the
/// `## Open questions` bullets that are one of `deleted_questions` gone. A
/// code section the note lacks but the rows need goes at the end.
fn refiled_text(text: &str, remaining: &[AssistantItem], deleted_questions: &[String]) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let is_heading = |l: &str| l.starts_with("# ") || l.starts_with("## ");
    let answered = answered_section(&answered_from_rows(remaining));
    let log = qa_log_from_rows(remaining);
    let log = if log.is_empty() { String::new() } else { format!("{}\n\n{}", QA_HEADING, log) };
    let gone: Vec<String> = deleted_questions.iter().map(|q| super::history::normalize_question(q)).collect();
    let mut kept: Vec<String> = Vec::new();
    let (mut placed_answered, mut placed_log) = (false, false);
    let mut in_open = false;
    let mut open_start = 0usize;
    let mut removed_in_open = false;
    // An Open questions section left with no content loses its heading.
    fn close_open(kept: &mut Vec<String>, start: usize, removed: bool) {
        if removed && kept[start + 1..].iter().all(|l| l.trim().is_empty()) {
            kept.truncate(start);
        }
    }
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if is_heading(line) {
            if in_open {
                close_open(&mut kept, open_start, removed_in_open);
            }
            in_open = false;
            let heading = line.trim_end();
            if heading == ANSWERED_HEADING || heading == QA_HEADING {
                let end = lines[i + 1..].iter().position(|l| is_heading(l)).map_or(lines.len(), |p| i + 1 + p);
                // Blank lines closing the section stay as the gap before what follows.
                let blanks = lines[i..end].iter().rev().take_while(|l| l.trim().is_empty()).count();
                let (section, placed) = if heading == ANSWERED_HEADING {
                    (&answered, &mut placed_answered)
                } else {
                    (&log, &mut placed_log)
                };
                *placed = true;
                if !section.is_empty() {
                    kept.extend(section.split('\n').map(str::to_string));
                    kept.extend(lines[end - blanks..end].iter().map(|l| l.to_string()));
                }
                i = end;
                continue;
            }
            in_open = heading == "## Open questions";
            open_start = kept.len();
            removed_in_open = false;
        } else if in_open {
            if let Some(bullet) = line.strip_prefix("- ") {
                if gone.contains(&super::history::normalize_question(bullet)) {
                    removed_in_open = true;
                    i += 1;
                    continue;
                }
            }
        }
        kept.push(line.to_string());
        i += 1;
    }
    if in_open {
        close_open(&mut kept, open_start, removed_in_open);
    }
    let mut out = kept.join("\n").trim_end().to_string();
    for (placed, section) in [(placed_answered, &answered), (placed_log, &log)] {
        if !placed && !section.is_empty() {
            out.push_str(&format!("\n\n{}", section));
        }
    }
    out.push('\n');
    out
}

/// Rewrites a filed note after items were deleted and commits just that file
/// (`Meeting note: <stem>, items deleted`). Ok(false) when the file is missing
/// or nothing in it changes. `dry_run` changes and commits nothing.
pub fn refile(
    note_path: &Path,
    remaining: &[AssistantItem],
    deleted: &[AssistantItem],
    dry_run: bool,
) -> Result<bool, String> {
    let Ok(text) = fs::read_to_string(note_path) else {
        return Ok(false);
    };
    let questions: Vec<String> = deleted
        .iter()
        .filter(|r| matches!(r.kind.as_str(), "question" | "for_you"))
        .map(|r| r.question.clone())
        .collect();
    let updated = refiled_text(&text, remaining, &questions);
    if updated == text || dry_run {
        return Ok(updated != text);
    }
    fs::write(note_path, updated).map_err(|e| format!("could not write {}: {}", note_path.display(), e))?;
    let dir = note_path.parent().ok_or_else(|| "the note has no folder".to_string())?;
    let stem = note_path.file_stem().and_then(|s| s.to_str()).unwrap_or("note");
    commit_paths(dir, &[note_path.to_path_buf()], &format!("Meeting note: {}, items deleted", stem))?;
    Ok(true)
}

/// `git add` then `git commit -- <paths>` in the repo holding `dir`, so
/// anything else staged or dirty there stays out of the commit. Paths are
/// given relative to `dir`'s repo root when they sit under `dir`.
fn commit_paths(dir: &Path, paths: &[PathBuf], message: &str) -> Result<(), String> {
    // `dir` may be a subfolder of the repo (a refile runs in the note's folder);
    // pass each path by absolute name made relative to `dir` when possible.
    let rel: Vec<String> = paths
        .iter()
        .map(|p| p.strip_prefix(dir).unwrap_or(p).to_string_lossy().into_owned())
        .collect();
    let mut add = vec!["add", "--"];
    add.extend(rel.iter().map(String::as_str));
    run_git(dir, &add)?;
    let mut commit = vec!["commit", "-m", message, "--"];
    commit.extend(rel.iter().map(String::as_str));
    run_git(dir, &commit)
}

fn run_git(dir: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e: io::Error| format!("could not run git {}: {}", args.join(" "), e))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answered_section_lists_oldest_first_with_how_labels() {
        let item = |q: &str, a: &str, how| AnsweredItem { id: q.into(), question: q.into(), answer: a.into(), how, at: 1 };
        let items = vec![
            item("Where is the data center?", "In Atlanta.", AnswerHow::Call),
            item("Who owns backups?", "The ops team.", AnswerHow::Chat),
            item("Is pricing in scope?", "Yes.", AnswerHow::You),
        ];
        assert_eq!(
            answered_section(&items),
            "## Answered questions\n\n- **Where is the data center?** In Atlanta. (On the call)\n- **Who owns backups?** The ops team. (Worked out in Ask)\n- **Is pricing in scope?** Yes. (You answered)"
        );
        assert_eq!(with_answered("## Summary\n\nx\n", &items[..1]), "## Summary\n\nx\n\n## Answered questions\n\n- **Where is the data center?** In Atlanta. (On the call)");
    }

    #[test]
    fn nothing_is_appended_when_nothing_was_answered() {
        assert_eq!(answered_section(&[]), "");
        assert_eq!(with_answered("## Summary\n\nx", &[]), "## Summary\n\nx");
    }

    #[test]
    fn parse_note_extracts_slug_and_body() {
        let raw = "=== SLUG ===\nteam-standup\n=== NOTE ===\n## Summary\n\nThings happened.\n";
        let draft = parse_note(raw).unwrap();
        assert_eq!(draft.slug, "team-standup");
        assert!(draft.markdown.starts_with("## Summary"));
        assert!(draft.markdown.contains("Things happened."));
    }

    #[test]
    fn parse_note_rejects_a_missing_marker() {
        let no_slug = "=== NOTE ===\n## Summary\n\nText.\n";
        assert!(parse_note(no_slug).is_err());

        let no_note = "=== SLUG ===\nteam-standup\n";
        assert!(parse_note(no_note).is_err());
    }

    fn init_git_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .status()
                .expect("git should be on PATH for this test");
            assert!(status.success(), "git {} failed", args.join(" "));
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test"]);
    }

    #[test]
    fn each_answered_item_is_one_line_in_the_note() {
        let items = vec![AnsweredItem {
            id: "a".to_string(),
            question: "What storage?".to_string(),
            answer: "NetApp,\n  about 40 TB.\n\n- Pure too".to_string(),
            how: AnswerHow::Call,
            at: 1,
        }];
        let section = answered_section(&items);
        assert_eq!(
            section,
            "## Answered questions\n\n- **What storage?** NetApp, about 40 TB. - Pure too (On the call)"
        );
        assert_eq!(section.lines().count(), 3);
    }

    // ---- vault filing ----

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn row(id: &str, kind: &str) -> AssistantItem {
        AssistantItem {
            id: id.into(),
            kind: kind.into(),
            call_time: 1.0,
            shown_at: "2026-10-21T15:00:00.000Z".into(),
            question: String::new(),
            lead: String::new(),
            bullets: vec![],
            why: String::new(),
            source: String::new(),
            phase: String::new(),
            changed_lines: vec![],
            answer: String::new(),
            answered_how: String::new(),
            answered_call_time: None,
            owner: String::new(),
            owner_name: String::new(),
            due: String::new(),
        }
    }

    fn answered_row(id: &str, q: &str, a: &str, at: f64) -> AssistantItem {
        let mut r = row(id, "question");
        r.question = q.into();
        r.answer = a.into();
        r.answered_how = "call".into();
        r.answered_call_time = Some(at);
        r
    }

    fn card_row(id: &str, q: &str, lead: &str) -> AssistantItem {
        let mut r = row(id, "answer");
        r.question = q.into();
        r.lead = lead.into();
        r.phase = "checked".into();
        r
    }

    fn todo_row(id: &str, text: &str) -> AssistantItem {
        let mut r = row(id, "todo");
        r.lead = text.into();
        r.question = text.into();
        r
    }

    /// A git-initialised vault with project pages and an index linking PNC.
    fn vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let projects = dir.path().join("Wiki/Projects");
        fs::create_dir_all(&projects).unwrap();
        fs::write(projects.join("broadcom-pnc-bank.md"), "# PNC\n\nAbout PNC.\n\n## Meetings\n\n- 2026-10-20: intro call: [[2026-10-20-pnc-intro]]\n\n## Notes\n\nlater\n").unwrap();
        fs::write(projects.join("mission-control.md"), "# MC\n").unwrap();
        fs::write(projects.join("README.md"), "x").unwrap();
        fs::write(projects.join(".hidden.md"), "x").unwrap();
        fs::write(projects.join("Notes Draft.md"), "x").unwrap();
        fs::write(dir.path().join("Wiki/index.md"), "# Wiki\n- [[Projects/broadcom-pnc-bank]] PNC\n").unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "seed"]);
        dir
    }

    fn pnc_rows() -> Vec<AssistantItem> {
        vec![
            answered_row("a1", "What storage?", "NetApp.", 5.0),
            card_row("c1", "Where is the DR site?", "Charlotte."),
            todo_row("t1", "Send the diagram"),
        ]
    }

    fn pnc_draft() -> NoteDraft {
        NoteDraft {
            slug: "pnc-sync".into(),
            markdown: "## Summary\n\nWave order.\n\n## Open questions\n\n- Is pricing in scope?\n- Who signs off?\n".into(),
        }
    }

    fn input<'a>(
        dir: &'a Path,
        draft: &'a NoteDraft,
        rows: &'a [AssistantItem],
        project: Option<&'a str>,
        dry_run: bool,
    ) -> SaveInput<'a> {
        SaveInput {
            vault_root: dir,
            date: "2026-10-21",
            draft,
            title: Some("PNC sync with Jim"),
            brief: "Confirm wave order",
            rows,
            transcript: "You: hi",
            project,
            dry_run,
        }
    }

    #[test]
    fn project_list_keeps_only_kebab_case_pages_sorted() {
        let dir = vault();
        assert_eq!(list_projects(dir.path()), vec!["broadcom-pnc-bank", "mission-control"]);
        assert!(list_projects(&dir.path().join("nowhere")).is_empty());
    }

    #[test]
    fn suggestion_comes_from_title_and_brief_then_transcript() {
        let dir = vault();
        let root = dir.path();
        // resolve_project matches the whole slug as words, so the title names it.
        assert_eq!(suggest_project(root, "Broadcom PNC Bank sync with Jim", "", "").as_deref(), Some("broadcom-pnc-bank"));
        assert_eq!(suggest_project(root, "Weekly sync", "Confirm wave order", "we talked about broadcom pnc bank").as_deref(), Some("broadcom-pnc-bank"));
        assert_eq!(suggest_project(root, "Weekly sync", "Confirm wave order", "nothing relevant"), None);
        // Linked in the index but with no page: never suggested.
        fs::remove_file(root.join("Wiki/Projects/broadcom-pnc-bank.md")).unwrap();
        assert_eq!(suggest_project(root, "Broadcom PNC Bank sync", "", ""), None);
    }

    /// The seven-project vault from the brief, each page with its own H1.
    fn scored_vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("Wiki/Projects");
        fs::create_dir_all(&projects).unwrap();
        for (slug, h1) in [
            ("broadcom", "Broadcom"),
            ("broadcom-pnc-bank", "PNC Bank"),
            ("broadcom-cleveland-clinic", "Cleveland Clinic"),
            ("broadcom-principal-financial-group", "Principal Financial Group"),
            ("mission-control", "Mission Control"),
            ("homelab", "Homelab"),
            ("homelab-facts", "Homelab facts"),
        ] {
            fs::write(projects.join(format!("{slug}.md")), format!("---\ntags: x\n---\n# {h1}\n\nBody.\n")).unwrap();
        }
        dir
    }

    #[test]
    fn suggestion_scores_realistic_titles_over_the_project_list() {
        let dir = scored_vault();
        let root = dir.path();
        // A distinctive token (pnc) in the title beats the common one (broadcom).
        assert_eq!(suggest_project(root, "PNC sync with Jim", "", "").as_deref(), Some("broadcom-pnc-bank"));
        // Only a common token in the title: 1 x 2 = 2, below the bar.
        assert_eq!(suggest_project(root, "Broadcom sync", "", ""), None);
        // Common token in title (2) and transcript (1) reaches 3; ties go to the fewest tokens.
        assert_eq!(suggest_project(root, "Broadcom weekly", "", "the broadcom team").as_deref(), Some("broadcom"));
        // Transcript only, two distinctive tokens at weight 1 each.
        assert_eq!(
            suggest_project(root, "Weekly", "", "we met Cleveland Clinic. Cleveland Clinic again").as_deref(),
            Some("broadcom-cleveland-clinic")
        );
        assert_eq!(suggest_project(root, "Mission Control perf", "", "").as_deref(), Some("mission-control"));
        assert_eq!(suggest_project(root, "Dentist", "", "lunch plans").as_deref(), None);
        // Whole words only, any case.
        assert_eq!(suggest_project(root, "pncx syncing", "", ""), None);
        assert_eq!(suggest_project(root, "MISSION CONTROL", "", "").as_deref(), Some("mission-control"));
    }

    #[test]
    fn a_short_slug_token_counts_only_when_the_page_h1_has_it() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("Wiki/Projects");
        fs::create_dir_all(&projects).unwrap();
        fs::write(projects.join("q3-planning.md"), "# Q3 planning\n").unwrap();
        fs::write(projects.join("x1-garden.md"), "# Garden\n").unwrap();
        // q3 is in the H1 as a word, so it counts (3 x 2); x1 is not, so it does not.
        assert_eq!(suggest_project(dir.path(), "Q3 review", "", "").as_deref(), Some("q3-planning"));
        assert_eq!(suggest_project(dir.path(), "x1 review", "", ""), None);
    }

    #[test]
    fn page_h1_words_count_as_extra_tokens_of_the_slug() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("Wiki/Projects");
        fs::create_dir_all(&projects).unwrap();
        fs::write(projects.join("acme-rollout.md"), "# Acme Rollout Bluebird\n").unwrap();
        fs::write(projects.join("other.md"), "# Other\n").unwrap();
        assert_eq!(suggest_project(dir.path(), "Bluebird kickoff", "", "").as_deref(), Some("acme-rollout"));
    }

    #[test]
    fn a_refile_replaces_only_the_two_code_sections() {
        let text = "# T\n\n## Summary\n\nx\n\n## Answered questions\n\n- **What storage?** NetApp. (On the call)\n\n## Q&A log\n\nQ: Where is the DR site?\nA: Charlotte.\n\n## Follow-up\n\n- call Jim\n- send diagram\n";
        let kept = card_row("c1", "Where is the DR site?", "Charlotte.");
        // The answered row was deleted: its section goes, Follow-up stays byte for byte.
        let out = refiled_text(text, std::slice::from_ref(&kept), &[]);
        assert_eq!(
            out,
            "# T\n\n## Summary\n\nx\n\n## Q&A log\n\nQ: Where is the DR site?\nA: Charlotte.\n\n## Follow-up\n\n- call Jim\n- send diagram\n"
        );
        // Nothing deleted: the whole file is unchanged.
        let rows = vec![answered_row("a1", "What storage?", "NetApp.", 5.0), kept];
        assert_eq!(refiled_text(text, &rows, &[]), text);
        // Both sections gone: only they go.
        let out = refiled_text(text, &[], &[]);
        assert_eq!(out, "# T\n\n## Summary\n\nx\n\n## Follow-up\n\n- call Jim\n- send diagram\n");
    }

    #[test]
    fn the_models_slug_is_cleaned_before_any_path_or_commit_message() {
        assert_eq!(sanitize_slug("../Evil/Slug\nx"), "evil-slug-x");
        assert_eq!(sanitize_slug("///"), "meeting");
        assert_eq!(sanitize_slug(""), "meeting");
        assert_eq!(sanitize_slug("-A  b_c-"), "a-b-c");
        let long = sanitize_slug(&format!("{}-tail", "a".repeat(59)));
        assert_eq!(long, "a".repeat(59));

        let dir = vault();
        let root = dir.path();
        let (mut draft, rows) = (pnc_draft(), pnc_rows());
        draft.slug = "../Evil/Slug\nx".into();
        let filed = save_note(input(root, &draft, &rows, None, false)).unwrap();
        assert_eq!(filed.note_path, root.join("_sources/meetings/2026-10-21-evil-slug-x.md"));
        assert!(filed.transcript_path.ends_with("_sources/meetings/2026-10-21-evil-slug-x_transcript.md"));
        assert!(filed.note_path.exists());
        assert_eq!(git(root, &["log", "-1", "--format=%s"]).trim(), "Meeting note: evil-slug-x");
        draft.slug = "///".into();
        let filed = save_note(input(root, &draft, &rows, None, true)).unwrap();
        assert_eq!(filed.note_path, root.join("_sources/meetings/2026-10-21-meeting.md"));
    }

    #[test]
    fn save_files_the_note_links_the_project_and_commits_only_those_files() {
        let dir = vault();
        let root = dir.path();
        fs::write(root.join("staged.txt"), "keep me staged").unwrap();
        git(root, &["add", "staged.txt"]);
        let (draft, rows) = (pnc_draft(), pnc_rows());
        let filed = save_note(input(root, &draft, &rows, Some("broadcom-pnc-bank"), false)).unwrap();

        assert_eq!(filed.project.as_deref(), Some("broadcom-pnc-bank"));
        assert!(filed.note_path.ends_with("_sources/meetings/2026-10-21-pnc-sync.md"));
        let note = fs::read_to_string(&filed.note_path).unwrap();
        assert!(note.starts_with(
            "# PNC sync with Jim\n\n- Date: 2026-10-21\n- Source: meetily-fork recording, saved from the app on 2026-10-21.\n- Full transcript: [[2026-10-21-pnc-sync_transcript]]\n- Linked projects: [[broadcom-pnc-bank]]\n"
        ), "{note}");
        assert!(note.contains("## Brief\n\nConfirm wave order\n\n## Summary"), "{note}");
        assert_eq!(note.matches("## Answered questions").count(), 1);
        assert!(note.contains("- **What storage?** NetApp. (On the call)"));
        assert_eq!(note.matches("## Q&A log").count(), 1);
        assert!(note.contains("Q: Where is the DR site?\nA: Charlotte."));
        assert!(!note.contains("Send the diagram"), "{note}");
        assert!(note.trim_end().ends_with("A: Charlotte."), "code sections come last: {note}");
        assert_eq!(fs::read_to_string(&filed.transcript_path).unwrap(), "You: hi");

        let page = fs::read_to_string(root.join("Wiki/Projects/broadcom-pnc-bank.md")).unwrap();
        assert!(page.contains("- 2026-10-20: intro call: [[2026-10-20-pnc-intro]]\n- 2026-10-21: PNC sync with Jim: [[2026-10-21-pnc-sync]]\n\n## Notes"), "{page}");

        let mut files: Vec<String> = git(root, &["log", "-1", "--name-only", "--format="]).lines().map(String::from).collect();
        files.sort();
        assert_eq!(
            files,
            vec![
                "Wiki/Projects/broadcom-pnc-bank.md",
                "_sources/meetings/2026-10-21-pnc-sync.md",
                "_sources/meetings/2026-10-21-pnc-sync_transcript.md"
            ]
        );
        assert_eq!(git(root, &["log", "-1", "--format=%s"]).trim(), "Meeting note: pnc-sync");
        assert_eq!(git(root, &["diff", "--cached", "--name-only"]).trim(), "staged.txt");
    }

    #[test]
    fn project_page_without_a_meetings_section_gets_one_at_the_end() {
        let page = "# MC\n\ntext\n";
        assert_eq!(
            with_meeting_link(page, "2026-10-21", "Sync", "2026-10-21-sync").unwrap(),
            "# MC\n\ntext\n\n## Meetings\n\n- 2026-10-21: Sync: [[2026-10-21-sync]]\n"
        );
        // An empty Meetings section keeps what follows it.
        assert_eq!(
            with_meeting_link("## Meetings\n\n## Notes\n", "d", "T", "s").unwrap(),
            "## Meetings\n\n- d: T: [[s]]\n\n## Notes\n"
        );
    }

    #[test]
    fn the_same_link_is_never_added_twice() {
        let once = with_meeting_link("## Meetings\n\n- a: b: [[x]]\n", "d", "T", "s").unwrap();
        assert_eq!(with_meeting_link(&once, "d", "T", "s"), None);
        assert_eq!(once.matches("[[s]]").count(), 1);
    }

    #[test]
    fn a_taken_name_gets_a_number_and_no_project_means_no_link() {
        let dir = vault();
        let root = dir.path();
        let (draft, rows) = (pnc_draft(), pnc_rows());
        save_note(input(root, &draft, &rows, None, false)).unwrap();
        let second = save_note(input(root, &draft, &rows, None, false)).unwrap();
        assert!(second.note_path.ends_with("2026-10-21-pnc-sync-2.md"));
        assert!(second.transcript_path.ends_with("2026-10-21-pnc-sync-2_transcript.md"));
        let note = fs::read_to_string(&second.note_path).unwrap();
        assert!(!note.contains("Linked projects"));
        assert!(note.contains("[[2026-10-21-pnc-sync-2_transcript]]"));
        let page = fs::read_to_string(root.join("Wiki/Projects/broadcom-pnc-bank.md")).unwrap();
        assert!(!page.contains("pnc-sync"), "the page is untouched without a project");
    }

    #[test]
    fn a_project_outside_the_list_is_refused_and_nothing_is_written() {
        let dir = vault();
        let root = dir.path();
        let head = git(root, &["rev-parse", "HEAD"]);
        let (draft, rows) = (pnc_draft(), pnc_rows());
        for bad in ["../secret", "nope", "README"] {
            let err = save_note(input(root, &draft, &rows, Some(bad), false)).unwrap_err();
            assert!(err.contains("not a project"), "{err}");
        }
        assert!(!root.join("_sources").exists());
        assert_eq!(git(root, &["rev-parse", "HEAD"]), head);
    }

    #[test]
    fn dry_run_writes_edits_and_commits_nothing() {
        let dir = vault();
        let root = dir.path();
        let head = git(root, &["rev-parse", "HEAD"]);
        let (draft, rows) = (pnc_draft(), pnc_rows());
        let filed = save_note(input(root, &draft, &rows, Some("broadcom-pnc-bank"), true)).unwrap();
        assert!(filed.note_path.ends_with("_sources/meetings/2026-10-21-pnc-sync.md"));
        assert!(!root.join("_sources").exists());
        assert!(git(root, &["status", "--porcelain"]).trim().is_empty());
        assert_eq!(git(root, &["rev-parse", "HEAD"]), head);
    }

    #[test]
    fn a_missing_title_falls_back_to_the_slug() {
        let dir = vault();
        let (draft, rows) = (pnc_draft(), pnc_rows());
        let mut i = input(dir.path(), &draft, &rows, None, false);
        i.title = None;
        i.brief = "";
        let note = fs::read_to_string(save_note(i).unwrap().note_path).unwrap();
        assert!(note.starts_with("# Pnc sync\n"), "{note}");
        assert!(!note.contains("## Brief"));
    }

    #[test]
    fn answered_section_is_oldest_answer_first_from_rows() {
        let rows = vec![
            answered_row("a", "Second?", "2.", 9.0),
            answered_row("b", "First?", "1.", 3.0),
            row("c", "question"),
        ];
        assert_eq!(
            code_sections(&rows),
            "## Answered questions\n\n- **First?** 1. (On the call)\n- **Second?** 2. (On the call)"
        );
        assert_eq!(code_sections(&[todo_row("t", "x")]), "");
    }

    #[test]
    fn refile_drops_a_deleted_answer_and_open_question_and_commits_only_the_note() {
        let dir = vault();
        let root = dir.path();
        let (draft, mut rows) = (pnc_draft(), pnc_rows());
        let filed = save_note(input(root, &draft, &rows, Some("broadcom-pnc-bank"), false)).unwrap();
        let mut open_q = row("q1", "question");
        open_q.question = "Is pricing in scope?".into();
        fs::write(root.join("other.txt"), "dirty").unwrap();

        // Delete the answered row: no storage line, one Q&A log, one commit on the note only.
        let deleted = rows.remove(0);
        assert!(refile(&filed.note_path, &rows, &[deleted], false).unwrap());
        let note = fs::read_to_string(&filed.note_path).unwrap();
        assert!(!note.contains("What storage?"), "{note}");
        assert!(!note.contains("## Answered questions"));
        assert_eq!(note.matches("## Q&A log").count(), 1);
        assert!(note.contains("- Is pricing in scope?"), "an answered row is not an open bullet");
        assert_eq!(git(root, &["log", "-1", "--name-only", "--format=%s"]).trim(), "Meeting note: 2026-10-21-pnc-sync, items deleted\n\n_sources/meetings/2026-10-21-pnc-sync.md");
        assert!(git(root, &["status", "--porcelain"]).contains("?? other.txt"));

        // Delete an open question the model listed: only that bullet goes.
        assert!(refile(&filed.note_path, &rows, &[open_q], false).unwrap());
        let note = fs::read_to_string(&filed.note_path).unwrap();
        assert!(!note.contains("Is pricing in scope?"));
        assert!(note.contains("- Who signs off?"));
        assert!(note.contains("## Brief\n\nConfirm wave order"));
    }

    #[test]
    fn deleting_the_last_open_question_removes_its_empty_heading() {
        let text = "# T\n\n## Summary\n\nx\n\n## Open questions\n\n- Only one?\n\n## Answered questions\n\n- **a** b (On the call)\n";
        let mut q = row("q", "for_you");
        q.question = "only one".into();
        let out = refiled_text(text, &[], &[q.question.clone()]);
        assert_eq!(out, "# T\n\n## Summary\n\nx\n");
    }

    #[test]
    fn refile_dry_run_and_a_missing_file_change_nothing() {
        let dir = vault();
        let root = dir.path();
        let (draft, mut rows) = (pnc_draft(), pnc_rows());
        let filed = save_note(input(root, &draft, &rows, None, false)).unwrap();
        let before = fs::read_to_string(&filed.note_path).unwrap();
        let head = git(root, &["rev-parse", "HEAD"]);
        let deleted = rows.remove(0);
        assert!(refile(&filed.note_path, &rows, &[deleted], true).unwrap());
        assert_eq!(fs::read_to_string(&filed.note_path).unwrap(), before);
        assert_eq!(git(root, &["rev-parse", "HEAD"]), head);
        assert!(!refile(&root.join("gone.md"), &rows, &[], false).unwrap());
    }
}
