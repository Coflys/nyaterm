pub use nyaterm_core::core::ai::history::*;
pub async fn clear_ai_history(app: &tauri::AppHandle) -> crate::error::AppResult<()> {
    use tauri::Manager;
    super::stream::cancel_all_chat_streams();
    if let Some(manager) = app.try_state::<std::sync::Arc<super::CodexAppServerManager>>() {
        manager.release_all_sessions().await;
    }
    nyaterm_core::core::ai::history::clear_ai_history_storage(app)
}

pub async fn delete_ai_session(
    app: &tauri::AppHandle,
    session_id: String,
) -> crate::error::AppResult<()> {
    use tauri::Manager;
    if let Some(manager) = app.try_state::<std::sync::Arc<super::CodexAppServerManager>>() {
        manager.release_session(&session_id).await;
    }
    nyaterm_core::core::ai::history::delete_ai_session(app, session_id)
}
