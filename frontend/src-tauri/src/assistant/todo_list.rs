// assistant/todo_list.rs
//
// The live To do list: what Joseph promised on the call (You) and what the
// other side owes him (Them). The ask lane's replies carry TODO lines next to
// its questions; this module holds the shapes the panel and the meeting's
// saved history share. Pure data and functions, no I/O.

use serde::Serialize;

use super::ask_list::{collapse, same_item};

/// Who has to do it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TodoOwner {
    /// Joseph promised it, or agreed when the other side asked him.
    You,
    /// Someone on the other side promised it to Joseph.
    Them,
}

/// Where a to-do came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TodoOrigin {
    /// Picked up from the transcript by the assistant.
    Call,
    /// Joseph typed it in the Ask box after `todo:`.
    Typed,
}

/// The call showed the to-do is finished.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoDone {
    /// A few words on how the call showed it, e.g. "said it was sent".
    pub how: String,
    /// Seconds from recording start (the transcript's audio_start_time scale).
    pub call_time: f64,
    /// Epoch milliseconds.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub id: String,
    /// The to-do in a few plain words, e.g. "Send the design doc".
    pub text: String,
    pub owner: TodoOwner,
    /// For a Them to-do, the person's name when the call says it; else "".
    pub who: String,
    /// When it is due, as the call said it ("By Friday", "Monday"); else "".
    pub due: String,
    pub origin: TodoOrigin,
    /// Seconds from recording start (the transcript's audio_start_time
    /// scale): when it was said, or typed.
    pub call_time: f64,
    /// Epoch milliseconds it was added.
    pub at: i64,
    /// Set once the call shows it is finished. Joseph never ticks it off.
    pub done: Option<TodoDone>,
}

// ---------------------------------------------------------------------
// Reply lines
// ---------------------------------------------------------------------

/// A to-do the model wrote, before it has an id or a time.
#[derive(Debug, Clone, PartialEq)]
pub struct RawTodo {
    pub text: String,
    pub owner: TodoOwner,
    pub who: String,
    pub due: String,
}

/// The TODO-family lines of one reply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TodoLines {
    /// `TODO:` lines.
    pub adds: Vec<RawTodo>,
    /// `TODO DONE:` lines: the to-do as listed, and how the call showed it.
    pub done: Vec<(String, String)>,
    /// `NOT TODO:` lines: the to-do as listed.
    pub retract: Vec<String>,
    /// A TODO-family line was read, usable or not. A reply of only unusable
    /// ones is still a reply: it changes nothing and is not a failed turn.
    pub seen: bool,
}

impl TodoLines {
    pub fn is_empty(&self) -> bool {
        self.adds.is_empty() && self.done.is_empty() && self.retract.is_empty()
    }
}

/// Strips `prefix` from the start of `line` without regard to ASCII case.
fn strip_key<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let head = line.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &line[prefix.len()..])
}

/// Reads one reply line. A TODO-family line that has what it needs is added
/// to `out`; any other line (including a TODO line with no text or no valid
/// owner) is ignored. Returns true when the line was TODO-family at all, so
/// the caller can leave it out of the question parser.
pub fn parse_line(line: &str, out: &mut TodoLines) -> bool {
    let line = line.trim();
    let line = line.strip_prefix("- ").unwrap_or(line).trim();
    // TODO DONE: must be tried before TODO:
    if strip_key(line, "TODO DONE:").is_some() || strip_key(line, "NOT TODO:").is_some() || strip_key(line, "TODO:").is_some() {
        out.seen = true;
    }
    if let Some(body) = strip_key(line, "TODO DONE:") {
        let (text, fields) = split_fields(body, &["how"]);
        if !text.is_empty() {
            let how = fields.into_iter().find(|(k, _)| k == "how").map(|(_, v)| v).unwrap_or_default();
            out.done.push((text, how));
        }
        true
    } else if let Some(body) = strip_key(line, "NOT TODO:") {
        let (text, _) = split_fields(body, &[]);
        if !text.is_empty() {
            out.retract.push(text);
        }
        true
    } else if let Some(body) = strip_key(line, "TODO:") {
        let (text, fields) = split_fields(body, &["owner", "who", "when"]);
        let field = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default();
        let owner = match field("owner").to_ascii_lowercase().as_str() {
            "you" => Some(TodoOwner::You),
            "them" => Some(TodoOwner::Them),
            _ => None,
        };
        if let (false, Some(owner)) = (text.is_empty(), owner) {
            out.adds.push(RawTodo { text, owner, who: field("who"), due: field("when") });
        }
        true
    } else {
        false
    }
}

