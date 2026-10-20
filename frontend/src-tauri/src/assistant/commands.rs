// assistant/commands.rs
//
// Tauri commands the assistant panel calls. Each one is a thin adapter over
// a plain async function in core.rs; the real logic (and its tests) live
// there, not here.

use tauri::State;

use super::core::{self, StatusOut};
use super::lanes::CardKind;
use super::AssistantHandle;

#[tauri::command]
pub async fn assistant_get_state(handle: State<'_, AssistantHandle>) -> Result<StatusOut, String> {
    Ok(core::get_state(handle.inner()).await)
}

#[tauri::command]
pub async fn assistant_set_enabled(handle: State<'_, AssistantHandle>, enabled: bool) -> Result<(), String> {
    core::set_enabled(handle.inner(), enabled).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_ask(handle: State<'_, AssistantHandle>, text: String) -> Result<(), String> {
    core::ask(
        handle.inner(),
        text,
        CardKind::Ask,
        Some("This question was asked by Joseph directly. Do not reply SKIP."),
    )
    .await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_explain(handle: State<'_, AssistantHandle>) -> Result<(), String> {
    core::explain(handle.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_catchup(handle: State<'_, AssistantHandle>) -> Result<(), String> {
    core::catchup(handle.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_set_mode(handle: State<'_, AssistantHandle>, mode: String) -> Result<(), String> {
    core::set_mode(handle.inner(), &mode).await
}

#[tauri::command]
pub async fn assistant_set_listening(
    handle: State<'_, AssistantHandle>,
    listening: bool,
) -> Result<(), String> {
    core::set_listening(handle.inner(), listening).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_voice_start(handle: State<'_, AssistantHandle>) -> Result<(), String> {
    core::voice_start(handle.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_voice_finish(handle: State<'_, AssistantHandle>) -> Result<(), String> {
    core::voice_finish(handle.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_voice_cancel(handle: State<'_, AssistantHandle>) -> Result<(), String> {
    core::voice_cancel(handle.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_draft_note(handle: State<'_, AssistantHandle>) -> Result<(), String> {
    core::draft_note(handle.inner()).await
}

#[tauri::command]
pub async fn assistant_save_note(handle: State<'_, AssistantHandle>) -> Result<(), String> {
    core::save_note(handle.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_discard_note(handle: State<'_, AssistantHandle>) -> Result<(), String> {
    core::discard_note(handle.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_set_brief(handle: State<'_, AssistantHandle>, text: String) -> Result<(), String> {
    core::set_brief(handle.inner(), text).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_refresh_asks(handle: State<'_, AssistantHandle>) -> Result<(), String> {
    core::refresh_asks(handle.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_dismiss_ask(handle: State<'_, AssistantHandle>, id: String) -> Result<(), String> {
    core::dismiss_ask(handle.inner(), id).await;
    Ok(())
}

/// Joseph typed the answer to a For-you question: the item moves to the
/// answered list. The panel appends the answer to the brief itself.
#[tauri::command]
pub async fn assistant_answer_ask(
    handle: State<'_, AssistantHandle>,
    id: String,
    answer: String,
) -> Result<(), String> {
    core::answer_ask(handle.inner(), id, answer).await;
    Ok(())
}

#[tauri::command]
pub async fn assistant_screen_recording_allowed() -> Result<bool, String> {
    Ok(super::vision::permission::screen_recording_allowed())
}

#[tauri::command]
pub async fn assistant_open_screen_recording_settings() -> Result<(), String> {
    super::vision::permission::open_screen_recording_settings()
}
