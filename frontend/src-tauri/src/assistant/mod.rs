// assistant/mod.rs
//
// The live meeting assistant: claude CLI runner, answer lanes, trigger engine,
// voice ask, note drafter, Tauri commands. Fully isolated from recording and
// transcription; every entry point here must catch its own errors so an
// assistant failure never touches the recording path.

pub mod card;
pub mod ask_lane;
pub mod ask_list;
pub mod claude_cli;
pub mod commands;
pub mod core;
pub mod lanes;
pub mod note;
pub mod settings;
pub mod transcript;
pub mod trigger;
pub mod vision;
pub mod voice_ask;

use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

pub use core::AssistantCore;
pub use settings::{AssistantSettings, ClaudeProbe};

use crate::state::AppState;

#[derive(Clone)]
pub struct AssistantHandle(pub Arc<Mutex<AssistantCore>>);

impl AssistantHandle {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(AssistantCore::default())))
    }
}

impl Default for AssistantHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[tauri::command]
pub async fn assistant_get_settings(
    state: State<'_, AppState>,
) -> Result<AssistantSettings, String> {
    AssistantSettings::load(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn assistant_save_settings(
    state: State<'_, AppState>,
    handle: State<'_, AssistantHandle>,
    settings: AssistantSettings,
) -> Result<(), String> {
    settings
        .save(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    // The switch also reaches a call that is already running.
    core::apply_see_zoom_window(handle.inner(), settings.see_zoom_window).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_test_claude(state: State<'_, AppState>) -> Result<ClaudeProbe, String> {
    let settings = AssistantSettings::load(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(settings::probe_claude(&settings).await)
}
