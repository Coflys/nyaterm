use gpui::{Context, IntoElement, MouseButton, SharedString, div, prelude::*, px, rgb, svg};
use nyaterm_ui::{NyaButton, NyaContextMenu, NyaMenuItem, NyaTooltip};
use rust_i18n::t;

use crate::features::NyaTermApp;
use crate::features::shell::{SessionTabDragPreview, WorkspaceTabDragPayload};
use crate::models::WorkspaceTabId;

impl NyaTermApp {
    pub(super) fn editor_strip_tab(
        &mut self,
        id: String,
        index: usize,
        number: usize,
        active: Option<&str>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(document) = self.transfer.editor_tab_snapshot(&id) else {
            return div().into_any_element();
        };
        let palette = self.theme_palette();
        let key = WorkspaceTabId::Editor(id).layout_key();
        let selected = active == Some(key.as_str());
        let click_key = key.clone();
        let close_key = key.clone();
        let middle_key = key.clone();
        let drop_key = key.clone();
        let payload = WorkspaceTabDragPayload {
            source_workspace_id: self.workspace_id,
            root_tab_id: key.clone(),
            source_revision: self.workspace_revision(),
            tab_id: WorkspaceTabId::from_layout_key(&key),
            order_index: index,
            source_group_id: self.shell.terminal_group_for_tab(&key),
            display_name: document.name.clone(),
            kind_icon: "icons/file/text.svg",
            preview_background: palette.surface,
            preview_border: palette.border,
            preview_text: palette.text,
            preview_text_muted: palette.text_muted,
            preview_accent: palette.link,
        };
        let app = cx.weak_entity();
        let title = format!(
            "{}\n{}",
            document.remote_path,
            document
                .session_id
                .as_deref()
                .and_then(|id| self.session.display_name(id))
                .unwrap_or_default()
        );
        let tab = div()
            .id(SharedString::from(format!("editor-tab-{}", document.id)))
            .h(px(36.))
            .min_w(px(118.))
            .max_w(px(236.))
            .flex_none()
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .border_r_1()
            .border_color(rgb(palette.border))
            .bg(rgb(if selected {
                palette.bg
            } else {
                palette.surface
            }))
            .text_color(rgb(if selected {
                palette.text
            } else {
                palette.text_muted
            }))
            .cursor_pointer()
            .hover(move |this| this.bg(rgb(palette.hover)))
            .tooltip(move |window, cx| NyaTooltip::new(title.clone()).build(window, cx))
            .on_click(cx.listener(move |app, _, _, cx| app.activate_workspace_tab(&click_key, cx)))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |app, _, _, cx| {
                    cx.stop_propagation();
                    app.request_workspace_tabs_close(vec![middle_key.clone()], None, cx);
                }),
            )
            .on_drag(payload, move |payload, position, window, cx| {
                let _ = app.update(cx, |app, cx| {
                    app.begin_session_tab_drag(payload, position, window, cx)
                });
                cx.new(|_| SessionTabDragPreview::new(payload.clone(), position))
            })
            .on_drop(
                cx.listener(move |app, payload: &WorkspaceTabDragPayload, _, cx| {
                    app.accept_session_tab_drop(payload, cx);
                    let group = app.shell.terminal_group_for_tab(&drop_key);
                    if app.commit_session_tab_preview(payload, &group, cx) {
                        return;
                    }
                    if payload.source_workspace_id == app.workspace_id {
                        app.reorder_session_relative(
                            payload.tab_id.layout_key(),
                            drop_key.clone(),
                            false,
                            cx,
                        );
                    }
                }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.text_dimmed))
                    .child(number.to_string()),
            )
            .child(
                svg()
                    .path("icons/file/text.svg")
                    .size(px(14.))
                    .text_color(rgb(palette.link)),
            )
            .when(document.dirty, |this| {
                this.child(div().size(px(6.)).rounded_full().bg(rgb(palette.link)))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_xs()
                    .child(document.name),
            )
            .child(
                NyaButton::new("editor-tab-close", "×")
                    .small()
                    .on_click(cx.listener(move |app, _, _, cx| {
                        cx.stop_propagation();
                        app.request_workspace_tabs_close(vec![close_key.clone()], None, cx);
                    })),
            );
        let app = cx.weak_entity();
        NyaContextMenu::new_dynamic(tab, move |_, cx| {
            let Some(app) = app.upgrade() else {
                return Vec::new();
            };
            let group = app
                .read(cx)
                .shell
                .terminal_group_for_tab(&key)
                .and_then(|group| app.read(cx).shell.terminal_group_tabs(&group))
                .map(|(tabs, _)| tabs)
                .unwrap_or_default();
            let index = group
                .iter()
                .position(|id| id == &key)
                .unwrap_or(group.len());
            let all = app.read(cx).workspace_tab_ids();
            [
                (t!("common.close").to_string(), vec![key.clone()]),
                (
                    t!("workspaceTabs.closeOthers").to_string(),
                    group.iter().filter(|id| *id != &key).cloned().collect(),
                ),
                (
                    t!("workspaceTabs.closeRight").to_string(),
                    group.iter().skip(index + 1).cloned().collect(),
                ),
                (t!("workspaceTabs.closeGroup").to_string(), group),
                (t!("workspaceTabs.closeAll").to_string(), all),
            ]
            .into_iter()
            .map(|(label, targets)| {
                let app = app.downgrade();
                NyaMenuItem::action(label)
                    .disabled(targets.is_empty())
                    .on_click(move |_, _, cx| {
                        let targets = targets.clone();
                        let app = app.clone();
                        // Open close confirmations only after the popup has dismissed.
                        cx.defer(move |cx| {
                            let _ = app.update(cx, |app, cx| {
                                app.request_workspace_tabs_close(targets, None, cx)
                            });
                        });
                    })
            })
            .collect()
        })
        .into_any_element()
    }
}