/// `<text> | KEY: value | KEY: value`. A segment counts as a field only when
/// its key is one of `keys` (case-insensitive); any other segment stays part
/// of the text, so `|` may appear in a to-do. Values and text are collapsed
/// to one line. The first value of a key wins.
fn split_fields(body: &str, keys: &[&str]) -> (String, Vec<(String, String)>) {
    let mut text = String::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    for (i, seg) in body.split('|').enumerate() {
        let seg_trim = seg.trim();
        let field = keys.iter().find_map(|k| {
            let rest = strip_key(seg_trim, k)?.trim_start();
            rest.strip_prefix(':').map(|v| (k.to_string(), collapse(v)))
        });
        match field {
            Some((k, v)) if i > 0 => {
                if !fields.iter().any(|(fk, _)| *fk == k) {
                    fields.push((k, v));
                }
            }
            _ => {
                if i > 0 {
                    text.push_str(" | ");
                }
                text.push_str(seg_trim);
            }
        }
    }
    (collapse(&text), fields)
}

// ---------------------------------------------------------------------
// Applying a reply to the list
// ---------------------------------------------------------------------

/// Adds each picked-up to-do that is new. One that matches (same owner, same
/// words or a rewording) a to-do already on the list, open or done, is
/// skipped; so is one that matches a dismissed to-do's text, whoever owns it.
pub fn add_picked(todos: &mut Vec<TodoItem>, dismissed_texts: &[String], raw: Vec<RawTodo>, call_time: f64, now: i64) {
    for r in raw {
        let seen = todos.iter().any(|t| t.owner == r.owner && same_item(&t.text, &r.text))
            || dismissed_texts.iter().any(|d| same_item(d, &r.text));
        if seen {
            continue;
        }
        todos.push(TodoItem {
            id: uuid::Uuid::new_v4().to_string(),
            text: r.text,
            owner: r.owner,
            who: r.who,
            due: r.due,
            origin: TodoOrigin::Call,
            call_time,
            at: now,
            done: None,
        });
    }
}

/// The open to-do a reply's wording points at: an exact (case-insensitive)
/// match first, else the one open to-do that `same_item` accepts. Two or more
/// fuzzy matches and no exact one point at nothing, so a vague line never
/// changes the wrong to-do. With `call_only`, typed to-dos are not candidates
/// in either pass.
fn find_open(todos: &[TodoItem], text: &str, call_only: bool) -> Option<usize> {
    let open = |t: &TodoItem| t.done.is_none() && (!call_only || t.origin == TodoOrigin::Call);
    if let Some(i) = todos.iter().position(|t| open(t) && t.text.trim().eq_ignore_ascii_case(text.trim())) {
        return Some(i);
    }
    let mut fuzzy = todos.iter().enumerate().filter(|(_, t)| open(t) && same_item(&t.text, text)).map(|(i, _)| i);
    match (fuzzy.next(), fuzzy.next()) {
        (Some(i), None) => Some(i),
        _ => None,
    }
}

/// Marks each named open to-do done (either owner, typed or not). Matching
/// nothing, or a to-do that is already done, changes nothing.
pub fn apply_done(todos: &mut [TodoItem], done: Vec<(String, String)>, call_time: f64, now: i64) {
    for (text, how) in done {
        if let Some(i) = find_open(todos, &text, false) {
            todos[i].done = Some(TodoDone { how, call_time, at: now });
        }
    }
}

/// The deep pass withdrawing a to-do the quick pass added: removes each named
/// open, call-origin to-do, records its id in `dismissed_ids` (so the saved
/// history drops it too) and its text in `dismissed_texts` (so no sub-lane
/// adds it back). Typed and done to-dos are never touched.
pub fn retract(todos: &mut Vec<TodoItem>, dismissed_ids: &mut Vec<String>, dismissed_texts: &mut Vec<String>, texts: Vec<String>) {
    for text in texts {
        if let Some(i) = find_open(todos, &text, true) {
            let gone = todos.remove(i);
            dismissed_ids.push(gone.id);
            dismissed_texts.push(gone.text);
        }
    }
}

