// The per-meeting record of the live assistant: the Brief Joseph typed, and
// where the meeting was filed in the vault (filled by the filing step).

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMeta {
    pub meeting_id: String,
    pub brief: String,
    pub vault_project: Option<String>,
    pub vault_note_path: Option<String>,
    pub vault_transcript_path: Option<String>,
    pub filed_at: Option<String>,
}

/// A meeting Joseph filed before, for the note writer's project hints.
#[derive(Debug, Clone, PartialEq)]
pub struct FiledMeeting {
    pub title: String,
    pub brief: String,
    pub project: String,
}

pub struct AssistantMetaRepository;

impl AssistantMetaRepository {
    pub async fn get(pool: &SqlitePool, meeting_id: &str) -> Result<Option<AssistantMeta>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT meeting_id, brief, vault_project, vault_note_path, vault_transcript_path, filed_at \
             FROM meeting_assistant_meta WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|r| AssistantMeta {
            meeting_id: r.get("meeting_id"),
            brief: r.get("brief"),
            vault_project: r.get("vault_project"),
            vault_note_path: r.get("vault_note_path"),
            vault_transcript_path: r.get("vault_transcript_path"),
            filed_at: r.get("filed_at"),
        }))
    }

    /// The `limit` most recently filed meetings that were put under a project, newest first.
    pub async fn recent_filed(pool: &SqlitePool, limit: i64) -> Result<Vec<FiledMeeting>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT m.title, a.brief, a.vault_project FROM meeting_assistant_meta a \
             JOIN meetings m ON m.id = a.meeting_id \
             WHERE a.vault_project IS NOT NULL AND a.filed_at IS NOT NULL \
             ORDER BY a.filed_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| FiledMeeting { title: r.get("title"), brief: r.get("brief"), project: r.get("vault_project") })
            .collect())
    }

    /// Inserts the row, or updates only `brief` so the vault columns stay.
    pub async fn set_brief(pool: &SqlitePool, meeting_id: &str, brief: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO meeting_assistant_meta (meeting_id, brief) VALUES (?, ?) \
             ON CONFLICT(meeting_id) DO UPDATE SET brief = excluded.brief",
        )
        .bind(meeting_id)
        .bind(brief)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Records where the meeting was filed: updates only the vault columns,
    /// inserting the row (empty brief) if there is none.
    pub async fn set_filed(
        pool: &SqlitePool,
        meeting_id: &str,
        project: Option<&str>,
        note_path: &str,
        transcript_path: &str,
        filed_at: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO meeting_assistant_meta \
             (meeting_id, vault_project, vault_note_path, vault_transcript_path, filed_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(meeting_id) DO UPDATE SET \
             vault_project = excluded.vault_project, \
             vault_note_path = excluded.vault_note_path, \
             vault_transcript_path = excluded.vault_transcript_path, \
             filed_at = excluded.filed_at",
        )
        .bind(meeting_id)
        .bind(project)
        .bind(note_path)
        .bind(transcript_path)
        .bind(filed_at)
        .execute(pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::assistant_items::tests::{add_meeting, test_pool};

    #[tokio::test]
    async fn brief_then_filed_keep_each_others_columns() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", None).await;
        assert_eq!(AssistantMetaRepository::get(&pool, "m1").await.unwrap(), None);
        AssistantMetaRepository::set_brief(&pool, "m1", "PNC sync").await.unwrap();
        AssistantMetaRepository::set_filed(&pool, "m1", Some("p"), "/v/n.md", "/v/t.md", "2026-10-21T15:00:00Z")
            .await
            .unwrap();
        AssistantMetaRepository::set_brief(&pool, "m1", "PNC sync with Jim").await.unwrap();
        let m = AssistantMetaRepository::get(&pool, "m1").await.unwrap().unwrap();
        assert_eq!(m.brief, "PNC sync with Jim");
        assert_eq!(m.vault_project.as_deref(), Some("p"));
        assert_eq!(m.vault_note_path.as_deref(), Some("/v/n.md"));
        assert_eq!(m.vault_transcript_path.as_deref(), Some("/v/t.md"));
        assert_eq!(m.filed_at.as_deref(), Some("2026-10-21T15:00:00Z"));
    }

    #[tokio::test]
    async fn set_filed_inserts_a_row_with_empty_brief() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", None).await;
        AssistantMetaRepository::set_filed(&pool, "m1", None, "/n", "/t", "x").await.unwrap();
        let m = AssistantMetaRepository::get(&pool, "m1").await.unwrap().unwrap();
        assert_eq!((m.brief.as_str(), m.vault_project), ("", None));
    }

    #[tokio::test]
    async fn recent_filed_lists_project_filings_newest_first() {
        let pool = test_pool().await;
        for id in ["m1", "m2", "m3", "m4"] {
            add_meeting(&pool, id, None).await;
        }
        let file = |id: &'static str, project: Option<&'static str>, at: &'static str| {
            let pool = pool.clone();
            async move { AssistantMetaRepository::set_filed(&pool, id, project, "/n", "/t", at).await.unwrap() }
        };
        file("m1", Some("a"), "2026-11-01T10:00:00Z").await;
        file("m2", Some("b"), "2026-11-03T10:00:00Z").await;
        file("m3", None, "2026-11-04T10:00:00Z").await;
        AssistantMetaRepository::set_brief(&pool, "m2", "second").await.unwrap();
        let got = AssistantMetaRepository::recent_filed(&pool, 10).await.unwrap();
        let seen: Vec<(&str, &str)> = got.iter().map(|f| (f.project.as_str(), f.brief.as_str())).collect();
        assert_eq!(seen, vec![("b", "second"), ("a", "")]);
        assert_eq!(AssistantMetaRepository::recent_filed(&pool, 1).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn deleting_the_meeting_removes_the_row() {
        let pool = test_pool().await;
        add_meeting(&pool, "m1", None).await;
        AssistantMetaRepository::set_brief(&pool, "m1", "b").await.unwrap();
        sqlx::query("DELETE FROM meetings WHERE id = 'm1'").execute(&pool).await.unwrap();
        assert_eq!(AssistantMetaRepository::get(&pool, "m1").await.unwrap(), None);
    }
}
