use rust_i18n::t;

use gpui::{Context, FontWeight, IntoElement, SharedString, div, prelude::*, px, rgb};
use nyaterm_core::{CloudConflictKind, truncate_preview};

use crate::features::NyaTermApp;
use crate::features::formatting::{
    SyncHistoryTimeFormatter, cloud_sync_status_dot_color, configured_cloud_sync_provider,
    format_cloud_provider,
};
use crate::features::sync::SyncHistoryFilter;
use crate::features::view_widgets::{cloud_sync_history_row, dialog_action_button};
use crate::widgets::small_button;
use nyaterm_ui::{NyaButton, NyaButtonVariant, NyaScrollable};

impl NyaTermApp {
    pub(in crate::features) fn sync_backup_history_panel(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let palette = self.theme_palette();
        let provider = configured_cloud_sync_provider(self.cloud_sync.settings());
        let provider_label = format_cloud_provider(&provider);
        let enabled = self.cloud_sync.settings().enabled;
        let status_message = self.cloud_sync.status().to_string();
        let state = self.cloud_sync.activity_status();
        let state_label = t!(match state {
            "disabled" => "settings.syncState.disabled",
            "conflict" => "settings.syncHistory.needsAttention",
            "running" => "settings.syncState.running",
            _ => "settings.syncState.idle",
        });
        let formatter = SyncHistoryTimeFormatter::new();
        let last_success = self
            .cloud_sync
            .history()
            .iter()
            .filter(|entry| {
                entry.kind == "sync"
                    && entry.status == "success"
                    && entry.provider.as_deref() == Some(provider.as_str())
            })
            .map(|entry| entry.timestamp_ms)
            .max()
            .map(|timestamp| {
                format!(
                    "{} {}",
                    formatter.day(timestamp),
                    formatter.clock(timestamp)
                )
            });
        let history = self.cloud_sync.history().to_vec();
        let expanded = self.cloud_sync.history_expanded().clone();
        let conflict = self.cloud_sync.conflict().cloned();
        let branch_view = conflict.as_ref().map(|conflict| {
            let options =
                crate::features::view_widgets::cloud_sync_conflict::branch_options(conflict);
            let select = self.select_entity(
                "cloud-conflict-branch",
                options,
                conflict.selected_revision.clone(),
                self.cloud_sync.job_running(),
                cx,
            );
            crate::features::view_widgets::cloud_sync_conflict::branch_details(
                conflict,
                div()
                    .w_full()
                    .h(px(nyaterm_ui::NYA_FORM_CONTROL_HEIGHT_PX))
                    .child(nyaterm_ui::NyaSelect::new(&select)),
            )
        });
        let sync_action_enabled = enabled && !self.cloud_sync.job_running();

        let filter = self.cloud_sync.history_filter();
        let limit = self.cloud_sync.history_visible_limit();
        let mut history = history;
        history.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms));
        let matching_count = history.iter().filter(|entry| filter.matches(entry)).count();
        let mut filters = div().px_2().py_2().flex_none().flex().flex_wrap().gap_1();
        for (value, label) in [
            (SyncHistoryFilter::All, t!("settings.historyAll")),
            (
                SyncHistoryFilter::Success,
                t!("settings.historyOverviewSuccess"),
            ),
            (
                SyncHistoryFilter::Failed,
                t!("settings.historyOverviewFailed"),
            ),
            (
                SyncHistoryFilter::Conflict,
                t!("settings.historyOverviewConflict"),
            ),
        ] {
            filters = filters.child(
                NyaButton::new(format!("sync-history-filter-{value:?}"), label)
                    .variant(NyaButtonVariant::Ghost)
                    .small()
                    .compact()
                    .selected(filter == value)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.cloud_sync.filter_history(value);
                        cx.notify();
                    })),
            );
        }
        let mut rows = div().flex().flex_col();
        if matching_count == 0 {
            rows = rows.child(
                div()
                    .py_6()
                    .px_3()
                    .text_center()
                    .text_size(px(11.))
                    .text_color(rgb(palette.text_dimmed))
                    .child(if history.is_empty() {
                        t!("settings.historyNoEntries")
                    } else {
                        t!("settings.historyNoResultsTitle")
                    }),
            );
        } else {
            let mut previous_day = None;
            for entry in history
                .into_iter()
                .filter(|entry| filter.matches(entry))
                .take(limit)
            {
                let day = formatter.day(entry.timestamp_ms);
                if previous_day.as_ref() != Some(&day) {
                    rows = rows.child(
                        div()
                            .px_3()
                            .pt_3()
                            .pb_1()
                            .text_size(px(10.))
                            .font_weight(FontWeight(600.))
                            .text_color(rgb(palette.text_muted))
                            .child(day.clone()),
                    );
                    previous_day = Some(day);
                }
                let entry_id = entry.id.clone();
                let is_open = expanded.contains(&entry_id);
                // Copy all original fields, including full revisions and messages.
                let diagnostics = serde_json::to_string_pretty(&entry).unwrap_or_default();
                rows = rows.child(cloud_sync_history_row(
                    palette,
                    entry,
                    &formatter,
                    is_open,
                    cx.listener(move |this, _, _, cx| {
                        this.toggle_cloud_sync_history_details(&entry_id, cx);
                    }),
                    cx.listener(move |this, _, _, cx| {
                        this.copy_text_with_feedback("copy-sync-history", diagnostics.clone(), cx);
                        cx.notify();
                    }),
                ));
            }
            if matching_count > limit {
                rows = rows.child(
                    div().p_2().child(
                        NyaButton::new(
                            "sync-history-show-more",
                            t!("settings.syncHistory.showMore"),
                        )
                        .variant(NyaButtonVariant::Ghost)
                        .small()
                        .full_width()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cloud_sync.show_more_history();
                            cx.notify();
                        })),
                    ),
                );
            }
        }

        let indicator = if state == "running" {
            nyaterm_ui::chat::running_indicator("cloud-sync-running", rgb(palette.link))
                .into_any_element()
        } else {
            div()
                .size(px(8.))
                .rounded_full()
                .flex_none()
                .bg(cloud_sync_status_dot_color(palette, state))
                .into_any_element()
        };
        let heading = div()
            .flex()
            .items_center()
            .gap_2()
            .child(indicator)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .font_weight(FontWeight(600.))
                    .text_color(rgb(palette.text))
                    .child(state_label),
            )
            .child(sync_history_action_button(
                palette,
                "sync-history-push-now",
                "icons/fe/upload.svg",
                t!("settings.syncPushNow"),
                sync_action_enabled,
                cx.listener(|this, _, window, cx| {
                    if this.cloud_sync.settings().enabled && !this.cloud_sync.job_running() {
                        this.prompt_provider_cloud_sync_push(window, cx);
                    }
                }),
            ))
            .child(sync_history_action_button(
                palette,
                "sync-history-pull-now",
                "icons/fe/download.svg",
                t!("settings.syncPullNow"),
                sync_action_enabled,
                cx.listener(|this, _, window, cx| {
                    if this.cloud_sync.settings().enabled && !this.cloud_sync.job_running() {
                        this.prompt_provider_cloud_sync_pull(window, cx);
                    }
                }),
            ));
        let last_success =
            last_success.unwrap_or_else(|| t!("settings.syncHistory.noSuccess").to_string());
        let header = div()
            .flex_none()
            .px_3()
            .py_3()
            .border_b_1()
            .border_color(rgb(palette.border))
            .flex()
            .flex_col()
            .gap_2()
            .child(heading)
            .child(
                div()
                    .pl_4()
                    .text_size(px(11.))
                    .text_color(rgb(palette.text_muted))
                    .child(provider_label),
            )
            .child(
                div()
                    .pl_4()
                    .text_size(px(11.))
                    .text_color(rgb(palette.text_muted))
                    .child(t!("settings.syncHistory.lastSuccess", value = last_success)),
            )
            .when(
                !status_message.trim().is_empty() && conflict.is_none(),
                |this| {
                    this.child(
                        div()
                            .pl_4()
                            .text_size(px(11.))
                            .line_height(px(17.))
                            .text_color(rgb(palette.text_muted))
                            .child(t!(
                                "settings.syncHistory.operationNotice",
                                value = truncate_preview(&status_message, 140)
                            )),
                    )
                },
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(self.shell_transparent_color(palette.surface))
            .child(header)
            .when_some(conflict, |this, conflict| {
                let can_resolve = conflict.preview.local_content_hash.is_some();
                let can_pull = can_resolve && conflict.selected_revision.is_some();
                let preview = conflict.preview;
                let remote_inconsistent = preview.kind == CloudConflictKind::RemoteInconsistent;
                let overwrite_blocked = preview.overwrite_blocked;
                let recovery_revision = preview.recovery_revision.clone();
                let has_recovery_candidate = recovery_revision.is_some();
                this.child(
                    div()
                        .flex_none()
                        .m_2()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(palette.warning))
                        .bg(rgb(palette.input))
                        .when_some(branch_view, |card, view| card.child(div().p_3().child(view)))
                        .child(
                            div()
                                .px_3()
                                .py_2()
                                .border_b_1()
                                .border_color(rgb(palette.warning))
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .font_weight(FontWeight(700.))
                                        .text_color(rgb(palette.warning))
                                        .child(if remote_inconsistent {
                                            t!("settings.syncRemoteIncompleteTitle")
                                        } else {
                                            t!("settings.syncConflictTitle")
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .px_3()
                                .py_2()
                                .text_size(px(11.))
                                .text_color(rgb(palette.text))
                                .child(if remote_inconsistent {
                                    t!("settings.syncHistory.incompleteHint")
                                } else {
                                    t!("settings.syncHistory.conflictExplanation")
                                }),
                        )
                        .child(
                            div()
                                .px_3()
                                .pb_2()
                                .grid()
                                .grid_cols(1)
                                .gap_2()
                                .child(
                                    div()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(rgb(palette.border))
                                        .bg(rgb(palette.input))
                                        .px_2()
                                        .py_1()
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(rgb(palette.text_muted))
                                                .child(t!("settings.providerLabel")),
                                        )
                                        .child(
                                            div()
                                                .mt_0()
                                                .font_family(
                                                    crate::features::shell::gpui_code_font_family(),
                                                )
                                                .text_size(px(11.))
                                                .text_color(rgb(palette.text))
                                                .child(format_cloud_provider(&preview.provider)),
                                        ),
                                )
                                .when_some(recovery_revision, |this, revision| {
                                    this.child(
                                        div()
                                            .rounded_md()
                                            .border_1()
                                            .border_color(rgb(palette.border))
                                            .bg(rgb(palette.input))
                                            .px_2()
                                            .py_1()
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(rgb(palette.text_muted))
                                                    .child(
                                                        t!("settings.currentRemoteSnapshot"),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .font_family(
                                                        crate::features::shell::gpui_code_font_family(),
                                                    )
                                                    .text_size(px(11.))
                                                    .text_color(rgb(palette.text))
                                                    .child(truncate_preview(&revision, 16)),
                                            ),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .px_3()
                                .pb_3()
                                .flex()
                                .gap_2()
                                .child(if remote_inconsistent && has_recovery_candidate && !can_pull {
                                    small_button(
                                        palette,
                                        "sync-panel-recover-current",
                                        t!("settings.useCurrentRemoteSnapshot"),
                                        cx.listener({
                                            let provider_action = conflict.provider_action;
                                            move |this, _, window, cx| {
                                                this.prompt_cloud_sync_recover_current(
                                                    provider_action,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }),
                                    )
                                    .into_any_element()
                                } else if remote_inconsistent && !can_pull {
                                    // Nothing on the remote can be recovered:
                                    // pulling would download the same unreadable
                                    // payload, so only the explicit overwrite is
                                    // offered.
                                    div().into_any_element()
                                } else if can_pull {
                                    small_button(
                                        palette,
                                        "sync-panel-force-pull",
                                        t!("settings.downloadRemoteVersion"),
                                        cx.listener({
                                            let provider_action = conflict.provider_action;
                                            move |this, _, window, cx| {
                                                this.prompt_cloud_sync_force_pull(
                                                    provider_action,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }),
                                    )
                                    .into_any_element()
                                } else { div().text_xs().child(t!("settings.syncChooseBranch")).into_any_element() })
                                .when(!overwrite_blocked && can_resolve, |this| {
                                    this.child(dialog_action_button(
                                        palette,
                                        "sync-panel-force-push",
                                        t!("settings.uploadLocalVersion"),
                                        false,
                                        cx.listener({
                                            let provider_action = conflict.provider_action;
                                            move |this, _, window, cx| {
                                                this.prompt_cloud_sync_force_push(
                                                    provider_action,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }),
                                    ))
                                })
                                .when(overwrite_blocked, |this| {
                                    this.child(
                                        div()
                                            .text_size(px(11.))
                                            .line_height(px(16.))
                                            .text_color(rgb(palette.warning))
                                            .child(t!("settings.overwriteBlockedNotice")),
                                    )
                                }),
                        ),
                )
            })
            .child(div().px_3().pt_3().text_size(px(12.)).font_weight(FontWeight(600.))
                .text_color(rgb(palette.text)).child(t!("settings.syncHistory.title")))
            .child(filters)
            .child(
                div()
                    .id(SharedString::from("sync-backup-history-list"))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(rows),
            )
    }
}

fn sync_history_action_button(
    palette: crate::theme::ThemePalette,
    id: impl Into<String>,
    icon_path: &'static str,
    tooltip: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let id: SharedString = id.into().into();
    let selector = id.clone();
    div()
        .id(SharedString::from(format!("{id}-wrapper")))
        .debug_selector(move || selector.to_string())
        .size(px(24.))
        .flex_none()
        .child(
            NyaButton::new(id, "")
                .icon(icon_path)
                .text_color(rgb(palette.text_muted))
                .tooltip(tooltip)
                .variant(NyaButtonVariant::Ghost)
                .height(px(24.))
                .small()
                .compact()
                .disabled(!enabled)
                .on_click(on_click),
        )
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use gpui::{
        AppContext as _, Context, Entity, IntoElement, Modifiers, ParentElement as _, Render,
        Styled as _, TestAppContext, VisualTestContext, Window, div, px,
    };
    use nyaterm_core::cloud_sync::{CloudSyncSettings, SYNC_CURRENT_FILE};
    use nyaterm_core::{AppRuntime, RuntimeMode};
    use nyaterm_store::ConnectionStore;
    use nyaterm_ui::{NyaDialogWindowExt, nya_root};

    use crate::entities::{OverlayStore, StartupRestoreStore, UiStoreHandles};
    use crate::features::NyaTermApp;
    use crate::models::SnapshotPasswordPromptKind;
    use crate::test_support::{TestConfigDir, spawn_webdav_service_unavailable_server};

    struct SidebarHost {
        app: Entity<NyaTermApp>,
    }

    impl Render for SidebarHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(320.))
                .h(px(720.))
                .child(self.app.update(cx, |app, cx| {
                    app.sync_backup_history_panel(cx).into_any_element()
                }))
        }
    }

    fn test_app(cx: &mut TestAppContext, root: &Path) -> Entity<NyaTermApp> {
        let store = ConnectionStore::open(root.join("config")).expect("test store");
        store
            .save_master_password(Some("synthetic-password"))
            .expect("test master password");
        store
            .save_cloud_sync_settings(CloudSyncSettings {
                enabled: true,
                provider: "local_directory".to_string(),
                ..CloudSyncSettings::default()
            })
            .expect("test sync settings");
        drop(store);
        let runtime = AppRuntime::from_parts_for_test(
            RuntimeMode::Portable,
            root.to_path_buf(),
            root.join("config"),
            root.join("logs"),
            root.join("cache"),
            None,
        );
        let stores = UiStoreHandles {
            startup_restore: cx.new(|_| StartupRestoreStore::default()),
            overlays: cx.new(|_| OverlayStore::default()),
        };
        let app = cx.new(|cx| NyaTermApp::new(runtime, stores, cx));
        cx.update_entity(&app, |app, cx| app.sync_component_theme(cx));
        app
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            _ = window.draw(cx);
        });
        cx.run_until_parked();
    }

    fn click_action(cx: &mut VisualTestContext, id: &'static str) {
        draw(cx);
        let bounds = cx.debug_bounds(id).expect("sync action should render");
        cx.simulate_click(bounds.center(), Modifiers::default());
        draw(cx);
    }

    #[test]
    fn sync_sidebar_actions_open_visible_password_dialogs_and_cancel_cleanly() {
        let test_dir = TestConfigDir::new("nyaterm-sync-sidebar");
        let mut cx = TestAppContext::single();
        let _test_app_cleanup = crate::test_support::TestAppCleanup::new(&cx);
        let app = test_app(&mut cx, test_dir.path());
        let host_app = app.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let host = cx.new(|_| SidebarHost { app: host_app });
            nya_root(host, window, cx)
        });
        let cx: &mut VisualTestContext = cx;
        for (id, kind) in [
            (
                "sync-history-push-now",
                SnapshotPasswordPromptKind::CloudProviderPush,
            ),
            (
                "sync-history-pull-now",
                SnapshotPasswordPromptKind::CloudProviderPull,
            ),
        ] {
            click_action(cx, id);
            assert!(
                cx.debug_bounds("snapshot-password-dialog-content")
                    .is_some()
            );
            assert!(cx.debug_bounds("nya-dialog-action-button").is_some());
            cx.update(|window, cx| {
                assert!(window.has_active_nya_dialog(cx));
                let state = app.read(cx);
                assert_eq!(
                    state
                        .settings
                        .snapshot_password_prompt()
                        .expect("prompt")
                        .kind,
                    kind
                );
                assert!(!state.cloud_sync.job_running());
            });
            cx.simulate_keystrokes("enter");
            draw(cx);
            cx.update(|window, cx| {
                assert!(
                    window.has_active_nya_dialog(cx),
                    "empty passwords must keep the dialog open"
                );
                assert!(app.read(cx).settings.snapshot_password_prompt_active());
                assert!(!app.read(cx).cloud_sync.job_running());
            });
            cx.simulate_keystrokes("escape");
            draw(cx);
            cx.update(|window, cx| {
                assert!(!window.has_active_nya_dialog(cx));
                assert!(!app.read(cx).settings.snapshot_password_prompt_active());
                assert!(app.read(cx).cloud_sync.history().is_empty());
            });
        }
    }

    #[test]
    fn local_backup_restore_requires_confirmation_before_password() {
        let test_dir = TestConfigDir::new("nyaterm-backup-restore-confirm");
        let mut cx = TestAppContext::single();
        let _test_app_cleanup = crate::test_support::TestAppCleanup::new(&cx);
        let app = test_app(&mut cx, test_dir.path());
        let host_app = app.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let host = cx.new(|_| SidebarHost { app: host_app });
            nya_root(host, window, cx)
        });
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.prompt_encrypted_portable_snapshot_import(window, cx)
            });
        });
        draw(cx);
        assert!(cx.debug_bounds("nya-dialog-action-button").is_some());
        assert!(
            cx.debug_bounds("snapshot-password-dialog-content")
                .is_none()
        );
        cx.simulate_keystrokes("escape");
        draw(cx);
        cx.update(|window, cx| {
            assert!(!window.has_active_nya_dialog(cx));
            assert!(!app.read(cx).settings.snapshot_password_prompt_active());
        });

        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.prompt_encrypted_portable_snapshot_import(window, cx)
            });
        });
        draw(cx);
        cx.simulate_keystrokes("enter");
        draw(cx);
        assert!(
            cx.debug_bounds("snapshot-password-dialog-content")
                .is_some()
        );
    }

    #[test]
    fn sync_sidebar_webdav_submission_reports_http_failure() {
        let (endpoint, server) = spawn_webdav_service_unavailable_server();
        let test_dir = TestConfigDir::new("nyaterm-sync-sidebar-webdav");
        let mut cx = TestAppContext::single();
        let _test_app_cleanup = crate::test_support::TestAppCleanup::new(&cx);
        let app = test_app(&mut cx, test_dir.path());
        cx.update_entity(&app, |app, _| {
            let mut settings = app.cloud_sync.settings().clone();
            settings.provider = "webdav".to_string();
            settings.webdav.endpoint = endpoint;
            let persisted_settings = settings.clone();
            app.store_blocking_client()
                .request_fn(nyaterm_store::StoreDomain::CloudSync, move |store| {
                    store.save_cloud_sync_settings(persisted_settings)
                })
                .expect("persist the provider configuration used by the test");
            app.cloud_sync
                .replace_settings(settings, Default::default());
        });
        let host_app = app.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let host = cx.new(|_| SidebarHost { app: host_app });
            nya_root(host, window, cx)
        });
        let cx: &mut VisualTestContext = cx;
        click_action(cx, "sync-history-push-now");
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.reset_text_input("snapshot-password.value", "synthetic-password", cx);
                app.apply_snapshot_password_input("synthetic-password".to_string(), cx);
            })
        });
        draw(cx);
        cx.simulate_keystrokes("enter");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            draw(cx);
            let recorded = cx.update(|_, cx| {
                let state = app.read(cx);
                state
                    .cloud_sync
                    .history()
                    .iter()
                    .find(|entry| entry.trigger == "manual_provider_push")
                    .map(|entry| {
                        assert_ne!(entry.status, "success");
                        assert!(state.cloud_sync.status().contains("503"));
                        assert!(!state.cloud_sync.job_running());
                    })
                    .is_some()
            });
            if recorded {
                break;
            }
            assert!(Instant::now() < deadline, "WebDAV failure was not recorded");
            std::thread::sleep(Duration::from_millis(5));
        }
        server.join().expect("mock WebDAV server");
    }

    #[test]
    fn sync_sidebar_password_submission_runs_push_and_pull_and_records_history() {
        let test_dir = TestConfigDir::new("nyaterm-sync-sidebar-roundtrip");
        let mut cx = TestAppContext::single();
        let _test_app_cleanup = crate::test_support::TestAppCleanup::new(&cx);
        let app = test_app(&mut cx, test_dir.path());
        let host_app = app.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let host = cx.new(|_| SidebarHost { app: host_app });
            nya_root(host, window, cx)
        });
        let cx: &mut VisualTestContext = cx;
        for (id, trigger, keyboard_submit) in [
            ("sync-history-push-now", "manual_provider_push", true),
            ("sync-history-pull-now", "manual_provider_pull", false),
        ] {
            click_action(cx, id);
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.reset_text_input("snapshot-password.value", "synthetic-password", cx);
                    app.apply_snapshot_password_input("synthetic-password".to_string(), cx);
                })
            });
            draw(cx);
            if keyboard_submit {
                cx.simulate_keystrokes("enter");
            } else {
                let submit = cx
                    .debug_bounds("nya-dialog-action-button")
                    .expect("submit button");
                cx.simulate_click(submit.center(), Modifiers::default());
            }
            draw(cx);
            cx.update(|window, cx| {
                assert!(
                    !window.has_active_nya_dialog(cx),
                    "submission must close the password dialog"
                );
                assert!(!app.read(cx).settings.snapshot_password_prompt_active());
            });
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                draw(cx);
                let recorded = cx.update(|_, cx| {
                    app.read(cx)
                        .cloud_sync
                        .history()
                        .iter()
                        .find(|entry| entry.trigger == trigger)
                        .map(|entry| entry.status.clone())
                });
                if let Some(status) = recorded {
                    assert_eq!(status, "success");
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "sync submission did not record an operation"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                test_dir
                    .path()
                    .join("config/cloud-sync-local/nyaterm")
                    .join(SYNC_CURRENT_FILE)
                    .is_file(),
                "push must produce an encrypted remote snapshot"
            );
        }
    }
    #[test]
    fn cloud_sync_sidebar_branch_selection_uses_keyboard_and_restores_trigger_focus() {
        use nyaterm_core::cloud_sync::conflict::CloudConflictBranch;
        use nyaterm_core::{CloudConflictPreview, CloudSyncError};
        let test_dir = TestConfigDir::new("nyaterm-cloud-sync-branch-keyboard");
        let mut cx = TestAppContext::single();
        let _cleanup = crate::test_support::TestAppCleanup::new(&cx);
        let app = test_app(&mut cx, test_dir.path());
        let mut preview: CloudConflictPreview = serde_json::from_value(serde_json::json!({
            "detected_at_ms": 1, "provider": "test", "local_payload_hash": "local",
            "remote_payload_hash": "remote", "remote_revision": "r0", "remote_created_at_ms": 1,
            "remote_device_id": "device", "message": "conflict", "local_content_hash": "local-content"
        })).unwrap();
        preview.branches = (0..16)
            .map(|n| CloudConflictBranch {
                revision: format!("r{n}"),
                device_id: format!("device-{n}"),
                device_name: None,
                created_at_ms: 1,
                readable: n != 15,
                differences: Vec::new(),
            })
            .collect();
        preview.observed_revisions = preview
            .branches
            .iter()
            .map(|b| b.revision.clone())
            .collect();
        cx.update_entity(&app, |app, cx| {
            app.apply_auto_cloud_sync_result(&Err(CloudSyncError::Conflict(Box::new(preview))), cx);
        });
        let host_app = app.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let host = cx.new(|_| SidebarHost { app: host_app });
            nya_root(host, window, cx)
        });
        let cx: &mut VisualTestContext = cx;
        draw(cx);
        let viewport = cx
            .debug_bounds("cloud-sync-branch-scroll")
            .expect("branch list is rendered");
        assert!(viewport.size.height <= px(260.));
        cx.update(|window, cx| {
            assert!(
                app.read(cx)
                    .cloud_sync
                    .conflict()
                    .unwrap()
                    .selected_revision
                    .is_none()
            );
            app.update(cx, |app, cx| {
                assert!(app.focus_select("cloud-conflict-branch", window, cx));
            });
        });
        cx.simulate_keystrokes("enter");
        draw(cx);
        cx.simulate_keystrokes("down down down down");
        draw(cx);
        cx.simulate_keystrokes("enter");
        draw(cx);
        cx.update(|window, cx| {
            let app = app.read(cx);
            let selected = app
                .cloud_sync
                .conflict()
                .unwrap()
                .selected_revision
                .as_deref()
                .expect("keyboard selection commits");
            assert_ne!(selected, "r0");
            assert_ne!(selected, "r15", "unreadable branches cannot be chosen");
            assert!(app.select_is_focused("cloud-conflict-branch", window, cx));
        });
    }
}
