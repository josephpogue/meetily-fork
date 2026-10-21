// The live assistant's output for a saved meeting: one row per card or Ask
// next question the assistant showed during the call.

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection, SqlitePool};

/// One saved item, in the camelCase shape the commands return and
/// `assistant.json` holds. `id` is unique across all meetings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantItem {
    pub id: String,
    /// "answer" | "ask" | "explain" | "catchup" | "question" | "for_you"
    pub kind: String,
    /// Seconds from recording start, the scale of transcripts.audio_start_time.
    pub call_time: f64,
    /// RFC3339 UTC, when the item first appeared.
    pub shown_at: String,
    pub question: String,
    pub lead: String,
    pub bullets: Vec<String>,
    pub why: String,
    pub source: String,
    pub phase: String,
    pub changed_lines: Vec<String>,
    pub answer: String,
    /// "call" | "chat" | "you" | ""
    pub answered_how: String,
    pub answered_call_time: Option<f64>,
    /// To-do items only ("you" | "them"); "" on every other kind.
    pub owner: String,
    /// For a Them to-do, the person's name when the call said it.
    pub owner_name: String,
    /// When a to-do is due, as the call said it.
    pub due: String,
}

pub struct AssistantItemsRepository;

impl AssistantItemsRepository {
    /// Replaces the meeting's rows with `items` (already in position order),
    /// in one transaction.
    pub async fn replace_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
        items: &[AssistantItem],
    ) -> Result<u32, sqlx::Error> {
        let mut tx = pool.begin().await?;
        Self::delete_with_connection(&mut tx, meeting_id).await?;
        for (position, item) in items.iter().enumerate() {
            let bullets = serde_json::to_string(&item.bullets).unwrap_or_else(|_| "[]".into());
            let changed = serde_json::to_string(&item.changed_lines).unwrap_or_else(|_| "[]".into());
            sqlx::query(
                "INSERT INTO meeting_assistant_items (
                    id, meeting_id, position, kind, call_time, shown_at, question, lead,
                    bullets_json, why, source, phase, changed_lines_json, answer,
                    answered_how, answered_call_time, owner, owner_name, due
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&item.id)
            .bind(meeting_id)
            .bind(position as i64)
            .bind(&item.kind)
            .bind(item.call_time)
            .bind(&item.shown_at)
            .bind(&item.question)
            .bind(&item.lead)
            .bind(bullets)
            .bind(&item.why)
            .bind(&item.source)
            .bind(&item.phase)
            .bind(changed)
            .bind(&item.answer)
            .bind(&item.answered_how)
            .bind(item.answered_call_time)
            .bind(&item.owner)
            .bind(&item.owner_name)
            .bind(&item.due)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(items.len() as u32)
    }

    /// The meeting's items in position order; empty when none.
    pub async fn list_for_meeting(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<AssistantItem>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, kind, call_time, shown_at, question, lead, bullets_json, why, source,
                    phase, changed_lines_json, answer, answered_how, answered_call_time,
                    owner, owner_name, due
             FROM meeting_assistant_items WHERE meeting_id = ? ORDER BY position ASC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let bullets: String = row.try_get("bullets_json")?;
            let changed: String = row.try_get("changed_lines_json")?;
            out.push(AssistantItem {
                id: row.try_get("id")?,
                kind: row.try_get("kind")?,
                call_time: row.try_get("call_time")?,
                shown_at: row.try_get("shown_at")?,
                question: row.try_get("question")?,
                lead: row.try_get("lead")?,
                bullets: serde_json::from_str(&bullets).unwrap_or_default(),
                why: row.try_get("why")?,
                source: row.try_get("source")?,
                phase: row.try_get("phase")?,
                changed_lines: serde_json::from_str(&changed).unwrap_or_default(),
                answer: row.try_get("answer")?,
                answered_how: row.try_get("answered_how")?,
                answered_call_time: row.try_get("answered_call_time")?,
                owner: row.try_get("owner")?,
                owner_name: row.try_get("owner_name")?,
                due: row.try_get("due")?,
            });
        }
        Ok(out)
    }

    /// Deletes the meeting's rows and returns how many went.
    pub async fn delete_for_meeting(pool: &SqlitePool, meeting_id: &str) -> Result<u32, sqlx::Error> {
        let mut conn = pool.acquire().await?;
        Self::delete_with_connection(&mut conn, meeting_id).await
    }

    /// Same, on an open connection or transaction (meeting deletion uses it).
    pub async fn delete_with_connection(
        conn: &mut SqliteConnection,
        meeting_id: &str,
    ) -> Result<u32, sqlx::Error> {
        let result = sqlx::query("DELETE FROM meeting_assistant_items WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *conn)
            .await?;
        Ok(result.rows_affected() as u32)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// An in-memory database with a minimal `meetings` table and the real
    /// migration applied.
    pub(crate) async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE meetings (id TEXT PRIMARY KEY, title TEXT, folder_path TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        for migration in [
            include_str!("../../../migrations/20261020120000_meeting_assistant_items.sql"),
            include_str!("../../../migrations/20261021120000_meeting_assistant_todo_fields.sql"),
        ] {
            for stmt in migration.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                sqlx::query(stmt).execute(&pool).await.unwrap();
            }
        }
        pool
    }

    pub(crate) async fn add_meeting(pool: &SqlitePool, id: &str, folder: Option<&str>) {
        sqlx::query("INSERT INTO meetings (id, title, folder_path) VALUES (?, 't', ?)")
            .bind(id)
            .bind(folder)
            .execute(pool)
            .await
            .unwrap();
    }

    fn item(id: &str, call_time: f64) -> AssistantItem {
        AssistantItem {
            id: id.into(),
            kind: "answer".into(),
            call_time,
            shown_at: "2026-10-20T17:34:21.030Z".into(),
            question: "What is the rate?".into(),
            lead: "Four percent.".into(),
            bullets: vec!["one".into(), "two".into()],
            why: "because".into(),
            source: "notes.md".into(),
            phase: "checked".into(),
            changed_lines: vec!["lead".into()],
            answer: "yes".into(),
            answered_how: "chat".into(),
            answered_call_time: Some(130.5),
            owner: "them".into(),
            owner_name: "Marcus".into(),
            due: "Monday".into(),
        }
    }

    #[tokio::test]
    async fn save_then_list_round_trips_every_field_in_position_order() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", None).await;
        let mut plain = item("b", 20.0);
        plain.answered_call_time = None;
        let items = vec![item("a", 10.0), plain];
        assert_eq!(AssistantItemsRepository::replace_for_meeting(&pool, "m1", &items).await.unwrap(), 2);
        let back = AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap();
        assert_eq!(back, items);
    }

    #[tokio::test]
    async fn a_row_from_the_old_schema_reads_back_with_empty_todo_fields() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", None).await;
        sqlx::query(
            "INSERT INTO meeting_assistant_items (id, meeting_id, position, kind, call_time, shown_at)
             VALUES ('old', 'm1', 0, 'answer', 1.0, '2026-10-20T17:34:21.030Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let back = AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap();
        assert_eq!((back[0].owner.as_str(), back[0].owner_name.as_str(), back[0].due.as_str()), ("", "", ""));
    }

    #[tokio::test]
    async fn saving_again_replaces_and_never_duplicates() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", None).await;
        AssistantItemsRepository::replace_for_meeting(&pool, "m1", &[item("a", 1.0), item("b", 2.0)])
            .await
            .unwrap();
        AssistantItemsRepository::replace_for_meeting(&pool, "m1", &[item("c", 3.0)]).await.unwrap();
        let back = AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id, "c");
    }

    #[tokio::test]
    async fn delete_removes_only_that_meetings_rows() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", None).await;
        add_meeting(&pool, "m2", None).await;
        AssistantItemsRepository::replace_for_meeting(&pool, "m1", &[item("a", 1.0), item("b", 2.0)])
            .await
            .unwrap();
        AssistantItemsRepository::replace_for_meeting(&pool, "m2", &[item("c", 1.0)]).await.unwrap();
        assert_eq!(AssistantItemsRepository::delete_for_meeting(&pool, "m1").await.unwrap(), 2);
        assert!(AssistantItemsRepository::list_for_meeting(&pool, "m1").await.unwrap().is_empty());
        assert_eq!(AssistantItemsRepository::list_for_meeting(&pool, "m2").await.unwrap().len(), 1);
    }
}