/// The shown list: newest first by `at`, a later-added one first on a tie.
/// `todos` is in the order they were added.
pub fn newest_first(todos: &[TodoItem]) -> Vec<TodoItem> {
    let mut out: Vec<TodoItem> = todos.iter().rev().cloned().collect();
    out.sort_by(|a, b| b.at.cmp(&a.at));
    out
}

/// CURRENT TO-DOS lines for the model prompt, one per to-do, oldest first.
pub fn render(todos: &[TodoItem]) -> String {
    todos
        .iter()
        .map(|t| {
            let mut tags = match (t.owner, t.who.is_empty()) {
                (TodoOwner::You, _) => "you".to_string(),
                (TodoOwner::Them, true) => "them".to_string(),
                (TodoOwner::Them, false) => format!("them: {}", t.who),
            };
            if t.origin == TodoOrigin::Typed {
                tags.push_str(", typed");
            }
            if t.done.is_some() {
                tags.push_str(", done");
            }
            let due = if t.due.is_empty() { String::new() } else { format!(" | WHEN: {}", t.due) };
            format!("- [{}] {}{}", tags, t.text, due)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(raw: &str) -> TodoLines {
        let mut out = TodoLines::default();
        for line in raw.lines() {
            parse_line(line, &mut out);
        }
        out
    }

    fn item(text: &str, owner: TodoOwner, origin: TodoOrigin) -> TodoItem {
        TodoItem {
            id: format!("id-{}", text),
            text: text.to_string(),
            owner,
            who: String::new(),
            due: String::new(),
            origin,
            call_time: 1.0,
            at: 100,
            done: None,
        }
    }

    fn raw(text: &str, owner: TodoOwner) -> RawTodo {
        RawTodo { text: text.to_string(), owner, who: String::new(), due: String::new() }
    }

    #[test]
    fn a_you_line_with_a_due_time_parses() {
        let l = lines("TODO: Send the design doc | OWNER: you | WHO: | WHEN: By Friday");
        assert_eq!(
            l.adds,
            vec![RawTodo {
                text: "Send the design doc".into(),
                owner: TodoOwner::You,
                who: String::new(),
                due: "By Friday".into()
            }]
        );
    }

    #[test]
    fn a_them_line_keeps_the_name_and_due_time() {
        let l = lines("TODO: Send the host inventory export | OWNER: them | WHO: Marcus | WHEN: Monday");
        assert_eq!(l.adds.len(), 1);
        assert_eq!(l.adds[0].owner, TodoOwner::Them);
        assert_eq!(l.adds[0].who, "Marcus");
        assert_eq!(l.adds[0].due, "Monday");
    }

    #[test]
    fn who_and_when_are_optional() {
        let l = lines("TODO: Check with legal | OWNER: you");
        assert_eq!(l.adds.len(), 1);
        assert_eq!((l.adds[0].who.as_str(), l.adds[0].due.as_str()), ("", ""));
    }

    #[test]
    fn keys_are_case_insensitive_and_spaces_collapse() {
        let l = lines("  - todo:  Send   the\tdoc | owner: YOU | when:  by   Friday ");
        assert_eq!(l.adds.len(), 1);
        assert_eq!(l.adds[0].text, "Send the doc");
        assert_eq!(l.adds[0].owner, TodoOwner::You);
        assert_eq!(l.adds[0].due, "by Friday");
    }

    #[test]
    fn lines_without_a_text_or_a_valid_owner_are_ignored() {
        assert!(lines("TODO: x | OWNER: boss").adds.is_empty());
        assert!(lines("TODO:  | OWNER: you").adds.is_empty());
        assert!(lines("TODO: Send it").adds.is_empty());
    }

    #[test]
    fn done_and_retract_lines_parse() {
        let l = lines("TODO DONE: Send the design doc | HOW: said he sent it\nNOT TODO: Send the design doc");
        assert_eq!(l.done, vec![("Send the design doc".to_string(), "said he sent it".to_string())]);
        assert_eq!(l.retract, vec!["Send the design doc".to_string()]);
        assert!(l.adds.is_empty());
        assert!(!l.is_empty());
    }

    #[test]
    fn a_done_line_without_how_still_parses() {
        let l = lines("TODO DONE: Send the design doc");
        assert_eq!(l.done, vec![("Send the design doc".to_string(), String::new())]);
    }

    #[test]
    fn a_new_to_do_is_added_with_the_turn_time() {
        let mut todos = Vec::new();
        add_picked(&mut todos, &[], vec![raw("Send the design doc", TodoOwner::You)], 2430.5, 7);
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].origin, TodoOrigin::Call);
        assert_eq!((todos[0].call_time, todos[0].at), (2430.5, 7));
        assert!(todos[0].done.is_none());
        assert!(!todos[0].id.is_empty());
    }

    #[test]
    fn a_reworded_repeat_is_not_added_but_the_other_owner_is() {
        let mut todos = vec![item("Send the design doc with the domain layout", TodoOwner::You, TodoOrigin::Call)];
        add_picked(&mut todos, &[], vec![raw("Send the design doc with domain layout", TodoOwner::You)], 1.0, 1);
        assert_eq!(todos.len(), 1);
        add_picked(&mut todos, &[], vec![raw("Send the design doc with domain layout", TodoOwner::Them)], 1.0, 1);
        assert_eq!(todos.len(), 2);
    }

    #[test]
    fn a_done_to_do_still_blocks_a_repeat() {
        let mut done = item("Check the licence count", TodoOwner::You, TodoOrigin::Call);
        done.done = Some(TodoDone { how: "x".into(), call_time: 1.0, at: 1 });
        let mut todos = vec![done];
        add_picked(&mut todos, &[], vec![raw("Check the licence count", TodoOwner::You)], 1.0, 1);
        assert_eq!(todos.len(), 1);
    }

    #[test]
    fn a_dismissed_text_is_never_added_again() {
        let mut todos = Vec::new();
        let dismissed = vec!["Send the design doc".to_string()];
        add_picked(&mut todos, &dismissed, vec![raw("Send the design doc", TodoOwner::Them)], 1.0, 1);
        assert!(todos.is_empty());
    }

    #[test]
    fn not_todo_never_picks_a_typed_to_do_and_needs_a_single_fuzzy_match() {
        let call_you = item("Send the design document with the domain layout", TodoOwner::You, TodoOrigin::Call);
        let call_them = item("Send the design document review notes", TodoOwner::Them, TodoOrigin::Call);
        let mut typed = item("Send the design document", TodoOwner::You, TodoOrigin::Typed);
        typed.id = "typed".into();
        let words = |t: &[TodoItem]| t.iter().map(|x| x.text.clone()).collect::<Vec<_>>();

        // Two call to-dos match fuzzily and none exactly: nothing changes.
        let mut todos = vec![call_you.clone(), call_them.clone()];
        retract(&mut todos, &mut vec![], &mut vec![], vec!["Send the design document".into()]);
        assert_eq!(todos.len(), 2);

        // Only one open: it goes.
        let mut todos = vec![call_you.clone()];
        retract(&mut todos, &mut vec![], &mut vec![], vec!["Send the design document".into()]);
        assert!(todos.is_empty());

        // A typed exact match is untouched; the single fuzzy call one goes.
        let mut todos = vec![typed.clone(), call_you.clone()];
        retract(&mut todos, &mut vec![], &mut vec![], vec!["Send the design document".into()]);
        assert_eq!(words(&todos), vec!["Send the design document".to_string()]);
    }

    #[test]
    fn todo_done_changes_nothing_when_two_open_to_dos_match_only_fuzzily() {
        let mut todos = vec![
            item("Send the design document with the domain layout", TodoOwner::You, TodoOrigin::Call),
            item("Send the design document review notes", TodoOwner::Them, TodoOrigin::Call),
        ];
        apply_done(&mut todos, vec![("Send the design document".into(), "sent".into())], 1.0, 1);
        assert!(todos.iter().all(|t| t.done.is_none()));
        // An exact match still wins over the fuzzy ones.
        apply_done(&mut todos, vec![("send the design doc review notes".into(), "sent".into())], 1.0, 1);
        assert!(todos[0].done.is_none() && todos[1].done.is_some());
        // One fuzzy match among the open ones is enough.
        let mut one = vec![item("Send the design document with the domain layout", TodoOwner::You, TodoOrigin::Call)];
        apply_done(&mut one, vec![("Send the design document".into(), "sent".into())], 1.0, 1);
        assert!(one[0].done.is_some());
    }

    #[test]
    fn done_marks_an_open_to_do_of_either_owner() {
        let mut todos = vec![item("Send the design doc", TodoOwner::Them, TodoOrigin::Call)];
        apply_done(&mut todos, vec![("Send the design doc".into(), "said he sent it".into())], 2430.5, 9);
        assert_eq!(todos[0].done, Some(TodoDone { how: "said he sent it".into(), call_time: 2430.5, at: 9 }));
    }

    #[test]
    fn done_marks_a_typed_to_do() {
        let mut todos = vec![item("Send Dana the runbook template", TodoOwner::You, TodoOrigin::Typed)];
        apply_done(&mut todos, vec![("Send Dana the runbook template".into(), "sent".into())], 5.0, 9);
        assert!(todos[0].done.is_some());
    }

    #[test]
    fn done_matching_nothing_or_an_already_done_one_changes_nothing() {
        let mut todos = vec![item("Send the design doc", TodoOwner::You, TodoOrigin::Call)];
        apply_done(&mut todos, vec![("Book the venue".into(), "x".into())], 5.0, 9);
        assert!(todos[0].done.is_none());
        apply_done(&mut todos, vec![("Send the design doc".into(), "first".into())], 5.0, 9);
        apply_done(&mut todos, vec![("Send the design doc".into(), "second".into())], 6.0, 10);
        assert_eq!(todos[0].done.as_ref().unwrap().how, "first");
        assert_eq!(todos[0].done.as_ref().unwrap().at, 9);
    }

    #[test]
    fn retract_removes_an_open_call_to_do_and_remembers_the_id() {
        let mut todos = vec![item("Send the design doc", TodoOwner::You, TodoOrigin::Call)];
        let (mut ids, mut texts) = (Vec::new(), Vec::new());
        retract(&mut todos, &mut ids, &mut texts, vec!["Send the design doc".into()]);
        assert!(todos.is_empty());
        assert_eq!(ids, vec!["id-Send the design doc".to_string()]);
        assert_eq!(texts, vec!["Send the design doc".to_string()]);
    }

    #[test]
    fn retract_ignores_typed_and_done_to_dos() {
        let mut done = item("Check the licence count", TodoOwner::You, TodoOrigin::Call);
        done.done = Some(TodoDone { how: "x".into(), call_time: 1.0, at: 1 });
        let mut todos = vec![done, item("Send Dana the runbook template", TodoOwner::You, TodoOrigin::Typed)];
        let (mut ids, mut texts) = (Vec::new(), Vec::new());
        retract(
            &mut todos,
            &mut ids,
            &mut texts,
            vec!["Check the licence count".into(), "Send Dana the runbook template".into()],
        );
        assert_eq!(todos.len(), 2);
        assert!(ids.is_empty() && texts.is_empty());
    }

    #[test]
    fn the_list_is_newest_first_with_later_added_first_on_ties() {
        let mut a = item("a", TodoOwner::You, TodoOrigin::Call);
        let mut b = item("b", TodoOwner::You, TodoOrigin::Call);
        let mut c = item("c", TodoOwner::You, TodoOrigin::Call);
        a.at = 10;
        b.at = 20;
        c.at = 20;
        let shown = newest_first(&[a, b, c]);
        let order: Vec<&str> = shown.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(order, vec!["c", "b", "a"]);
    }

    #[test]
    fn render_marks_owner_name_typed_done_and_due() {
        let mut a = item("Send the design doc", TodoOwner::You, TodoOrigin::Call);
        a.due = "By Friday".into();
        let mut b = item("Send the host inventory export", TodoOwner::Them, TodoOrigin::Call);
        b.who = "Marcus".into();
        b.due = "Monday".into();
        let c = item("Send Dana the runbook template", TodoOwner::You, TodoOrigin::Typed);
        let mut d = item("Check the licence count", TodoOwner::You, TodoOrigin::Call);
        d.done = Some(TodoDone { how: "x".into(), call_time: 1.0, at: 1 });
        let e = item("Get the quote", TodoOwner::Them, TodoOrigin::Call);
        assert_eq!(
            render(&[a, b, c, d, e]),
            "- [you] Send the design doc | WHEN: By Friday\n\
             - [them: Marcus] Send the host inventory export | WHEN: Monday\n\
             - [you, typed] Send Dana the runbook template\n\
             - [you, done] Check the licence count\n\
             - [them] Get the quote"
        );
    }
}
