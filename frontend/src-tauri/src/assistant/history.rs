// assistant/history.rs
//
// Collects what the assistant showed during a call (cards and Ask next
// questions) so the meeting save can keep it. The collector lives for a
// session: it resets when a new session opens, not when recording stops,
// so the save after stop still has the items.

use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::ask_lane::AsksOut;
use super::ask_list::{AnswerHow, AnsweredItem, AskItem};
use super::lanes::{CardKind, CardOut};
use super::todo_list::{TodoItem, TodoOrigin, TodoOwner};
use crate::database::repositories::assistant_items::{AssistantItem, AssistantItemsRepository};

pub type HistoryHandle = Arc<StdMutex<HistoryCollector>>;

pub fn new_handle() -> HistoryHandle {
    Arc::new(StdMutex::new(HistoryCollector::default()))
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// An item as collected, before call times are known.
#[derive(Debug, Clone)]
struct Entry {
    /// The id the saved row gets, made when the entry is created, so saving
    /// twice gives the same row ids and a later delete can find the row.
    saved_id: String,
    /// Ids this item has been seen under (an Ask next question can change id).
    ids: Vec<String>,
    is_card: bool,
    kind: String,
    shown_ms: i64,
    question: String,
    lead: String,
    bullets: Vec<String>,
    why: String,
    source: String,
    phase: String,
    changed_lines: Vec<String>,
    answer: String,
    answered_how: String,
    answered_ms: Option<i64>,
    /// To-do entries only: the call times arrive already on the recording's
    /// audio scale, so they are kept as given and never derived from `shown_ms`.
    todo_call_time: Option<f64>,
    todo_answered_call_time: Option<f64>,
    owner: String,
    owner_name: String,
    due: String,
}

impl Entry {
    fn blank(kind: &str, is_card: bool, id: &str, shown_ms: i64) -> Self {
        Entry {
            saved_id: Uuid::new_v4().to_string(),
            ids: vec![id.to_string()],
            is_card,
            kind: kind.to_string(),
            shown_ms,
            question: String::new(),
            lead: String::new(),
            bullets: Vec::new(),
            why: String::new(),
            source: String::new(),
            phase: String::new(),
            changed_lines: Vec::new(),
            answer: String::new(),
            answered_how: String::new(),
            answered_ms: None,
            todo_call_time: None,
            todo_answered_call_time: None,
            owner: String::new(),
            owner_name: String::new(),
            due: String::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct HistoryCollector {
    /// When the session opened, epoch ms: the fallback recording start.
    opened_ms: i64,
    /// First-seen order, which breaks call-time ties.
    entries: Vec<Entry>,
    /// Live ids Joseph deleted this session. No observe path brings one back.
    deleted: Vec<String>,
    /// The meeting these entries were last saved for, so a delete after the
    /// save can reach the saved rows.
    saved_meeting_id: Option<String>,
}

fn card_kind(kind: CardKind) -> &'static str {
    match kind {
        CardKind::Answer => "answer",
        CardKind::Ask => "ask",
        CardKind::Explain => "explain",
        CardKind::Catchup => "catchup",
    }
}

fn how_label(how: AnswerHow) -> &'static str {
    match how {
        AnswerHow::Call => "call",
        AnswerHow::Chat => "chat",
        AnswerHow::You => "you",
    }
}

/// Lowercase, whitespace collapsed, trailing `?.!` trimmed.
pub fn normalize_question(text: &str) -> String {
    let lower = text.to_lowercase();
    let collapsed = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_end_matches(|c| matches!(c, '?' | '.' | '!')).trim_end().to_string()
}

impl HistoryCollector {
    /// A new session opened: forget the last one.
    pub fn reset(&mut self, opened_ms: i64) {
        self.opened_ms = opened_ms;
        self.entries.clear();
        self.deleted.clear();
        self.saved_meeting_id = None;
    }

    /// Deletes every entry that was seen under one of `ids` and remembers all
    /// the ids it was seen under (and `ids` themselves), so a card still
    /// drafting, or a question or to-do re-sent under the same id, does not
    /// come back. Returns the removed entries' `saved_id`s.
    pub fn delete(&mut self, ids: &[String]) -> Vec<String> {
        let mut saved = Vec::new();
        let mut gone: Vec<String> = Vec::new();
        self.entries.retain(|e| {
            if e.ids.iter().any(|x| ids.contains(x)) {
                saved.push(e.saved_id.clone());
                gone.extend(e.ids.iter().cloned());
                false
            } else {
                true
            }
        });
        for id in gone.into_iter().chain(ids.iter().cloned()) {
            if !self.deleted.contains(&id) {
                self.deleted.push(id);
            }
        }
        saved
    }

    pub fn is_deleted(&self, id: &str) -> bool {
        self.deleted.iter().any(|d| d == id)
    }

    /// Every live id deleted this session.
    pub fn deleted_ids(&self) -> Vec<String> {
        self.deleted.clone()
    }

    /// The meeting the collected items were saved for, if they were.
    pub fn saved_meeting_id(&self) -> Option<String> {
        self.saved_meeting_id.clone()
    }

    /// Deletes the entries that were saved under these row ids, with every
    /// live id they were seen under. For a row deleted from a past meeting's
    /// Assistant tab while its collector is still live.
    pub fn delete_saved(&mut self, saved_ids: &[String]) {
        let ids: Vec<String> = self
            .entries
            .iter()
            .filter(|e| saved_ids.contains(&e.saved_id))
            .flat_map(|e| e.ids.iter().cloned())
            .collect();
        self.delete(&ids);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// One card emit. The first emit for an id sets when it was shown; every
    /// emit replaces the content, so the last state wins.
    pub fn observe_card(&mut self, card: &CardOut) {
        if self.is_deleted(&card.id) {
            return;
        }
        let idx = match self.entries.iter().position(|e| e.is_card && e.ids[0] == card.id) {
            Some(i) => i,
            None => {
                self.entries.push(Entry::blank(card_kind(card.kind), true, &card.id, card.ts));
                self.entries.len() - 1
            }
        };
        let e = &mut self.entries[idx];
        e.kind = card_kind(card.kind).to_string();
        e.question = card.question.clone();
        e.lead = card.lead.clone();
        e.bullets = card.bullets.clone();
        e.source = card.source.clone();
        e.phase = card.phase.clone();
        e.changed_lines = card.changed_lines.clone();
    }

    /// One Ask next snapshot: every open item, then everything answered.
    pub fn observe_asks(&mut self, out: &AsksOut, now_ms: i64) {
        for item in &out.asks {
            self.observe_open(item, "question", now_ms);
        }
        for item in &out.for_you {
            self.observe_open(item, "for_you", now_ms);
        }
        for done in &out.answered {
            self.observe_answered(done);
        }
        self.observe_todos(&out.todos, &out.dismissed_todos);
    }

    /// The to-dos of one emit: a dismissed id is dropped for good (it was a
    /// wrong pick-up), every other id is upserted with the latest state. An
    /// id missing from `todos` is left alone, so an empty emit removes nothing.
    /// An id not seen before still joins the entry with the same owner and
    /// normalized text: a to-do re-said after the assistant was switched off
    /// and on comes from a new lane with a new id, and updates the earlier
    /// entry (keeping when it was first said) instead of adding a second.
    fn observe_todos(&mut self, todos: &[TodoItem], dismissed: &[String]) {
        self.entries.retain(|e| !(e.kind == "todo" && e.ids.iter().any(|x| dismissed.contains(x))));
        let deleted = self.deleted.clone();
        for t in todos.iter().filter(|t| !dismissed.contains(&t.id) && !deleted.contains(&t.id)) {
            let owner = match t.owner {
                TodoOwner::You => "you",
                TodoOwner::Them => "them",
            };
            let by_id = self.entries.iter().position(|e| e.kind == "todo" && e.ids.contains(&t.id));
            let norm = normalize_question(&t.text);
            let by_text = || {
                self.entries
                    .iter()
                    .position(|e| e.kind == "todo" && e.owner == owner && !norm.is_empty() && normalize_question(&e.lead) == norm)
            };
            let (idx, fresh) = match by_id {
                Some(i) => (i, false),
                None => match by_text() {
                    Some(i) => {
                        self.entries[i].ids.push(t.id.clone());
                        (i, false)
                    }
                    None => {
                        self.entries.push(Entry::blank("todo", false, &t.id, t.at));
                        (self.entries.len() - 1, true)
                    }
                },
            };
            let merged = !fresh && !self.entries[idx].ids.is_empty() && self.entries[idx].ids[0] != t.id;
            let e = &mut self.entries[idx];
            if !merged {
                e.shown_ms = t.at;
                e.todo_call_time = Some(t.call_time);
            }
            e.lead = t.text.clone();
            e.source = match t.origin {
                TodoOrigin::Call => "call",
                TodoOrigin::Typed => "typed",
            }
            .to_string();
            e.owner = owner.to_string();
            e.owner_name = t.who.clone();
            e.due = t.due.clone();
            match &t.done {
                Some(d) => {
                    e.answer = d.how.clone();
                    e.answered_how = "call".to_string();
                    e.todo_answered_call_time = Some(d.call_time);
                }
                // A merged later lane never un-does an earlier done state.
                None if !merged => {
                    e.answer.clear();
                    e.answered_how.clear();
                    e.todo_answered_call_time = None;
                }
                None => {}
            }
        }
    }

    /// The entry for an id seen before, else one in the same `kind` with the
    /// same wording: a gap question and a For-you question never merge.
    fn find_question(&self, id: &str, question: &str, kind: &str) -> Option<usize> {
        if !id.is_empty() {
            if let Some(i) = self
                .entries
                .iter()
                .position(|e| !e.is_card && e.kind != "todo" && e.ids.iter().any(|x| x == id))
            {
                return Some(i);
            }
        }
        let norm = normalize_question(question);
        if norm.is_empty() {
            return None;
        }
        self.entries
            .iter()
            .position(|e| !e.is_card && e.kind == kind && normalize_question(&e.question) == norm)
    }

    fn observe_open(&mut self, item: &AskItem, kind: &str, now_ms: i64) {
        if self.is_deleted(&item.id) {
            return;
        }
        let idx = match self.find_question(&item.id, &item.question, kind) {
            Some(i) => i,
            None => {
                self.entries.push(Entry::blank(kind, false, &item.id, now_ms));
                self.entries.len() - 1
            }
        };
        let e = &mut self.entries[idx];
        if !e.ids.contains(&item.id) {
            e.ids.push(item.id.clone());
        }
        e.kind = kind.to_string();
        e.question = item.question.clone();
        e.why = item.why.clone();
    }

    fn observe_answered(&mut self, done: &AnsweredItem) {
        if self.is_deleted(&done.id) {
            return;
        }
        let kind = if matches!(done.how, AnswerHow::You) { "for_you" } else { "question" };
        let idx = match self.find_question(&done.id, &done.question, kind) {
            Some(i) => i,
            None => {
                let mut e = Entry::blank(kind, false, &done.id, done.at);
                e.question = done.question.clone();
                self.entries.push(e);
                self.entries.len() - 1
            }
        };
        let e = &mut self.entries[idx];
        if !e.ids.contains(&done.id) {
            e.ids.push(done.id.clone());
        }
        e.answer = done.answer.clone();
        e.answered_how = how_label(done.how).to_string();
        e.answered_ms = Some(done.at);
    }

    /// The items to save: call times against `start_ms`, in call-time order
    /// (ties keep first-seen order), cards with an empty lead dropped. Each
    /// item's id is its entry's `saved_id`, a uuid made when the entry was
    /// created: stable across saves and unique across meetings.
    pub fn finalize(&self, start_ms: i64) -> Vec<AssistantItem> {
        let secs = |ms: i64| ((ms - start_ms).max(0) as f64) / 1000.0;
        let mut kept: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| !(e.is_card && e.lead.trim().is_empty()))
            .collect();
        let call_time = |e: &Entry| e.todo_call_time.unwrap_or_else(|| secs(e.shown_ms));
        // Ties (for example items shown before the recording start, which
        // clamp to 0) fall back to when they were shown; stable after that,
        // so equal ones keep first-seen order.
        kept.sort_by(|a, b| call_time(a).total_cmp(&call_time(b)).then(a.shown_ms.cmp(&b.shown_ms)));
        kept.into_iter()
            .map(|e| AssistantItem {
                id: e.saved_id.clone(),
                kind: e.kind.clone(),
                call_time: call_time(e),
                shown_at: Utc
                    .timestamp_millis_opt(e.shown_ms)
                    .single()
                    .unwrap_or_else(Utc::now)
                    .to_rfc3339_opts(SecondsFormat::Millis, true),
                question: e.question.clone(),
                lead: e.lead.clone(),
                bullets: e.bullets.clone(),
                why: e.why.clone(),
                source: e.source.clone(),
                phase: e.phase.clone(),
                changed_lines: e.changed_lines.clone(),
                answer: e.answer.clone(),
                answered_how: e.answered_how.clone(),
                answered_call_time: e.todo_answered_call_time.or(e.answered_ms.map(secs)),
                owner: e.owner.clone(),
                owner_name: e.owner_name.clone(),
                due: e.due.clone(),
            })
            .collect()
    }

    pub fn opened_ms(&self) -> i64 {
        self.opened_ms
    }
}

// ---------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------

/// The recording start from `<folder>/metadata.json` (`created_at`), if
/// the file is there and readable.
pub fn recording_start_from_folder(folder: &Path) -> Option<DateTime<Utc>> {
    let raw = std::fs::read_to_string(folder.join("metadata.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let created = value.get("created_at")?.as_str()?;
    DateTime::parse_from_rfc3339(created).ok().map(|d| d.with_timezone(&Utc))
}

fn folder_file(folder: &Path) -> std::path::PathBuf {
    folder.join("assistant.json")
}

/// Writes `<folder>/assistant.json` atomically (temp file, then rename).
pub fn write_folder_file(
    folder: &Path,
    meeting_id: &str,
    recording_started_ms: i64,
    items: &[AssistantItem],
) -> std::io::Result<()> {
    let started = Utc
        .timestamp_millis_opt(recording_started_ms)
        .single()
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let doc = serde_json::json!({
        "version": 1,
        "meetingId": meeting_id,
        "recordingStartedAt": started,
        "items": items,
    });
    let text = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
    let tmp = folder.join("assistant.json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, folder_file(folder))
}

pub async fn folder_path_of(pool: &SqlitePool, meeting_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .filter(|p| !p.trim().is_empty())
}

/// Saves the collector's items for `meeting_id`: replaces the meeting's rows
/// and writes `assistant.json` into its folder. No items saves nothing. A
/// folder write failure is logged and the rows stay.
pub async fn save_for_meeting(
    pool: &SqlitePool,
    history: &HistoryHandle,
    meeting_id: &str,
) -> Result<u32, String> {
    let (collector_opened, snapshot) = {
        let c = history.lock().unwrap();
        (c.opened_ms(), c.clone_entries())
    };
    if snapshot.is_empty() {
        return Ok(0);
    }
    let folder = folder_path_of(pool, meeting_id).await.map(std::path::PathBuf::from);
    let start_ms = folder
        .as_deref()
        .and_then(recording_start_from_folder)
        .map(|d| d.timestamp_millis())
        .unwrap_or(collector_opened);
    let items = snapshot.finalize(start_ms);
    if items.is_empty() {
        return Ok(0);
    }
    let count = AssistantItemsRepository::replace_for_meeting(pool, meeting_id, &items)
        .await
        .map_err(|e| e.to_string())?;
    history.lock().unwrap().saved_meeting_id = Some(meeting_id.to_string());
    if let Some(folder) = folder.filter(|f| f.is_dir()) {
        if let Err(e) = write_folder_file(&folder, meeting_id, start_ms, &items) {
            log::warn!("assistant: could not write assistant.json for {}: {}", meeting_id, e);
        }
    }
    Ok(count)
}

/// Deletes the meeting's rows and its `assistant.json`; returns the rows deleted.
pub async fn delete_for_meeting(pool: &SqlitePool, meeting_id: &str) -> Result<u32, String> {
    let folder = folder_path_of(pool, meeting_id).await;
    let n = AssistantItemsRepository::delete_for_meeting(pool, meeting_id)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(folder) = folder {
        remove_folder_file(Path::new(&folder));
    }
    Ok(n)
}

/// The one step every delete goes through once its rows are gone from the
/// database (the vault update joins here later). `deleted` are the rows that
/// were removed; the meeting's remaining rows are what `assistant.json` keeps:
/// the file is rewritten from them with its old `recordingStartedAt`, and
/// removed when none remain. A meeting with no folder on disk has no file.
pub async fn after_items_deleted(
    pool: &SqlitePool,
    meeting_id: &str,
    deleted: &[AssistantItem],
) -> Result<(), String> {
    if deleted.is_empty() {
        return Ok(());
    }
    let remaining = AssistantItemsRepository::list_for_meeting(pool, meeting_id)
        .await
        .map_err(|e| e.to_string())?;
    let Some(folder) = folder_path_of(pool, meeting_id).await.map(std::path::PathBuf::from) else {
        return Ok(());
    };
    if !folder.is_dir() {
        return Ok(());
    }
    if remaining.is_empty() {
        remove_folder_file(&folder);
        return Ok(());
    }
    let start_ms = recording_start_ms_from_file(&folder)
        .or_else(|| recording_start_from_folder(&folder).map(|d| d.timestamp_millis()))
        .or_else(|| {
            // No file, no metadata: the first remaining item's show time less
            // its call time is the start it was saved against.
            let first = &remaining[0];
            DateTime::parse_from_rfc3339(&first.shown_at)
                .ok()
                .map(|d| d.timestamp_millis() - (first.call_time * 1000.0).round() as i64)
        })
        .unwrap_or_else(now_ms);
    if let Err(e) = write_folder_file(&folder, meeting_id, start_ms, &remaining) {
        log::warn!("assistant: could not rewrite assistant.json for {}: {}", meeting_id, e);
    }
    Ok(())
}

/// `recordingStartedAt` of an existing `<folder>/assistant.json`, in epoch ms.
fn recording_start_ms_from_file(folder: &Path) -> Option<i64> {
    let raw = std::fs::read_to_string(folder_file(folder)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let started = value.get("recordingStartedAt")?.as_str()?;
    DateTime::parse_from_rfc3339(started).ok().map(|d| d.timestamp_millis())
}

/// Deletes the saved rows of `meeting_id` with these ids, then runs the shared
/// after-delete step. Returns how many rows went; ids of other meetings or
/// unknown ids are ignored.
pub async fn delete_saved_items(
    pool: &SqlitePool,
    meeting_id: &str,
    ids: &[String],
) -> Result<u32, String> {
    let gone: Vec<AssistantItem> = AssistantItemsRepository::list_for_meeting(pool, meeting_id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|i| ids.contains(&i.id))
        .collect();
    let n = AssistantItemsRepository::delete_items(pool, meeting_id, ids)
        .await
        .map_err(|e| e.to_string())?;
    after_items_deleted(pool, meeting_id, &gone).await?;
    Ok(n)
}

/// Removes `<folder>/assistant.json` and nothing else in the folder. A
/// missing file is fine; a failure is logged.
pub fn remove_folder_file(folder: &Path) {
    let file = folder_file(folder);
    match std::fs::remove_file(&file) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("assistant: could not remove {}: {}", file.display(), e),
    }
}

impl HistoryCollector {
    fn clone_entries(&self) -> HistoryCollector {
        HistoryCollector {
            opened_ms: self.opened_ms,
            entries: self.entries.clone(),
            deleted: self.deleted.clone(),
            saved_meeting_id: self.saved_meeting_id.clone(),
        }
    }
}

/// Used by the card emitter and the ask hooks.
pub fn record_card(history: &HistoryHandle, card: &CardOut) {
    history.lock().unwrap().observe_card(card);
}

pub fn record_asks(history: &HistoryHandle, out: &AsksOut) {
    history.lock().unwrap().observe_asks(out, now_ms());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::assistant_items::tests::{add_meeting, test_pool};

    fn card(id: &str, phase: &str, lead: &str, ts: i64) -> CardOut {
        CardOut {
            id: id.into(),
            kind: CardKind::Answer,
            question: "Q?".into(),
            lead: lead.into(),
            bullets: vec!["b".into()],
            source: "s".into(),
            phase: phase.into(),
            changed_lines: vec![],
            ts,
        }
    }

    fn ask(id: &str, q: &str) -> AskItem {
        AskItem { id: id.into(), question: q.into(), why: "w".into() }
    }

    fn asks_out(asks: Vec<AskItem>, for_you: Vec<AskItem>, answered: Vec<AnsweredItem>) -> AsksOut {
        AsksOut {
            state: "idle".into(),
            asks,
            for_you,
            answered,
            updated_at: None,
            brief: String::new(),
            checked: true,
            checking: false,
            changed: vec![],
            todos: vec![],
            dismissed_todos: vec![],
        }
    }

    const START: &str = "2026-10-20T17:32:32.557Z";
    const SHOWN: &str = "2026-10-20T17:34:21.030Z";

    fn ms(s: &str) -> i64 {
        DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis()
    }

    #[test]
    fn call_time_matches_the_worked_example() {
        let mut c = HistoryCollector::default();
        c.reset(0);
        c.observe_card(&card("c1", "checked", "lead", ms(SHOWN)));
        let items = c.finalize(ms(START));
        assert_eq!(items[0].call_time, 108.473);
        assert_eq!(items[0].shown_at, SHOWN);
    }

    #[test]
    fn call_time_never_goes_below_zero() {
        let mut c = HistoryCollector::default();
        c.observe_card(&card("c1", "checked", "lead", 1_000));
        assert_eq!(c.finalize(5_000)[0].call_time, 0.0);
    }

    #[test]
    fn a_card_drafted_then_checked_saves_once_with_checked_content_and_first_time() {
        let mut c = HistoryCollector::default();
        c.observe_card(&card("c1", "drafting", "rough", 10_000));
        c.observe_card(&card("c1", "checked", "final", 30_000));
        let items = c.finalize(0);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].lead, "final");
        assert_eq!(items[0].phase, "checked");
        assert_eq!(items[0].call_time, 10.0);
    }

    #[test]
    fn a_card_with_an_empty_lead_is_not_saved() {
        let mut c = HistoryCollector::default();
        c.observe_card(&card("c1", "drafting", "  ", 10_000));
        assert!(c.finalize(0).is_empty());
    }

    #[test]
    fn the_same_question_under_two_ids_saves_once_with_latest_wording() {
        let mut c = HistoryCollector::default();
        c.observe_asks(&asks_out(vec![ask("a1", "What is the budget?")], vec![], vec![]), 10_000);
        c.observe_asks(&asks_out(vec![ask("a2", "  what is the   BUDGET ")], vec![], vec![]), 20_000);
        let items = c.finalize(0);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].call_time, 10.0);
        assert_eq!(items[0].question, "  what is the   BUDGET ");
        // And a snapshot under the older id still maps to it.
        c.observe_asks(&asks_out(vec![ask("a1", "Something else entirely?")], vec![], vec![]), 30_000);
        assert_eq!(c.finalize(0).len(), 1);
    }

    #[test]
    fn dismissed_questions_stay_and_kinds_map() {
        let mut c = HistoryCollector::default();
        c.observe_asks(&asks_out(vec![ask("a1", "One?")], vec![ask("f1", "Two?")], vec![]), 1_000);
        c.observe_asks(&asks_out(vec![], vec![], vec![]), 2_000);
        let items = c.finalize(0);
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].kind.as_str(), items[0].why.as_str()), ("question", "w"));
        assert_eq!(items[1].kind, "for_you");
    }

    #[test]
    fn an_answered_question_carries_its_answer() {
        let mut c = HistoryCollector::default();
        c.observe_asks(&asks_out(vec![ask("a1", "One?")], vec![], vec![]), 1_000);
        let done = AnsweredItem {
            id: "a1".into(),
            question: "One?".into(),
            answer: "Yes".into(),
            how: AnswerHow::Call,
            at: 61_500,
        };
        c.observe_asks(&asks_out(vec![], vec![], vec![done]), 62_000);
        let items = c.finalize(0);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].answer, "Yes");
        assert_eq!(items[0].answered_how, "call");
        assert_eq!(items[0].answered_call_time, Some(61.5));
        assert_eq!(items[0].call_time, 1.0);
    }

    #[test]
    fn an_answer_with_no_match_becomes_its_own_item_shown_at_the_answer_time() {
        let mut c = HistoryCollector::default();
        let done = AnsweredItem {
            id: "gone".into(),
            question: "Typed?".into(),
            answer: "Mine".into(),
            how: AnswerHow::You,
            at: 40_000,
        };
        c.observe_asks(&asks_out(vec![], vec![], vec![done.clone()]), 99_000);
        c.observe_asks(&asks_out(vec![], vec![], vec![done]), 99_500);
        let items = c.finalize(0);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "for_you");
        assert_eq!(items[0].call_time, 40.0);
        assert_eq!(items[0].answered_how, "you");
    }

    #[test]
    fn items_come_out_in_call_time_order_with_ties_in_first_seen_order() {
        let mut c = HistoryCollector::default();
        c.observe_card(&card("late", "checked", "L", 9_000));
        c.observe_card(&card("tie1", "checked", "T1", 5_000));
        c.observe_card(&card("tie2", "checked", "T2", 5_000));
        let leads: Vec<_> = c.finalize(0).into_iter().map(|i| i.lead).collect();
        assert_eq!(leads, vec!["T1", "T2", "L"]);
    }

    #[test]
    fn a_session_reopen_empties_the_collector() {
        let mut c = HistoryCollector::default();
        c.observe_card(&card("c1", "checked", "lead", 1_000));
        c.reset(7);
        assert!(c.is_empty());
        assert_eq!(c.opened_ms(), 7);
    }

    #[test]
    fn same_wording_in_a_different_kind_stays_two_items() {
        let mut c = HistoryCollector::default();
        c.reset(0);
        c.observe_asks(&asks_out(vec![ask("a1", "Is pricing in scope?")], vec![], vec![]), 1_000);
        // A For-you question with the same wording but a new id.
        c.observe_asks(
            &asks_out(vec![ask("a1", "Is pricing in scope?")], vec![ask("y1", "is pricing in scope")], vec![]),
            2_000,
        );
        let items = c.finalize(0);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, "question");
        assert_eq!(items[1].kind, "for_you");
        // Same kind, same wording, new id still merges.
        c.observe_asks(&asks_out(vec![ask("a2", "Is pricing in scope")], vec![], vec![]), 3_000);
        let items = c.finalize(0);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, "question");
        assert_eq!(items[1].kind, "for_you");
    }

    #[test]
    fn removing_the_folder_file_leaves_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        remove_folder_file(dir.path()); // missing file: fine
        std::fs::write(dir.path().join("assistant.json"), "{}").unwrap();
        std::fs::write(dir.path().join("metadata.json"), "{}").unwrap();
        std::fs::write(dir.path().join("audio.mp4"), "x").unwrap();
        remove_folder_file(dir.path());
        assert!(!dir.path().join("assistant.json").exists());
        assert!(dir.path().join("metadata.json").exists());
        assert!(dir.path().join("audio.mp4").exists());
    }

    fn seeded() -> HistoryHandle {
        let h = new_handle();
        h.lock().unwrap().reset(ms(START));
        record_card(&h, &card("c1", "checked", "lead", ms(SHOWN)));
        h
    }

    #[tokio::test]
    async fn save_writes_rows_and_the_folder_file_using_metadata_created_at() {
        let pool = test_pool().await;
        let dir = tempfile::tempdir().unwrap();
        // The collector opened a minute late; metadata.json has the true start.
        std::fs::write(
            dir.path().join("metadata.json"),
            format!("{{\"created_at\":\"{}\"}}", START),
        )
        .unwrap();
        add_meeting(&pool, "m1", Some(dir.path().to_str().unwrap())).await;
        let h = new_handle();
        h.lock().unwrap().reset(ms(START) + 60_000);
        record_card(&h, &card("c1", "checked", "lead", ms(SHOWN)));

        assert_eq!(save_for_meeting(&pool, &h, "m1").await.unwrap(), 1);
        let rows = AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap();
        assert_eq!(rows[0].call_time, 108.473);
        let file: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("assistant.json")).unwrap()).unwrap();
        assert_eq!(file["version"], 1);
        assert_eq!(file["meetingId"], "m1");
        assert_eq!(file["recordingStartedAt"], START);
        assert_eq!(file["items"][0]["callTime"], 108.473);
        assert_eq!(file["items"][0]["lead"], "lead");
        assert!(!dir.path().join("assistant.json.tmp").exists());

        // Saving again replaces, never duplicates.
        assert_eq!(save_for_meeting(&pool, &h, "m1").await.unwrap(), 1);
        assert_eq!(AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap().len(), 1);

        // Delete removes rows and the file.
        assert_eq!(delete_for_meeting(&pool, "m1").await.unwrap(), 1);
        assert!(AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap().is_empty());
        assert!(!dir.path().join("assistant.json").exists());
    }

    #[tokio::test]
    async fn without_metadata_the_session_open_time_is_the_start() {
        let pool = test_pool().await;
        let dir = tempfile::tempdir().unwrap();
        add_meeting(&pool, "m1", Some(dir.path().to_str().unwrap())).await;
        let h = seeded();
        save_for_meeting(&pool, &h, "m1").await.unwrap();
        let rows = AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap();
        assert_eq!(rows[0].call_time, 108.473);
    }

    #[tokio::test]
    async fn no_items_saves_nothing_and_writes_no_file() {
        let pool = test_pool().await;
        let dir = tempfile::tempdir().unwrap();
        add_meeting(&pool, "m1", Some(dir.path().to_str().unwrap())).await;
        let h = new_handle();
        assert_eq!(save_for_meeting(&pool, &h, "m1").await.unwrap(), 0);
        assert!(!dir.path().join("assistant.json").exists());
    }

    #[tokio::test]
    async fn a_missing_folder_still_keeps_the_rows() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", Some("/nonexistent/meetily-test-folder")).await;
        let h = seeded();
        assert_eq!(save_for_meeting(&pool, &h, "m1").await.unwrap(), 1);
        assert_eq!(AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap().len(), 1);
        // And no folder at all.
        add_meeting(&pool, "m2", None).await;
        // Row ids are unique across meetings, so a second meeting is a second collector.
        assert_eq!(save_for_meeting(&pool, &seeded(), "m2").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_failing_folder_write_logs_and_keeps_the_rows() {
        let pool = test_pool().await;
        let dir = tempfile::tempdir().unwrap();
        // A directory where the temp file goes makes the write fail.
        std::fs::create_dir(dir.path().join("assistant.json.tmp")).unwrap();
        add_meeting(&pool, "m1", Some(dir.path().to_str().unwrap())).await;
        let h = seeded();
        assert_eq!(save_for_meeting(&pool, &h, "m1").await.unwrap(), 1);
        assert_eq!(AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap().len(), 1);
    }

    fn todo(id: &str, owner: TodoOwner, text: &str, who: &str, due: &str, call_time: f64, at: i64) -> TodoItem {
        TodoItem {
            id: id.into(),
            text: text.into(),
            owner,
            who: who.into(),
            due: due.into(),
            origin: TodoOrigin::Call,
            call_time,
            at,
            done: None,
        }
    }

    fn with_todos(todos: Vec<TodoItem>, dismissed: Vec<&str>) -> AsksOut {
        let mut out = asks_out(vec![], vec![], vec![]);
        out.todos = todos;
        out.dismissed_todos = dismissed.into_iter().map(String::from).collect();
        out
    }

    fn collected_todos() -> HistoryCollector {
        let mut c = HistoryCollector::default();
        c.reset(0);
        let t1 = todo("t1", TodoOwner::You, "Send the design doc", "", "By Friday", 2405.0, 2_405_000);
        let t2 = todo("t2", TodoOwner::Them, "Send the export", "Marcus", "Monday", 2430.5, 2_430_500);
        c.observe_asks(&with_todos(vec![t2.clone(), t1.clone()], vec![]), 2_431_000);
        let mut t1_done = t1;
        t1_done.done = Some(super::super::todo_list::TodoDone {
            how: "said he sent it".into(),
            call_time: 2530.0,
            at: 2_530_000,
        });
        c.observe_asks(&with_todos(vec![t1_done], vec!["t2"]), 2_531_000);
        c
    }

    #[test]
    fn a_done_todo_saves_once_and_a_dismissed_one_never_does() {
        let items = collected_todos().finalize(1_000_000);
        assert_eq!(items.len(), 1);
        let t = &items[0];
        assert_eq!(t.kind, "todo");
        assert_eq!(t.lead, "Send the design doc");
        assert_eq!(t.call_time, 2405.0); // kept as given, not recomputed from `at`
        assert_eq!((t.owner.as_str(), t.owner_name.as_str(), t.due.as_str()), ("you", "", "By Friday"));
        assert_eq!(t.source, "call");
        assert_eq!(t.answer, "said he sent it");
        assert_eq!(t.answered_how, "call");
        assert_eq!(t.answered_call_time, Some(2530.0));
        assert!(t.question.is_empty() && t.why.is_empty() && t.phase.is_empty());
        assert!(t.bullets.is_empty() && t.changed_lines.is_empty());
    }

    #[test]
    fn an_empty_todos_emit_removes_nothing() {
        let mut c = collected_todos();
        c.observe_asks(&with_todos(vec![], vec![]), 3_000_000);
        assert_eq!(c.finalize(0).len(), 1);
    }

    #[test]
    fn an_open_todo_saves_with_no_answer_and_typed_origin() {
        let mut c = HistoryCollector::default();
        let mut t = todo("t1", TodoOwner::Them, "Send the export", "Marcus", "Monday", 2430.5, 2_430_500);
        t.origin = TodoOrigin::Typed;
        c.observe_asks(&with_todos(vec![t], vec![]), 0);
        let items = c.finalize(0);
        assert_eq!((items[0].source.as_str(), items[0].owner.as_str(), items[0].owner_name.as_str()), ("typed", "them", "Marcus"));
        assert_eq!((items[0].answer.as_str(), items[0].answered_how.as_str(), items[0].answered_call_time), ("", "", None));
    }

    #[test]
    fn a_todo_re_said_after_the_assistant_is_switched_off_and_on_saves_once() {
        let mut c = HistoryCollector::default();
        c.reset(0);
        let first = todo("t1", TodoOwner::You, "Send the design doc", "", "", 2405.0, 2_405_000);
        c.observe_asks(&with_todos(vec![first], vec![]), 2_406_000);
        // New lane, new id, same owner and words (case and spacing differ),
        // now done and with a due time.
        let mut again = todo("t9", TodoOwner::You, "  send the  Design doc. ", "", "By Friday", 3000.0, 3_000_000);
        again.done = Some(super::super::todo_list::TodoDone { how: "sent".into(), call_time: 3100.0, at: 3_100_000 });
        c.observe_asks(&with_todos(vec![again.clone()], vec![]), 3_101_000);
        let items = c.finalize(0);
        assert_eq!(items.len(), 1, "one entry, not two");
        assert_eq!((items[0].due.as_str(), items[0].answer.as_str()), ("By Friday", "sent"));
        assert_eq!(items[0].call_time, 2405.0, "keeps when it was first said");
        // The other owner with the same words is a different to-do.
        let them = todo("t10", TodoOwner::Them, "Send the design doc", "Dana", "", 3200.0, 3_200_000);
        c.observe_asks(&with_todos(vec![again, them], vec![]), 3_201_000);
        assert_eq!(c.finalize(0).len(), 2);
        // The new id is now an alias: dismissing it drops the entry it maps to.
        c.observe_asks(&with_todos(vec![], vec!["t9"]), 3_300_000);
        let left = c.finalize(0);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].owner, "them");
    }

    #[test]
    fn todos_do_not_leak_into_the_next_session() {
        let mut c = collected_todos();
        c.reset(5);
        assert!(c.is_empty());
    }

    #[tokio::test]
    async fn todo_fields_reach_the_rows_and_assistant_json() {
        let pool = test_pool().await;
        let dir = tempfile::tempdir().unwrap();
        add_meeting(&pool, "m1", Some(dir.path().to_str().unwrap())).await;
        let h = new_handle();
        h.lock().unwrap().reset(1_000_000);
        let t = todo("t2", TodoOwner::Them, "Send the export", "Marcus", "Monday", 2430.5, 2_430_500);
        record_asks(&h, &with_todos(vec![t], vec![]));
        assert_eq!(save_for_meeting(&pool, &h, "m1").await.unwrap(), 1);
        let rows = AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap();
        assert_eq!((rows[0].owner.as_str(), rows[0].owner_name.as_str(), rows[0].due.as_str()), ("them", "Marcus", "Monday"));
        let file: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("assistant.json")).unwrap()).unwrap();
        assert_eq!(file["items"][0]["owner"], "them");
        assert_eq!(file["items"][0]["ownerName"], "Marcus");
        assert_eq!(file["items"][0]["due"], "Monday");
    }

    fn open_two() -> HistoryCollector {
        let mut c = HistoryCollector::default();
        c.observe_asks(&asks_out(vec![ask("a1", "One?")], vec![ask("f1", "Two?")], vec![]), 1_000);
        c
    }

    #[test]
    fn a_deleted_question_is_gone_and_stays_gone_when_sent_again() {
        let mut c = open_two();
        let removed = c.delete(&["a1".to_string()]);
        assert_eq!(removed.len(), 1);
        let questions: Vec<_> = c.finalize(0).into_iter().map(|i| i.question).collect();
        assert_eq!(questions, vec!["Two?"]);
        c.observe_asks(&asks_out(vec![ask("a1", "One?")], vec![ask("f1", "Two?")], vec![]), 2_000);
        let questions: Vec<_> = c.finalize(0).into_iter().map(|i| i.question).collect();
        assert_eq!(questions, vec!["Two?"]);
    }

    #[test]
    fn a_deleted_answered_question_does_not_come_back_answered() {
        let mut c = open_two();
        c.delete(&["a1".to_string()]);
        let done = AnsweredItem {
            id: "a1".into(),
            question: "One?".into(),
            answer: "Yes".into(),
            how: AnswerHow::Call,
            at: 5_000,
        };
        c.observe_asks(&asks_out(vec![], vec![], vec![done]), 6_000);
        assert_eq!(c.finalize(0).len(), 1);
    }

    #[test]
    fn a_card_deleted_while_drafting_does_not_return_when_checked() {
        let mut c = HistoryCollector::default();
        c.observe_card(&card("c1", "drafting", "rough", 10_000));
        assert_eq!(c.delete(&["c1".to_string()]).len(), 1);
        c.observe_card(&card("c1", "checked", "final", 30_000));
        assert!(c.finalize(0).is_empty());
    }

    #[test]
    fn a_deleted_todo_does_not_return_when_emitted_again() {
        let mut c = HistoryCollector::default();
        let t = todo("t1", TodoOwner::You, "Send the design doc", "", "", 10.0, 10_000);
        c.observe_asks(&with_todos(vec![t.clone()], vec![]), 11_000);
        assert_eq!(c.finalize(0).len(), 1);
        c.delete(&["t1".to_string()]);
        assert!(c.finalize(0).is_empty());
        c.observe_asks(&with_todos(vec![t], vec![]), 12_000);
        assert!(c.finalize(0).is_empty());
    }

    #[test]
    fn delete_remembers_every_id_an_item_was_seen_under() {
        let mut c = HistoryCollector::default();
        c.observe_asks(&asks_out(vec![ask("a1", "What is the budget?")], vec![], vec![]), 1_000);
        c.observe_asks(&asks_out(vec![ask("a2", "what is the budget")], vec![], vec![]), 2_000);
        c.delete(&["a2".to_string()]);
        c.observe_asks(&asks_out(vec![ask("a1", "What is the budget?")], vec![], vec![]), 3_000);
        assert!(c.finalize(0).is_empty());
    }

    #[test]
    fn finalizing_twice_gives_the_same_ids_which_are_the_saved_ids() {
        let mut c = open_two();
        let first: Vec<_> = c.finalize(0).into_iter().map(|i| i.id).collect();
        let second: Vec<_> = c.finalize(0).into_iter().map(|i| i.id).collect();
        assert_eq!(first, second);
        let removed = c.delete(&["f1".to_string()]);
        assert_eq!(removed.len(), 1);
        assert!(first.contains(&removed[0]), "delete returns the saved id the row has");
        let other = open_two();
        assert_ne!(other.finalize(0)[0].id, first[0], "ids differ across collectors");
    }

    #[test]
    fn a_reset_forgets_deleted_ids_and_the_saved_meeting() {
        let mut c = open_two();
        c.delete(&["a1".to_string()]);
        c.saved_meeting_id = Some("m1".into());
        c.reset(5);
        assert!(c.deleted_ids().is_empty());
        assert_eq!(c.saved_meeting_id(), None);
        c.observe_asks(&asks_out(vec![ask("a1", "One?")], vec![], vec![]), 6_000);
        assert_eq!(c.finalize(0).len(), 1);
    }

    #[tokio::test]
    async fn saving_records_the_meeting_and_a_saved_row_delete_reaches_the_collector() {
        let pool = test_pool().await;
        let dir = tempfile::tempdir().unwrap();
        add_meeting(&pool, "m1", Some(dir.path().to_str().unwrap())).await;
        let h = new_handle();
        h.lock().unwrap().reset(0);
        record_asks(&h, &asks_out(vec![ask("a1", "One?")], vec![ask("f1", "Two?")], vec![]));
        assert_eq!(h.lock().unwrap().saved_meeting_id(), None);
        save_for_meeting(&pool, &h, "m1").await.unwrap();
        assert_eq!(h.lock().unwrap().saved_meeting_id(), Some("m1".to_string()));
        let rows = AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap();
        let one = rows.iter().find(|r| r.question == "One?").unwrap().id.clone();

        // Delete the saved row, and the live collector forgets it too.
        assert_eq!(delete_saved_items(&pool, "m1", &[one]).await.unwrap(), 1);
        h.lock().unwrap().delete_saved(&[rows.iter().find(|r| r.question == "One?").unwrap().id.clone()]);
        record_asks(&h, &asks_out(vec![ask("a1", "One?")], vec![ask("f1", "Two?")], vec![]));
        save_for_meeting(&pool, &h, "m1").await.unwrap();
        let after: Vec<_> =
            AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap().into_iter().map(|i| i.question).collect();
        assert_eq!(after, vec!["Two?"]);
    }

    fn row(id: &str, call_time: f64) -> AssistantItem {
        AssistantItem {
            id: id.into(),
            kind: "answer".into(),
            call_time,
            shown_at: "2026-10-20T17:34:21.030Z".into(),
            question: "Q?".into(),
            lead: format!("lead {id}"),
            bullets: vec![],
            why: String::new(),
            source: String::new(),
            phase: "checked".into(),
            changed_lines: vec![],
            answer: String::new(),
            answered_how: String::new(),
            answered_call_time: None,
            owner: String::new(),
            owner_name: String::new(),
            due: String::new(),
        }
    }

    #[tokio::test]
    async fn after_deleting_rows_assistant_json_lists_the_rest_and_goes_when_none_remain() {
        let pool = test_pool().await;
        let dir = tempfile::tempdir().unwrap();
        add_meeting(&pool, "m1", Some(dir.path().to_str().unwrap())).await;
        let items = vec![row("x", 1.0), row("y", 2.0)];
        AssistantItemsRepository::replace_for_meeting(&pool, "m1", &items).await.unwrap();
        // The file's start differs from metadata.json: the file's wins.
        std::fs::write(dir.path().join("metadata.json"), "{\"created_at\":\"2020-01-01T00:00:00.000Z\"}").unwrap();
        write_folder_file(dir.path(), "m1", ms(START), &items).unwrap();

        assert_eq!(delete_saved_items(&pool, "m1", &["x".to_string()]).await.unwrap(), 1);
        let file: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("assistant.json")).unwrap()).unwrap();
        assert_eq!(file["recordingStartedAt"], START);
        assert_eq!(file["items"].as_array().unwrap().len(), 1);
        assert_eq!(file["items"][0]["id"], "y");
        assert!(!dir.path().join("assistant.json.tmp").exists());

        assert_eq!(delete_saved_items(&pool, "m1", &["y".to_string()]).await.unwrap(), 1);
        assert!(!dir.path().join("assistant.json").exists());
        assert!(dir.path().join("metadata.json").exists());
    }

    #[tokio::test]
    async fn after_deleting_with_no_file_the_start_comes_from_metadata() {
        let pool = test_pool().await;
        let dir = tempfile::tempdir().unwrap();
        add_meeting(&pool, "m1", Some(dir.path().to_str().unwrap())).await;
        std::fs::write(dir.path().join("metadata.json"), format!("{{\"created_at\":\"{}\"}}", START)).unwrap();
        AssistantItemsRepository::replace_for_meeting(&pool, "m1", &[row("x", 1.0), row("y", 2.0)]).await.unwrap();
        delete_saved_items(&pool, "m1", &["x".to_string()]).await.unwrap();
        let file: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("assistant.json")).unwrap()).unwrap();
        assert_eq!(file["recordingStartedAt"], START);
        assert_eq!(file["items"][0]["id"], "y");
    }
}
