// assistant/settings.rs
//
// Single-row assistant_settings table, plus claude binary resolution and probing.
// vault_root empty means ~/brain/wiki resolved at use time; deep_read_dirs empty means ~/brain.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantSettings {
    pub enabled: bool,
    pub claude_path: String,
    pub fast_model: String,
    pub fast_effort: String,
    pub deep_model: String,
    pub deep_effort: String,
    pub trigger_mode: String,
    pub quiet_gap_secs: f64,
    pub names: String,
    pub vault_root: String,
    pub deep_read_dirs: String,
    /// "See the Zoom or Teams window": capture the Zoom (else Teams) meeting window and send
    /// changed frames to the lanes. On by default.
    pub see_zoom_window: bool,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            claude_path: String::new(),
            fast_model: "claude-sonnet-5".to_string(),
            fast_effort: "medium".to_string(),
            deep_model: "claude-opus-5".to_string(),
            deep_effort: "medium".to_string(),
            trigger_mode: "gated".to_string(),
            quiet_gap_secs: 2.0,
            names: "joseph,joe".to_string(),
            vault_root: String::new(),
            deep_read_dirs: String::new(),
            see_zoom_window: true,
        }
    }
}

impl AssistantSettings {
    pub async fn load(pool: &SqlitePool) -> Result<Self, sqlx::Error> {
        sqlx::query_as::<_, Self>(
            r#"
            SELECT enabled, claude_path, fast_model, fast_effort, deep_model, deep_effort,
                   trigger_mode, quiet_gap_secs, names, vault_root, deep_read_dirs, see_zoom_window
            FROM assistant_settings WHERE id = '1'
            "#,
        )
        .fetch_one(pool)
        .await
    }

    pub async fn save(&self, pool: &SqlitePool) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO assistant_settings (
                id, enabled, claude_path, fast_model, fast_effort, deep_model, deep_effort,
                trigger_mode, quiet_gap_secs, names, vault_root, deep_read_dirs, see_zoom_window
            ) VALUES ('1', $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT(id) DO UPDATE SET
                enabled = excluded.enabled,
                claude_path = excluded.claude_path,
                fast_model = excluded.fast_model,
                fast_effort = excluded.fast_effort,
                deep_model = excluded.deep_model,
                deep_effort = excluded.deep_effort,
                trigger_mode = excluded.trigger_mode,
                quiet_gap_secs = excluded.quiet_gap_secs,
                names = excluded.names,
                vault_root = excluded.vault_root,
                deep_read_dirs = excluded.deep_read_dirs,
                see_zoom_window = excluded.see_zoom_window
            "#,
        )
        .bind(self.enabled)
        .bind(&self.claude_path)
        .bind(&self.fast_model)
        .bind(&self.fast_effort)
        .bind(&self.deep_model)
        .bind(&self.deep_effort)
        .bind(&self.trigger_mode)
        .bind(self.quiet_gap_secs)
        .bind(&self.names)
        .bind(&self.vault_root)
        .bind(&self.deep_read_dirs)
        .bind(self.see_zoom_window)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Persists just the `enabled` column, independent of `save`'s full-row
    /// upsert. The panel switch only ever knows the on/off it just set, not
    /// the rest of the settings row (fast/deep model, names, etc.); a full
    /// `save()` from a possibly-stale in-memory `AssistantSettings` would
    /// silently clobber whatever the settings page saved after that.
    pub async fn persist_enabled(pool: &SqlitePool, enabled: bool) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO assistant_settings (id, enabled) VALUES ('1', $1)
            ON CONFLICT(id) DO UPDATE SET enabled = excluded.enabled
            "#,
        )
        .bind(enabled)
        .execute(pool)
        .await?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeProbe {
    pub ok: bool,
    pub version: String,
    pub error: Option<String>,
}

/// Never bundled, never the llama-helper sidecar (that one idles out and is
/// name-hardcoded to a different lifecycle). Order: settings path, `which claude`,
/// then common install paths.
pub fn resolve_claude_binary(settings: &AssistantSettings) -> Option<PathBuf> {
    if !settings.claude_path.is_empty() {
        let configured = PathBuf::from(&settings.claude_path);
        if configured.is_file() {
            return Some(configured);
        }
    }

    if let Ok(found) = which::which("claude") {
        return Some(found);
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/claude"));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/claude"));

    candidates.into_iter().find(|p| p.is_file())
}

/// Runs `claude --version` with a 10s timeout to confirm the binary resolves and auth works.
pub async fn probe_claude(settings: &AssistantSettings) -> ClaudeProbe {
    let Some(bin) = resolve_claude_binary(settings) else {
        return ClaudeProbe {
            ok: false,
            version: String::new(),
            error: Some("claude binary not found".to_string()),
        };
    };

    let run = tokio::process::Command::new(&bin).arg("--version").output();

    match tokio::time::timeout(Duration::from_secs(10), run).await {
        Ok(Ok(output)) if output.status.success() => ClaudeProbe {
            ok: true,
            version: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            error: None,
        },
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            ClaudeProbe {
                ok: false,
                version: String::new(),
                error: Some(if stderr.is_empty() {
                    "claude --version failed".to_string()
                } else {
                    stderr
                }),
            }
        }
        Ok(Err(e)) => ClaudeProbe {
            ok: false,
            version: String::new(),
            error: Some(e.to_string()),
        },
        Err(_) => ClaudeProbe {
            ok: false,
            version: String::new(),
            error: Some("claude --version timed out".to_string()),
        },
    }
}

#[cfg(test)]
/// In-memory db with the real migrations applied, for assistant module tests.
pub(crate) async fn test_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("failed to open in-memory sqlite pool");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("failed to run migrations");
    pool
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn settings_defaults_load() {
        let pool = test_pool().await;
        let s = AssistantSettings::load(&pool).await.unwrap();
        assert!(s.enabled);
        assert_eq!(s.fast_model, "claude-sonnet-5");
        assert_eq!(s.trigger_mode, "gated");
        assert!(s.see_zoom_window);
    }

    #[tokio::test]
    async fn see_zoom_window_round_trips() {
        let pool = test_pool().await;
        let mut s = AssistantSettings::load(&pool).await.unwrap();
        s.see_zoom_window = false;
        s.save(&pool).await.unwrap();
        assert!(!AssistantSettings::load(&pool).await.unwrap().see_zoom_window);
    }

    #[tokio::test]
    async fn settings_save_round_trips() {
        let pool = test_pool().await;
        let mut s = AssistantSettings::load(&pool).await.unwrap();
        s.enabled = false;
        s.trigger_mode = "continuous".to_string();
        s.names = "joseph,jp".to_string();
        s.save(&pool).await.unwrap();

        let reloaded = AssistantSettings::load(&pool).await.unwrap();
        assert!(!reloaded.enabled);
        assert_eq!(reloaded.trigger_mode, "continuous");
        assert_eq!(reloaded.names, "joseph,jp");
    }

    #[tokio::test]
    async fn persist_enabled_touches_only_the_enabled_column() {
        let pool = test_pool().await;
        let mut s = AssistantSettings::load(&pool).await.unwrap();
        s.trigger_mode = "continuous".to_string();
        s.names = "joseph,jp".to_string();
        s.save(&pool).await.unwrap();

        AssistantSettings::persist_enabled(&pool, false).await.unwrap();

        let reloaded = AssistantSettings::load(&pool).await.unwrap();
        assert!(!reloaded.enabled);
        // Untouched by the narrow update.
        assert_eq!(reloaded.trigger_mode, "continuous");
        assert_eq!(reloaded.names, "joseph,jp");
    }
}
