use gpui::{AppContext as _, Entity, KeyDownEvent, Keystroke, TestAppContext};
use nyaterm_core::test_support::TestTempDir;
use nyaterm_transport::RemoteFilePath;

use crate::features::NyaTermApp;
use crate::features::file_manager::pane::{PaneEndpoint, PaneId};
use crate::features::file_manager::workspace::{FileManagerSeed, FileManagerWorkspace};
use crate::features::test_support::{app_with_visible_local_session, session_focus_test_window};

fn workspace(
    cx: &mut TestAppContext,
    app: &Entity<NyaTermApp>,
    path: &str,
) -> Entity<FileManagerWorkspace> {
    let workspace = cx.new(|cx| {
        FileManagerWorkspace::new(
            app.clone(),
            "file-manager-reentrancy".into(),
            FileManagerSeed {
                left: PaneEndpoint::Local,
                left_path: Some(RemoteFilePath::new(path)),
                right: PaneEndpoint::Local,
                right_path: Some(RemoteFilePath::new(path)),
            },
            cx,
        )
    });
    app.update(cx, |app, _| {
        app.file_manager
            .insert("file-manager-reentrancy".into(), workspace.clone());
    });
    workspace
}

#[gpui::test]
fn navigation_releases_workspace_before_app_updates_the_same_pane(cx: &mut TestAppContext) {
    let root = TestTempDir::new("nyaterm-file-manager-navigation-reentrancy");
    let app = app_with_visible_local_session(cx, root.path(), "file-manager-test");
    let path = root.path().to_string_lossy().into_owned();
    let workspace = workspace(cx, &app, &path);
    let cx = session_focus_test_window(cx, &app);

    for pane_id in [PaneId::Left, PaneId::Right] {
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                let id = workspace.id.clone();
                let path = RemoteFilePath::new(path.clone());
                workspace.dispatch_app(window, cx, move |app, _, cx| {
                    app.navigate_file_manager_pane(id, pane_id, path, cx);
                });
                assert_eq!(workspace.pane(pane_id).generation, 0);
            });
        });
        cx.read(|cx| {
            let pane = workspace.read(cx).pane(pane_id);
            assert_eq!(pane.generation, 1);
            assert_eq!(pane.path.display_path, path);
        });
    }
}

#[gpui::test]
fn copy_and_move_shortcuts_can_read_the_originating_workspace(cx: &mut TestAppContext) {
    let root = TestTempDir::new("nyaterm-file-manager-shortcut-reentrancy");
    let app = app_with_visible_local_session(cx, root.path(), "file-manager-test");
    let workspace = workspace(cx, &app, &root.path().to_string_lossy());
    let cx = session_focus_test_window(cx, &app);

    // Both commands consult the workspace even when no files are selected.
    for key in ["f5", "f6"] {
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.handle_key(
                    &KeyDownEvent {
                        keystroke: Keystroke::parse(key).unwrap(),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    window,
                    cx,
                );
            });
        });
        cx.read(|cx| assert!(app.read(cx).transfer.transfer_jobs().is_empty()));
    }
}

#[gpui::test]
fn layout_save_reads_the_workspace_after_its_listener_finishes(cx: &mut TestAppContext) {
    let root = TestTempDir::new("nyaterm-file-manager-layout-reentrancy");
    let app = app_with_visible_local_session(cx, root.path(), "file-manager-test");
    let workspace = workspace(cx, &app, &root.path().to_string_lossy());
    let cx = session_focus_test_window(cx, &app);

    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.split_percent = 65;
            workspace.set_active(PaneId::Right);
            workspace.dispatch_app(window, cx, |app, _, cx| {
                app.persist_file_manager_settings(cx);
            });
        });
    });
    cx.read(|cx| {
        let settings = &app.read(cx).settings.summary().ui_file_manager;
        assert_eq!(settings.split_percent, 65);
        assert_eq!(
            settings.active_pane,
            nyaterm_core::FileManagerPaneSide::Right
        );
    });
}
