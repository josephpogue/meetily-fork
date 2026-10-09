// assistant/ask_list.rs
//
// The Ask Next list: parse the ask lane's reply, keep item ids stable
// across refreshes so the panel does not flash, and drop anything Joseph
// dismissed. Pure functions, no I/O.

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

/// What one reply said, before ids are assigned.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawList {
    pub asks: Vec<(String, String)>,
    pub for_you: Vec<String>,
}

/// `ASK: <question> | WHY: <reason>` and `YOU: <question>` lines, or the
/// single word NONE. Anything else is ignored; a reply with no usable line
/// and no NONE is unparseable and returns None, so the caller keeps the
/// previous list instead of blanking it.
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
        } else if let Some(body) = line.strip_prefix("YOU:") {
            let question = body.trim().to_string();
            if !question.is_empty() && out.for_you.len() < MAX_FOR_YOU {
                out.for_you.push(question);
            }
        }
    }
    if out.asks.is_empty() && out.for_you.is_empty() && !saw_none {
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

/// Turns a fresh reply into the list the panel shows: dismissed questions
/// (and rewordings of them) are dropped, and each surviving item takes the
/// id of the previous item it matches so React keeps it in place.
pub fn build(prev: &AskList, raw: RawList, dismissed: &[String]) -> AskList {
    let keep = |q: &str| !dismissed.iter().any(|d| same_item(d, q));
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
        };
        let next = build(&prev, raw, &[]);
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
        };
        let next = build(&prev, raw, &[]);
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
        };
        let next = build(&AskList::default(), raw, &["How do you screen tenants?".to_string()]);
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
}
