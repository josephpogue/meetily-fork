// assistant/ask_list.rs
//
// The Ask Next list: parse the ask lane's reply, keep item ids stable
// across refreshes so the panel does not flash, drop anything Joseph
// dismissed, and move a question the call answered onto the answered list
// (kept with its answer, never deleted). Pure functions, no I/O.

use std::collections::HashSet;

use serde::Serialize;

use super::card::content_words;

pub const MAX_ASKS: usize = 5;
pub const MAX_FOR_YOU: usize = 2;

/// Two questions count as the same item at or above this share of content
/// words, measured against the longer of the two.
const SAME_ITEM_OVERLAP: f64 = 0.6;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskItem {
    pub id: String,
    pub question: String,
    /// Empty for a For-you item.
    pub why: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskList {
    pub asks: Vec<AskItem>,
    pub for_you: Vec<AskItem>,
}

/// How a question got answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AnswerHow {
    /// Said on the call.
    Call,
    /// Worked out by Joseph and the assistant in Ask.
    Chat,
    /// Joseph typed the answer to a For-you question.
    You,
}

/// A question that left the open list because it was answered.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnsweredItem {
    /// The open item's id when it was on the list.
    pub id: String,
    pub question: String,
    pub answer: String,
    pub how: AnswerHow,
    /// When it was answered, epoch milliseconds.
    pub at: i64,
}

/// What one reply said, before ids are assigned.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawList {
    pub asks: Vec<(String, String)>,
    pub for_you: Vec<String>,
    /// `DONE:` lines: question, answer, and how it was answered.
    pub done: Vec<(String, String, AnswerHow)>,
}

/// `ASK: <question> | WHY: <reason>`, `YOU: <question>` and
/// `DONE: <question> | ANSWER: <answer> | BY: call` lines, or the single word
/// NONE. Anything else is ignored; a reply with no usable line and no NONE is
/// unparseable and returns None, so the caller keeps the previous list
/// instead of blanking it. A reply of DONE lines alone is a usable reply.
pub fn parse(raw: &str) -> Option<RawList> {
    let mut out = RawList::default();
    let mut saw_none = false;
    for line in raw.lines() {
        let line = line.trim();
        let line = line.strip_prefix("- ").unwrap_or(line).trim();
        if line == "NONE" {
            saw_none = true;
        } else if let Some(body) = line.strip_prefix("ASK:") {
            let (question, why) = split_why(body);
            if !question.is_empty() && out.asks.len() < MAX_ASKS {
                out.asks.push((question, why));
            }
        } else if let Some(body) = line.strip_prefix("DONE:") {
            // Not capped: every answered question is kept.
            if let Some(done) = split_done(body) {
                out.done.push(done);
            }
        } else if let Some(body) = line.strip_prefix("YOU:") {
            // A YOU line carries no reason, but the model sometimes adds one.
            // Only the question is kept: it is shown in the panel and
            // appended to the brief when Joseph answers it.
            let (question, _why) = split_why(body);
            if !question.is_empty() && out.for_you.len() < MAX_FOR_YOU {
                out.for_you.push(question);
            }
        }
    }
    if out.asks.is_empty() && out.for_you.is_empty() && out.done.is_empty() && !saw_none {
        return None;
    }
    Some(out)
}

fn split_why(body: &str) -> (String, String) {
    match body.split_once('|') {
        Some((question, rest)) => {
            let rest = rest.trim();
            let why = rest.strip_prefix("WHY:").unwrap_or(rest).trim();
            (question.trim().to_string(), why.to_string())
        }
        None => (body.trim().to_string(), String::new()),
    }
}

/// `<question> | ANSWER: <answer> | BY: call|chat`. None when the question or
/// the answer is empty. A missing or unknown BY means the call.
fn split_done(body: &str) -> Option<(String, String, AnswerHow)> {
    let (question, rest) = body.split_once('|')?;
    let rest = rest.trim().strip_prefix("ANSWER:")?;
    let (answer, by) = match rest.rfind("| BY:") {
        Some(pos) => (&rest[..pos], rest[pos + "| BY:".len()..].trim()),
        None => (rest, ""),
    };
    let how = if by.eq_ignore_ascii_case("chat") { AnswerHow::Chat } else { AnswerHow::Call };
    let (question, answer) = (question.trim(), answer.trim());
    if question.is_empty() || answer.is_empty() {
        return None;
    }
    Some((question.to_string(), answer.to_string(), how))
}

