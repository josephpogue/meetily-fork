// assistant/cleanup_run.rs
//
// The running side of picture cleanup: starting the Claude run beside the note,
// finishing it (parse, move, record), Try again, the commands the meeting page
// calls, and the purge sweep. The file work itself is in `cleanup.rs`.
//
// The run forks the call's deep session from a shared handle on the lanes, so
// it never takes the core lock the note run holds: both run side by side.

use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, State, Wry};

use super::cleanup::{self, CleanupRecord, CleanupState, CleanupSummary, Picture, RemovedPicture, SavedSession};
use super::history::{self, HistoryHandle};
use super::lanes::{claude_runner, AnswerLanes, LaneConfig};
use super::settings::{self, AssistantSettings};
use crate::state::AppState;

/// Sent to the app when a cleanup run ends; the payload is the meeting folder.
pub const EVENT: &str = "assistant-cleanup";

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The recording folder of the call that just ended: the saved meeting's
/// folder in the database. None until the meeting is saved.
pub async fn ended_meeting_folder(app: Option<&AppHandle<Wry>>, history: &HistoryHandle) -> Option<PathBuf> {
    let id = history.lock().unwrap().saved_meeting_id()?;
    let state = app?.try_state::<AppState>()?;
    history::folder_path_of(state.db_manager.pool(), &id).await.map(PathBuf::from)
}

/// Starts the cleanup beside the note and returns at once. Starts nothing, and
/// writes nothing, when there is no deep session, no pictures, a finished
/// cleanup, or a run already in flight. Never affects the note.
pub fn start_beside_note(app: Option<AppHandle<Wry>>, folder: PathBuf, lanes: AnswerLanes, transcript: String) {
    if lanes.is_closed() {
        return;
    }
    let Some((session_id, cfg)) = lanes.deep_session() else { return };
    let pictures = cleanup::list_pictures(&folder);
    if pictures.is_empty() {
        return;
    }
    if !cleanup::should_start(cleanup::load(&folder).as_ref(), cleanup::run_in_flight(&folder)) {
        return;
    }
    if !cleanup::begin_run(&folder) {
        return;
    }
    cleanup::save_session(
        &folder,
        &SavedSession {
            session_id,
            cwd: cfg.cwd.clone(),
            model: cfg.deep_model.clone(),
            effort: cfg.deep_effort.clone(),
            read_dirs: cfg.deep_read_dirs.clone(),
            sent: Vec::new(),
        },
    );
    tokio::spawn(run(app, folder, lanes, transcript, pictures));
}

async fn run(app: Option<AppHandle<Wry>>, folder: PathBuf, lanes: AnswerLanes, transcript: String, pictures: Vec<Picture>) {
    let result = lanes.run_cleanup(transcript, cleanup::picture_list(&pictures)).await;
    // A new recording closed these lanes mid-run: nothing moves and no record
    // is written, so the next Draft note may run it.
    if !lanes.is_closed() {
        finish(&folder, &pictures, result);
        if let Some(app) = &app {
            if let Err(e) = app.emit(EVENT, folder.to_string_lossy().to_string()) {
                log::warn!("picture cleanup: failed to emit {}: {}", EVENT, e);
            }
        }
    }
    cleanup::end_run(&folder);
}

/// Parses the reply, moves the pictures and writes the record; a failure at
/// any step writes a failed record and moves nothing more.
fn finish(folder: &Path, pictures: &[Picture], result: Result<String, String>) {
    let now = now_ms();
    let outcome = match result {
        Ok(raw) => cleanup::parse_reply(&raw, pictures)
            .map_err(|e| {
                log::warn!("picture cleanup: {}", e);
                "Claude's reply could not be read.".to_string()
            })
            .and_then(|removals| cleanup::apply(folder, pictures, &removals, now)),
        Err(e) => {
            log::warn!("picture cleanup: the claude run failed: {}", e);
            Err("The Claude run did not finish.".to_string())
        }
    };
    if let Err(error) = outcome {
        if let Err(e) = cleanup::record_failure(folder, &error, now) {
            log::warn!("picture cleanup: {}", e);
        }
    }
}

