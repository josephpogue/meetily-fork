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
            format!("- **{}** {} ({})", a.question, a.answer, how)
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

/// "team-standup" -> "Team standup".
fn title_from_slug(slug: &str) -> String {
    let spaced = slug.replace('-', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}

/// Writes `<vault_root>/_sources/meetings/<date>-<slug>.md` (title, date,
/// the note body, and a Q&A log section built from `qa_log`) and a sibling
/// `..._transcript.md` holding the raw transcript. `dry_run` writes nothing
/// and returns the paths that would have been written. A non-dry-run save
/// stages and commits both files in the vault's git repo; a git failure
/// comes back as `Err`, never a panic, so a Joseph-side git problem never
/// takes the app down with it.
pub fn save_note(
    vault_root: &Path,
    date: &str,
    draft: &NoteDraft,
    transcript: &str,
    qa_log: &str,
    dry_run: bool,
) -> Result<Vec<PathBuf>, String> {
    let meetings_dir = vault_root.join("_sources").join("meetings");
    let note_path = meetings_dir.join(format!("{}-{}.md", date, draft.slug));
    let transcript_path = meetings_dir.join(format!("{}-{}_transcript.md", date, draft.slug));

    if dry_run {
        return Ok(vec![note_path, transcript_path]);
    }

    fs::create_dir_all(&meetings_dir)
        .map_err(|e| format!("could not create {}: {}", meetings_dir.display(), e))?;

    let title = title_from_slug(&draft.slug);
    let note_content = format!(
        "# {}\n\n{}\n\n{}\n\n## Q&A log\n\n{}\n",
        title, date, draft.markdown, qa_log
    );
    fs::write(&note_path, note_content)
        .map_err(|e| format!("could not write {}: {}", note_path.display(), e))?;
    fs::write(&transcript_path, transcript)
        .map_err(|e| format!("could not write {}: {}", transcript_path.display(), e))?;

    let paths = vec![note_path.clone(), transcript_path.clone()];

    run_git(
        vault_root,
        &["add", &note_path.to_string_lossy(), &transcript_path.to_string_lossy()],
    )?;
    run_git(
        vault_root,
        &["commit", "-m", &format!("Meeting note: {}", draft.slug)],
    )?;

    Ok(paths)
}

fn run_git(vault_root: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(vault_root)
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
    fn save_note_writes_note_and_transcript_creating_dirs() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());

        let draft = NoteDraft {
            slug: "team-standup".to_string(),
            markdown: "## Summary\n\nWe shipped Proposal 3.\n".to_string(),
        };
        let paths = save_note(
            dir.path(),
            "2026-09-01",
            &draft,
            "You: hi\nThem: hi",
            "Q: which proposal?\nA: Proposal 3",
            false,
        )
        .unwrap();

        assert_eq!(paths.len(), 2);
        let note_content = fs::read_to_string(&paths[0]).unwrap();
        assert!(note_content.starts_with("# Team standup"));
        assert!(note_content.contains("2026-09-01"));
        assert!(note_content.contains("We shipped Proposal 3."));
        assert!(note_content.contains("## Q&A log"));
        assert!(note_content.contains("Q: which proposal?"));

        let transcript_content = fs::read_to_string(&paths[1]).unwrap();
        assert_eq!(transcript_content, "You: hi\nThem: hi");

        assert!(paths[0].ends_with("_sources/meetings/2026-09-01-team-standup.md"));
        assert!(paths[1].ends_with("_sources/meetings/2026-09-01-team-standup_transcript.md"));
    }

    #[test]
    fn save_note_dry_run_writes_nothing_and_returns_would_be_paths() {
        let dir = tempfile::tempdir().unwrap();
        let draft = NoteDraft {
            slug: "team-standup".to_string(),
            markdown: "## Summary\n\nWe shipped Proposal 3.\n".to_string(),
        };
        let paths = save_note(dir.path(), "2026-09-01", &draft, "transcript", "qa log", true).unwrap();

        assert_eq!(paths.len(), 2);
        for p in &paths {
            assert!(!p.exists(), "dry run must not write {}", p.display());
        }
        assert!(paths[0].ends_with("_sources/meetings/2026-09-01-team-standup.md"));
    }
}
