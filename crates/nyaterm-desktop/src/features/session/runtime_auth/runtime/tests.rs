use std::sync::Arc;

use gpui::AppContext as _;

use nyaterm_core::test_support::TestTempDir;
use nyaterm_core::{SavedConnection, SavedPassword};
use nyaterm_store::ConnectionStore;
use nyaterm_transport::connection_attempt::ConnectionAttempt;
use nyaterm_transport::ssh_authentication::{
    SshAuthenticationMaterial, SshAuthenticationMethod, SshAuthenticationRequest,
};
use nyaterm_transport::{SshCredentialPrompt, SshCredentialPromptKind, SshCredentialPromptReason};
use serde_json::json;

use super::prepare_response;
use crate::features::session::runtime_auth::state::{
    RuntimeAuthenticationContext, RuntimeAuthenticationForm,
};

fn request() -> SshAuthenticationRequest {
    SshAuthenticationRequest {
        prompt: SshCredentialPrompt {
            host: "localhost".into(),
            port: 22,
            username: "actual-user".into(),
            connection_name: "fixture".into(),
            kind: SshCredentialPromptKind::Password,
            reason: SshCredentialPromptReason::MissingPassword,
            attempt: 1,
            prompt_text: None,
            echo: false,
        },
        available_methods: vec![SshAuthenticationMethod::Password],
        key_response_id: None,
    }
}

fn store() -> (TestTempDir, ConnectionStore) {
    let dir = TestTempDir::new("nyaterm-desktop-runtime-auth");
    let store = ConnectionStore::open(dir.path()).unwrap();
    let connection: SavedConnection = serde_json::from_value(json!({
        "id":"fixture", "name":"fixture", "type":"ssh", "host":"localhost",
        "username":"actual-user", "auth":{"mode":"password"}
    }))
    .unwrap();
    store.save_connection(&connection).unwrap();
    (dir, store)
}

#[test]
fn saved_account_uses_only_its_password_and_reference_is_written_only_after_success() {
    let (_dir, store) = store();
    let id = store
        .save_password(SavedPassword {
            id: String::new(),
            name: "account".into(),
            username: "other-user".into(),
            password: Some("saved secret".into()),
            sort_order: 0,
            has_password: false,
        })
        .unwrap();
    let context = Arc::new(RuntimeAuthenticationContext::new(
        Some(store.runtime_connection_snapshot("fixture").unwrap()),
        None,
        None,
        None,
        ConnectionAttempt::default(),
    ));
    let mut form = RuntimeAuthenticationForm::new(request(), context.clone());
    form.saved_password = true;
    form.password_id = Some(id.clone());
    form.remember = true;
    let (response, key, save) = prepare_response(&store, &form, "ignored".into()).unwrap();
    assert!(
        matches!(response.material, SshAuthenticationMaterial::Password(ref p) if p == "saved secret")
    );
    assert!(key.is_none());
    assert!(
        store
            .get_connection("fixture")
            .unwrap()
            .unwrap()
            .auth
            .unwrap()
            .account_id
            .is_none()
    );
    let response_id = response.response_id.unwrap();
    context
        .remember_selection(response_id.clone(), None, None, save)
        .unwrap();
    let saves = context.finish(Some(&[response_id]));
    assert_eq!(saves.len(), 1);
    assert!(context.finish(Some(&[])).is_empty());
    for save in saves {
        store.save_runtime_credential(save).unwrap();
    }
    let connection = store.get_connection("fixture").unwrap().unwrap();
    let serialized = serde_json::to_value(connection).unwrap();
    assert_eq!(serialized["username"], "actual-user");
    assert_eq!(serialized["auth"]["account_id"], id);
    assert_eq!(serialized["auth"]["username_source"], "connection");
}

#[test]
fn failed_or_cancelled_authentication_discards_prepared_secrets_without_a_write() {
    for cancel in [false, true] {
        let (_dir, store) = store();
        let context = Arc::new(RuntimeAuthenticationContext::new(
            Some(store.runtime_connection_snapshot("fixture").unwrap()),
            None,
            None,
            None,
            ConnectionAttempt::default(),
        ));
        let mut form = RuntimeAuthenticationForm::new(request(), context.clone());
        form.remember = true;
        let (response, _, save) = prepare_response(&store, &form, "failed secret".into()).unwrap();
        let id = response.response_id.unwrap();
        context
            .remember_selection(id.clone(), None, None, save)
            .unwrap();
        if cancel {
            context.attempt.cancel();
        }
        let ids = [id];
        assert!(
            context
                .finish(if cancel { Some(&ids) } else { None })
                .is_empty()
        );
        assert!(
            store
                .get_connection("fixture")
                .unwrap()
                .unwrap()
                .auth
                .unwrap()
                .password
                .is_none()
        );
    }
}

#[test]
fn unavailable_selection_keeps_the_form_and_does_not_prepare_a_save() {
    let (_dir, store) = store();
    let mut form = RuntimeAuthenticationForm::new(request(), Arc::default());
    form.saved_password = true;
    form.password_id = Some("deleted-account".into());
    assert!(prepare_response(&store, &form, "".into()).is_err());
    assert_eq!(form.password_id.as_deref(), Some("deleted-account"));
    assert!(!form.remember);
}

#[gpui::test]
fn credential_save_failure_keeps_the_session_and_reports_a_warning(cx: &mut gpui::TestAppContext) {
    use std::time::{Duration, Instant};

    use nyaterm_core::models::ssh_runtime_auth::SshCredentialSaveIntent;
    use nyaterm_store::StoreDomain;

    use crate::features::test_support::app_with_visible_local_session;

    cx.executor().allow_parking();
    let root = TestTempDir::new("nyaterm-auth-save-failure");
    let app = app_with_visible_local_session(cx, root.path(), "connected");
    cx.update_entity(&app, |app, cx| {
        let client = app.store_blocking_client();
        let save = client
            .request_mutation_fn(StoreDomain::Security, |store| {
                let connection: SavedConnection = serde_json::from_value(json!({
                    "id":"fixture", "name":"fixture", "type":"ssh", "host":"localhost",
                    "username":"actual-user", "auth":{"mode":"password"}
                }))?;
                store.save_connection(&connection)?;
                let save = store.prepare_runtime_credential_save(
                    Some(store.runtime_connection_snapshot("fixture")?),
                    "actual-user".into(),
                    SshCredentialSaveIntent::ConnectionPassword,
                    Some("authenticated secret".into()),
                )?;
                store.delete_connection("fixture")?;
                Ok(save)
            })
            .unwrap();
        app.save_authenticated_credentials(
            "successful-request".into(),
            vec![save],
            ConnectionAttempt::default(),
            cx,
        );
    });
    let warning = rust_i18n::t!("sshAuth.saveFailed").to_string();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if cx.read(|cx| app.read(cx).shell.status() == warning) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "credential save did not complete"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    cx.read(|cx| {
        let app = app.read(cx);
        assert!(!app.session.metadata("connected").unwrap().disconnected);
        assert_eq!(app.visible_terminal_session_ids(), ["connected"]);
    });
}