/// Try again: runs the cleanup alone for a meeting whose record is Failed, by
/// forking the deep session saved with the meeting (so it works after a restart).
pub async fn retry(app: AppHandle<Wry>, pool: &SqlitePool, meeting_id: &str) -> Result<(), String> {
    let folder = folder_of(pool, meeting_id).await?;
    if !cleanup::should_start(cleanup::load(&folder).as_ref(), cleanup::run_in_flight(&folder)) {
        return Err("Picture cleanup is not waiting for a retry.".to_string());
    }
    let session = cleanup::load_session(&folder).ok_or("This call's Claude session was not saved, so it cannot be tried again.")?;
    let pictures = cleanup::list_pictures(&folder);
    if pictures.is_empty() {
        return Err("There are no pictures to clean up.".to_string());
    }
    let settings = AssistantSettings::load(pool).await.map_err(|e| e.to_string())?;
    let bin = settings::resolve_claude_binary(&settings).ok_or("The claude program was not found.")?;
    let transcript = sqlx::query_scalar::<_, String>("SELECT transcript FROM transcripts WHERE meeting_id = ? ORDER BY timestamp")
        .bind(meeting_id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?
        .join("\n");
    let cfg = LaneConfig {
        cwd: session.cwd,
        fast_model: String::new(),
        fast_effort: String::new(),
        deep_model: session.model,
        deep_effort: session.effort,
        deep_read_dirs: session.read_dirs,
        note_prompt: String::new(),
    };
    let lanes = AnswerLanes::from_saved_session(claude_runner(bin), session.session_id, cfg);
    if !cleanup::begin_run(&folder) {
        return Err("Picture cleanup is already running.".to_string());
    }
    tokio::spawn(run(Some(app), folder, lanes, transcript, pictures));
    Ok(())
}

async fn folder_of(pool: &SqlitePool, meeting_id: &str) -> Result<PathBuf, String> {
    history::folder_path_of(pool, meeting_id)
        .await
        .map(PathBuf::from)
        .ok_or_else(|| "This meeting has no recording folder.".to_string())
}

/// Deletes every expired `removed/` folder. Called on app start and when the
/// meeting list loads.
pub async fn purge_expired_all(pool: &SqlitePool) {
    let folders: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT folder_path FROM meetings WHERE folder_path IS NOT NULL AND TRIM(folder_path) != ''",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let now = now_ms();
    let purged = tokio::task::spawn_blocking(move || {
        folders.iter().filter(|f| cleanup::purge_if_expired(Path::new(f), now)).count()
    })
    .await
    .unwrap_or(0);
    if purged > 0 {
        log::info!("picture cleanup: deleted the removed pictures of {} meeting(s)", purged);
    }
}

/// What the meeting page shows for one meeting's cleanup.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupView {
    pub summary: CleanupSummary,
    /// Every removed picture, restored ones included (they show a Restored tag).
    pub removed: Vec<RemovedPicture>,
    /// A failed run whose session was saved: Try again is offered.
    pub can_retry: bool,
    /// A run is in flight.
    pub running: bool,
    /// "93 pictures removed, 51 MB freed on Mon, Nov 23"
    pub title: Option<String>,
    /// "Claude kept the 44 that show something new. ... deleted for good on Mon, Nov 23."
    pub note: Option<String>,
    /// "You can bring them back until Mon, Nov 23."
    pub bring_back: Option<String>,
    /// "Mon, Nov 23": when the removed pictures are deleted for good.
    pub until: Option<String>,
}

/// "Mon, Nov 23" in local time.
fn day_text(ms: i64) -> Option<String> {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|t| t.format("%a, %b %-d").to_string())
}