pub fn same_item(a: &str, b: &str) -> bool {
    if a.trim().eq_ignore_ascii_case(b.trim()) {
        return true;
    }
    let wa = content_words(a);
    let wb = content_words(b);
    let longer = wa.len().max(wb.len());
    if longer == 0 {
        return false;
    }
    let shared = wa.intersection(&wb).count();
    shared as f64 / longer as f64 >= SAME_ITEM_OVERLAP
}

/// Moves each DONE question of a reply onto `answered` (oldest first). A DONE
/// that matches an open item takes that item's id and wording; one that
/// matches nothing is a question Joseph asked on the call himself and gets a
/// new id. A question that is already answered keeps its first answer.
pub fn absorb_done(prev: &AskList, answered: &mut Vec<AnsweredItem>, done: Vec<(String, String, AnswerHow)>, now: i64) {
    for (question, answer, how) in done {
        if answered.iter().any(|a| same_item(&a.question, &question)) {
            continue;
        }
        let open = prev
            .asks
            .iter()
            .chain(prev.for_you.iter())
            .find(|i| same_item(&i.question, &question));
        let (id, question) = match open {
            Some(item) => (item.id.clone(), item.question.clone()),
            None => (uuid::Uuid::new_v4().to_string(), question),
        };
        answered.push(AnsweredItem { id, question, answer, how, at: now });
    }
}

/// The list without any question that is answered.
pub fn without_answered(list: &AskList, answered: &[AnsweredItem]) -> AskList {
    let open = |i: &&AskItem| !answered.iter().any(|a| same_item(&a.question, &i.question));
    AskList {
        asks: list.asks.iter().filter(open).cloned().collect(),
        for_you: list.for_you.iter().filter(open).cloned().collect(),
    }
}

/// Turns a fresh reply into the list the panel shows. DONE lines move onto
/// `answered` first; then dismissed and answered questions (and rewordings of
/// them) are dropped, and each surviving item takes the id of the previous
/// item it matches so React keeps it in place.
pub fn build(
    prev: &AskList,
    answered: &mut Vec<AnsweredItem>,
    mut raw: RawList,
    dismissed: &[String],
    now: i64,
) -> AskList {
    absorb_done(prev, answered, std::mem::take(&mut raw.done), now);
    let keep = |q: &str| {
        !dismissed.iter().any(|d| same_item(d, q)) && !answered.iter().any(|a| same_item(&a.question, q))
    };
    let mut used: HashSet<String> = HashSet::new();
    let asks = raw
        .asks
        .into_iter()
        .filter(|(q, _)| keep(q))
        .map(|(question, why)| AskItem {
            id: carry_id(&prev.asks, &question, &mut used),
            question,
            why,
        })
        .collect();
    let for_you = raw
        .for_you
        .into_iter()
        .filter(|q| keep(q))
        .map(|question| AskItem {
            id: carry_id(&prev.for_you, &question, &mut used),
            question,
            why: String::new(),
        })
        .collect();
    AskList { asks, for_you }
}

