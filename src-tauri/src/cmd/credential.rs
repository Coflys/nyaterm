use crate::config;
use crate::config::SavedCredential;
use crate::error::AppResult;
use tauri::Emitter;

fn schedule_cloud_sync_notify(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        crate::core::cloud_sync::notify_config_changed(&app).await;
    });
}

/// Frontend-facing credential summary.
///
/// The encrypted `password` blob never leaves the backend. `has_password` is a
/// transient field that `SavedCredential` skips during serialization, so it is
/// surfaced explicitly here — the terminal credential autofill requires it
/// before a credential can match a password prompt.
#[derive(Debug, serde::Serialize)]
pub struct SavedCredentialSummary {
    id: String,
    sort_order: i32,
    name: String,
    username: String,
    username_prompt_regex: Option<String>,
    password_prompt_regex: Option<String>,
    enabled: bool,
    has_password: bool,
}

impl From<SavedCredential> for SavedCredentialSummary {
    fn from(entry: SavedCredential) -> Self {
        Self {
            id: entry.id,
            sort_order: entry.sort_order,
            name: entry.name,
            username: entry.username,
            username_prompt_regex: entry.username_prompt_regex,
            password_prompt_regex: entry.password_prompt_regex,
            enabled: entry.enabled,
            has_password: entry.has_password,
        }
    }
}

#[tauri::command]
pub fn get_saved_credentials(app: tauri::AppHandle) -> AppResult<Vec<SavedCredentialSummary>> {
    Ok(config::load_credentials(&app)?
        .credentials
        .into_iter()
        .map(SavedCredentialSummary::from)
        .collect())
}

#[tauri::command]
pub fn get_saved_credential_password(
    app: tauri::AppHandle,
    id: String,
) -> AppResult<Option<String>> {
    Ok(config::load_credential_by_id(&app, &id)?.password)
}

#[tauri::command]
pub fn save_credential(app: tauri::AppHandle, entry: SavedCredential) -> AppResult<String> {
    let mut cfg = config::load_credentials(&app)?;
    let target_id = config::upsert_credential(&mut cfg, entry)?;
    config::save_credentials(&app, &cfg)?;
    let _ = app.emit("credentials-changed", ());
    schedule_cloud_sync_notify(app.clone());
    Ok(target_id)
}

#[tauri::command]
pub fn delete_credential(app: tauri::AppHandle, id: String) -> AppResult<()> {
    let mut cfg = config::load_credentials(&app)?;
    cfg.credentials.retain(|credential| credential.id != id);
    config::save_credentials(&app, &cfg)?;
    let _ = app.emit("credentials-changed", ());
    schedule_cloud_sync_notify(app.clone());
    Ok(())
}

#[derive(serde::Deserialize)]
pub struct CredentialSortOrderUpdate {
    pub id: String,
    pub sort_order: i32,
}

#[tauri::command]
pub fn reorder_credentials(
    app: tauri::AppHandle,
    updates: Vec<CredentialSortOrderUpdate>,
) -> AppResult<()> {
    let mut cfg = config::load_credentials(&app)?;
    let updates = updates
        .into_iter()
        .map(|update| (update.id, update.sort_order))
        .collect::<Vec<_>>();
    config::reorder_credentials(&mut cfg, &updates);
    config::save_credentials(&app, &cfg)?;
    let _ = app.emit("credentials-changed", ());
    schedule_cloud_sync_notify(app.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential(has_password: bool) -> SavedCredential {
        SavedCredential {
            id: "cred-1".to_string(),
            sort_order: 0,
            name: "Production".to_string(),
            username: "alice".to_string(),
            password: if has_password {
                Some("encrypted-secret".to_string())
            } else {
                None
            },
            username_prompt_regex: Some("login as:".to_string()),
            password_prompt_regex: Some("Password:".to_string()),
            enabled: true,
            has_password,
        }
    }

    #[test]
    fn saved_credential_list_redacts_password_but_reports_has_password() {
        let summary = SavedCredentialSummary::from(credential(true));
        let serialized = serde_json::to_value(&summary).expect("serialize credential summary");

        assert_eq!(serialized["has_password"], true);
        assert_eq!(serialized["password_prompt_regex"], "Password:");
        assert_eq!(serialized["username_prompt_regex"], "login as:");
        assert!(serialized.get("password").is_none());
    }

    #[test]
    fn saved_credential_list_reports_missing_password() {
        let summary = SavedCredentialSummary::from(credential(false));
        let serialized = serde_json::to_value(&summary).expect("serialize credential summary");

        assert_eq!(serialized["has_password"], false);
    }
}