fn view_of(folder: &Path, record: Option<CleanupRecord>) -> CleanupView {
    let summary = cleanup::summary(record.as_ref(), now_ms());
    let can_retry = summary.failed && cleanup::load_session(folder).is_some();
    let removed = match record {
        Some(CleanupRecord { state: CleanupState::Done, removed, .. }) => removed,
        _ => Vec::new(),
    };
    let until = if summary.removed > 0 { day_text(summary.delete_after_ms) } else { None };
    let (mut title, mut note, mut bring_back) = (None, None, None);
    if let Some(day) = until.clone() {
        let kept = cleanup::list_pictures(folder).len();
        title = Some(format!(
            "{} removed, {} freed on {}",
            crate::meeting_saved::format_pictures(summary.removed),
            crate::meeting_saved::format_size(summary.bytes),
            day
        ));
        note = Some(format!(
            "Claude kept the {} that show something new. Click a picture to see it full size. Removed pictures are deleted for good on {}.",
            kept, day
        ));
        bring_back = Some(format!("You can bring them back until {}.", day));
    }
    CleanupView { summary, removed, can_retry, running: cleanup::run_in_flight(folder), title, note, bring_back, until }
}

/// A path the page may ask for: inside a frames folder, no way out of it.
fn allowed_picture(file: &str) -> bool {
    let path = Path::new(file);
    let parts: Vec<Component> = path.components().collect();
    parts.len() >= 2
        && parts.iter().all(|c| matches!(c, Component::Normal(_)))
        && cleanup::FRAME_DIRS.iter().any(|d| path.starts_with(d))
        && path.extension().map(|e| e.eq_ignore_ascii_case("jpg")).unwrap_or(false)
}

#[tauri::command]
pub async fn picture_cleanup_get(state: State<'_, AppState>, meeting_id: String) -> Result<Option<CleanupView>, String> {
    let Some(folder) = history::folder_path_of(state.db_manager.pool(), &meeting_id).await.map(PathBuf::from) else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || {
        cleanup::purge_if_expired(&folder, now_ms());
        let record = cleanup::load(&folder);
        record.is_some().then(|| view_of(&folder, record))
    })
    .await
    .map_err(|e| e.to_string())
}

/// Restores the named pictures (their `file` values); an empty list restores all.
#[tauri::command]
pub async fn picture_cleanup_restore(
    state: State<'_, AppState>,
    meeting_id: String,
    files: Vec<String>,
) -> Result<CleanupView, String> {
    let folder = folder_of(state.db_manager.pool(), &meeting_id).await?;
    tokio::task::spawn_blocking(move || {
        let record = cleanup::restore(&folder, &files)?;
        Ok(view_of(&folder, Some(record)))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Undo for a restore: puts the named restored pictures back into `removed/`.
#[tauri::command]
pub async fn picture_cleanup_undo_restore(
    state: State<'_, AppState>,
    meeting_id: String,
    files: Vec<String>,
) -> Result<CleanupView, String> {
    let folder = folder_of(state.db_manager.pool(), &meeting_id).await?;
    tokio::task::spawn_blocking(move || {
        let record = cleanup::remove_again(&folder, &files)?;
        Ok(view_of(&folder, Some(record)))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn picture_cleanup_retry(app: AppHandle<Wry>, state: State<'_, AppState>, meeting_id: String) -> Result<(), String> {
    retry(app, state.db_manager.pool(), &meeting_id).await
}

/// One picture of the meeting as a data URL, for the panel's thumbnails and the
/// full-size viewer. `file` is a record-relative path, kept or removed.
#[tauri::command]
pub async fn picture_cleanup_picture(state: State<'_, AppState>, meeting_id: String, file: String) -> Result<String, String> {
    if !allowed_picture(&file) {
        return Err("not a picture of this meeting".to_string());
    }
    let folder = folder_of(state.db_manager.pool(), &meeting_id).await?;
    tokio::task::spawn_blocking(move || {
        let img = super::vision::encode::for_send(&cleanup::absolute(&folder, &file))?;
        Ok(format!("data:{};base64,{}", img.media_type, img.base64))
    })
    .await
    .map_err(|e| e.to_string())?
}