fn carry_id(prev: &[AskItem], question: &str, used: &mut HashSet<String>) -> String {
    let id = prev
        .iter()
        .filter(|p| !used.contains(&p.id))
        .find(|p| same_item(&p.question, question))
        .map(|p| p.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    used.insert(id.clone());
    id
}

/// The list in reply format, for the next turn's CURRENT LIST section.
pub fn render(list: &AskList) -> String {
    if list.asks.is_empty() && list.for_you.is_empty() {
        return "(empty)".to_string();
    }
    let mut lines = Vec::new();
    for a in &list.asks {
        if a.why.is_empty() {
            lines.push(format!("ASK: {}", a.question));
        } else {
            lines.push(format!("ASK: {} | WHY: {}", a.question, a.why));
        }
    }
    for y in &list.for_you {
        lines.push(format!("YOU: {}", y.question));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, q: &str) -> AskItem {
        AskItem { id: id.to_string(), question: q.to_string(), why: String::new() }
    }

    #[test]
    fn parse_reads_ask_and_you_lines() {
        let raw = "ASK: What are your management fees? | WHY: the brief asks about cost\nYOU: Is pricing in scope for this call?";
        let parsed = parse(raw).expect("valid list");
        assert_eq!(
            parsed.asks,
            vec![("What are your management fees?".to_string(), "the brief asks about cost".to_string())]
        );
        assert_eq!(parsed.for_you, vec!["Is pricing in scope for this call?".to_string()]);
    }

    #[test]
    fn parse_accepts_bullets_and_a_missing_why() {
        let parsed = parse("- ASK: How do you screen tenants?\n- YOU: Do you need a decision today?").unwrap();
        assert_eq!(parsed.asks, vec![("How do you screen tenants?".to_string(), String::new())]);
        assert_eq!(parsed.for_you.len(), 1);
    }

    #[test]
    fn parse_drops_a_why_from_a_you_line() {
        let raw = "YOU: Do you want to find out if managing or selling makes more sense, or have you already leaned one way? | WHY: decides which questions to prioritize\nYOU: Is pricing in scope for this call?";
        let parsed = parse(raw).expect("valid list");
        assert_eq!(
            parsed.for_you,
            vec![
                "Do you want to find out if managing or selling makes more sense, or have you already leaned one way?".to_string(),
                "Is pricing in scope for this call?".to_string(),
            ]
        );
    }

    #[test]
    fn parse_none_is_an_empty_list() {
        assert_eq!(parse("NONE"), Some(RawList::default()));
    }

    #[test]
    fn parse_rejects_prose() {
        assert_eq!(parse("I think you should ask about their fees."), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn parse_caps_asks_at_five_and_you_at_two() {
        let mut raw = String::new();
        for i in 0..7 {
            raw.push_str(&format!("ASK: question number {} about topic{} | WHY: w\n", i, i));
        }
        for i in 0..4 {
            raw.push_str(&format!("YOU: self question {} about thing{}\n", i, i));
        }
        let parsed = parse(&raw).unwrap();
        assert_eq!(parsed.asks.len(), MAX_ASKS);
        assert_eq!(parsed.for_you.len(), MAX_FOR_YOU);
    }

    #[test]
    fn same_item_matches_rewording_but_not_a_new_topic() {
        assert!(same_item("What are your management fees?", "What are your monthly management fees?"));
        assert!(same_item("How do you screen tenants?", "how do you screen tenants?"));
        assert!(!same_item("What are your management fees?", "How do you screen tenants?"));
    }

    #[test]
    fn build_carries_ids_over_reworded_items_and_mints_new_ones() {
        let prev = AskList {
            asks: vec![item("a1", "What are your management fees?")],
            for_you: vec![item("y1", "Is pricing in scope?")],
        };
        let raw = RawList {
            asks: vec![
                ("What are your monthly management fees?".to_string(), "cost".to_string()),
                ("How do you screen tenants?".to_string(), "risk".to_string()),
            ],
            for_you: vec!["Is pricing in scope?".to_string()],
            done: vec![],
        };
        let next = build(&prev, &mut Vec::new(), raw, &[], 0);
        assert_eq!(next.asks[0].id, "a1");
        assert_eq!(next.asks[0].question, "What are your monthly management fees?");
        assert_ne!(next.asks[1].id, "a1");
        assert!(!next.asks[1].id.is_empty());
        assert_eq!(next.for_you[0].id, "y1");
        assert_eq!(next.for_you[0].why, "");
    }

    #[test]
    fn build_never_gives_two_items_the_same_id() {
        let prev = AskList { asks: vec![item("a1", "What are your management fees?")], for_you: vec![] };
        let raw = RawList {
            asks: vec![
                ("What are your management fees?".to_string(), String::new()),
                ("What are your monthly management fees?".to_string(), String::new()),
            ],
            for_you: vec![],
            done: vec![],
        };
        let next = build(&prev, &mut Vec::new(), raw, &[], 0);
        assert_eq!(next.asks[0].id, "a1");
        assert_ne!(next.asks[1].id, "a1");
    }

    #[test]
    fn build_drops_reworded_dismissed_question() {
        let raw = RawList {
            asks: vec![
                ("How do you screen new tenants?".to_string(), String::new()),
                ("What are your management fees?".to_string(), String::new()),
            ],
            for_you: vec![],
            done: vec![],
        };
        let next = build(&AskList::default(), &mut Vec::new(), raw, &["How do you screen tenants?".to_string()], 0);
        assert_eq!(next.asks.len(), 1);
        assert_eq!(next.asks[0].question, "What are your management fees?");
    }

    #[test]
    fn render_round_trips_through_parse() {
        let list = AskList {
            asks: vec![AskItem { id: "a".into(), question: "What are your fees?".into(), why: "cost".into() }],
            for_you: vec![item("y", "Is pricing in scope?")],
        };
        let parsed = parse(&render(&list)).unwrap();
        assert_eq!(parsed.asks, vec![("What are your fees?".to_string(), "cost".to_string())]);
        assert_eq!(parsed.for_you, vec!["Is pricing in scope?".to_string()]);
        assert_eq!(render(&AskList::default()), "(empty)");
    }

    fn raw_asks(qs: &[&str]) -> RawList {
        RawList { asks: qs.iter().map(|q| (q.to_string(), String::new())).collect(), ..Default::default() }
    }

    fn done(q: &str, a: &str, how: AnswerHow) -> (String, String, AnswerHow) {
        (q.to_string(), a.to_string(), how)
    }

    fn answered_item(id: &str, q: &str, a: &str) -> AnsweredItem {
        AnsweredItem { id: id.into(), question: q.into(), answer: a.into(), how: AnswerHow::Call, at: 1 }
    }

    #[test]
    fn parse_reads_a_done_line_next_to_an_ask() {
        let raw = "ASK: Who owns backups? | WHY: owner unknown\nDONE: What storage do you run today? | ANSWER: NetApp, about 40 TB. | BY: call";
        let parsed = parse(raw).expect("valid list");
        assert_eq!(parsed.asks, vec![("Who owns backups?".to_string(), "owner unknown".to_string())]);
        assert_eq!(parsed.done, vec![done("What storage do you run today?", "NetApp, about 40 TB.", AnswerHow::Call)]);
    }

    #[test]
    fn parse_done_by_chat_and_a_missing_or_unknown_by_is_call() {
        let parsed = parse(
            "DONE: One? | ANSWER: a1 | BY: chat\nDONE: Two? | ANSWER: a2\nDONE: Three? | ANSWER: a3 | BY: carrier pigeon\n- DONE: Four? | ANSWER: a | b | BY: chat",
        )
        .unwrap();
        assert_eq!(
            parsed.done,
            vec![
                done("One?", "a1", AnswerHow::Chat),
                done("Two?", "a2", AnswerHow::Call),
                done("Three?", "a3", AnswerHow::Call),
                done("Four?", "a | b", AnswerHow::Chat),
            ]
        );
    }

    #[test]
    fn parse_ignores_a_done_without_an_answer_and_does_not_cap_done_lines() {
        let parsed = parse("DONE: No answer? | ANSWER:  | BY: call\nDONE: Bare question?\nDONE: Missing marker? | BY: call\nNONE").unwrap();
        assert!(parsed.done.is_empty());
        let mut raw = String::new();
        for i in 0..9 {
            raw.push_str(&format!("DONE: question {} about topic{}? | ANSWER: answer {} | BY: call\n", i, i, i));
        }
        assert_eq!(parse(&raw).unwrap().done.len(), 9);
    }

    #[test]
    fn parse_a_reply_of_only_done_lines_is_not_unparseable() {
        let parsed = parse("DONE: Who owns backups? | ANSWER: The ops team. | BY: call").expect("parses");
        assert!(parsed.asks.is_empty() && parsed.for_you.is_empty());
        assert_eq!(parsed.done.len(), 1);
        // A DONE line that is ignored leaves nothing usable.
        assert_eq!(parse("DONE: Who owns backups? | ANSWER:"), None);
    }

    #[test]
    fn build_moves_a_done_open_item_to_answered_with_its_id_and_wording() {
        let prev = AskList {
            asks: vec![item("a1", "What storage do you run today?"), item("a2", "Who owns backups?")],
            for_you: vec![],
        };
        let mut raw = raw_asks(&["Who owns backups?", "What storage do you run today in production?"]);
        raw.done = vec![done("What storage platform do you run today?", "NetApp, about 40 TB.", AnswerHow::Call)];
        let mut answered = Vec::new();
        let next = build(&prev, &mut answered, raw, &[], 77);
        assert_eq!(answered.len(), 1);
        assert_eq!(answered[0].id, "a1");
        assert_eq!(answered[0].question, "What storage do you run today?", "the words Joseph saw");
        assert_eq!(answered[0].answer, "NetApp, about 40 TB.");
        assert_eq!(answered[0].how, AnswerHow::Call);
        assert_eq!(answered[0].at, 77);
        assert_eq!(next.asks.len(), 1, "the DONE item leaves even when the reply lists it as ASK too");
        assert_eq!(next.asks[0].id, "a2");
    }

    #[test]
    fn build_a_done_that_matches_nothing_is_a_new_answered_item() {
        let mut answered = Vec::new();
        let mut raw = RawList::default();
        raw.done = vec![done("Where is the data center?", "In Atlanta.", AnswerHow::Call)];
        let next = build(&AskList::default(), &mut answered, raw, &[], 5);
        assert!(next.asks.is_empty());
        assert_eq!(answered.len(), 1);
        assert!(!answered[0].id.is_empty());
        assert_eq!(answered[0].question, "Where is the data center?");
    }

    #[test]
    fn build_keeps_the_first_answer_and_drops_asks_that_match_an_answered_question() {
        let mut answered = vec![answered_item("x", "What storage do you run today?", "NetApp.")];
        let mut raw = raw_asks(&["What storage do you run today in production?", "Who owns backups?"]);
        raw.for_you = vec!["What storage do you run today?".to_string()];
        raw.done = vec![done("What storage do you run today?", "Pure.", AnswerHow::Chat)];
        let next = build(&AskList::default(), &mut answered, raw, &[], 9);
        assert_eq!(answered.len(), 1);
        assert_eq!(answered[0].answer, "NetApp.");
        assert_eq!(answered[0].how, AnswerHow::Call);
        assert_eq!(next.asks.len(), 1);
        assert_eq!(next.asks[0].question, "Who owns backups?");
        assert!(next.for_you.is_empty());
    }

    #[test]
    fn without_answered_filters_both_lists() {
        let list = AskList {
            asks: vec![item("a1", "What storage do you run today?"), item("a2", "Who owns backups?")],
            for_you: vec![item("y1", "Is pricing in scope?")],
        };
        let answered = vec![
            answered_item("a1", "What storage do you run today?", "NetApp."),
            answered_item("y1", "Is pricing in scope?", "Yes."),
        ];
        let left = without_answered(&list, &answered);
        assert_eq!(left.asks.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["a2"]);
        assert!(left.for_you.is_empty());
    }

    #[test]
    fn answered_serializes_camel_case_with_a_lowercase_how() {
        let json = serde_json::to_value(answered_item("a", "Q?", "A.")).unwrap();
        assert_eq!(json, serde_json::json!({"id":"a","question":"Q?","answer":"A.","how":"call","at":1}));
        assert_eq!(serde_json::to_value(AnswerHow::Chat).unwrap(), serde_json::json!("chat"));
        assert_eq!(serde_json::to_value(AnswerHow::You).unwrap(), serde_json::json!("you"));
    }
}
