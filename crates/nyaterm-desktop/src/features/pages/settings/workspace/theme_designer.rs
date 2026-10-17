use gpui::{
    AnyElement, AppContext as _, BoxShadow, Context, Entity, IntoElement, ParentElement as _, Rgba,
    Styled as _, Subscription, Window, div, prelude::*, px, rgb, rgba,
};
use nyaterm_core::themes::management::ThemeApplyTarget;
use nyaterm_core::themes::{COLOR_FIELDS, builtins, is_builtin, parse_color};
use nyaterm_ui::{
    NyaButton, NyaButtonVariant, NyaColorPicker, NyaColorPickerEvent, NyaColorPickerState,
    NyaDropdownMenu, NyaMenuItem, NyaScrollable as _,
};
use rust_i18n::t;

use crate::features::pages::settings::panel::SettingsPanel;
use crate::features::settings::theme_designer::{ThemeDesignerState, ThemeIntent};

#[derive(Default)]
pub(in crate::features::pages::settings) struct ThemePickerControls {
    fields: std::collections::HashMap<String, (Entity<NyaColorPickerState>, Subscription, String)>,
}

impl SettingsPanel {
    /// Called from a deferred snapshot update, never from rendering.
    pub(in crate::features::pages::settings) fn sync_theme_picker_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = &self.settings.theme_designer;
        let Some(theme) = state
            .draft
            .as_ref()
            .filter(|_| state.open && state.editing && !state.submitting)
        else {
            self.theme_picker_controls.fields.clear();
            return;
        };
        let mut values = vec![("swatch".to_string(), theme.swatch.clone(), false)];
        values.extend(
            COLOR_FIELDS
                .iter()
                .map(|f| (f.path().to_string(), theme.color(*f).to_string(), f.alpha())),
        );
        let mut changed = false;
        for (path, value, alpha) in values {
            let Some(bytes) = parse_color(&value) else {
                continue;
            };
            let color = rgba(u32::from_be_bytes(bytes));
            if let Some((picker, _, previous)) = self.theme_picker_controls.fields.get_mut(&path) {
                if *previous != value {
                    picker.update(cx, |picker, cx| picker.set_value(color, window, cx));
                    *previous = value;
                    changed = true;
                }
                continue;
            }
            let picker = cx.new(|cx| NyaColorPickerState::new(window, cx).default_value(color));
            let field = path.clone();
            let subscription = cx.subscribe(&picker, move |panel, _, event, cx| {
                let NyaColorPickerEvent::Change(Some(color)) = event else {
                    return;
                };
                let c = Rgba::from(*color);
                let channel = |v: f32| (v * 255.).round().clamp(0., 255.) as u8;
                let value = if alpha {
                    format!(
                        "rgba({}, {}, {}, {:.4})",
                        channel(c.r),
                        channel(c.g),
                        channel(c.b),
                        c.a
                    )
                } else {
                    format!(
                        "#{:02x}{:02x}{:02x}",
                        channel(c.r),
                        channel(c.g),
                        channel(c.b)
                    )
                };
                panel.with_app(cx, |app, cx| {
                    app.edit_theme_designer(&field, value, cx);
                    app.ensure_theme_designer_inputs(cx);
                });
            });
            self.theme_picker_controls
                .fields
                .insert(path, (picker, subscription, value));
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    pub(in crate::features::pages::settings) fn theme_designer_section(
        &mut self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.settings.theme_designer.clone();
        let palette = self.theme_palette();
        if !state.open {
            return NyaButton::new("theme-designer-toggle", t!("themeDesigner.manage"))
                .when(state.restore_focus, |button| button.autofocus())
                .on_click(cx.listener(|panel, _, _, cx| {
                    panel.with_app(cx, |app, cx| {
                        app.theme_designer_action(ThemeIntent::Toggle, cx)
                    });
                }))
                .into_any_element();
        }
        let blocked = state.submitting || state.pending.is_some() || state.close_confirmation;
        let mut section = div().flex().flex_col().size_full().min_h_0().gap_3().p_4();
        let current = state
            .draft
            .as_ref()
            .map(|theme| theme.id.clone())
            .unwrap_or_default();
        let copy_id = current.clone();
        let mut header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                NyaButton::new("theme-manager-back", t!("themeDesigner.returnAppearance"))
                    .icon("icons/menu/chevron-left.svg")
                    .variant(NyaButtonVariant::Ghost)
                    .autofocus()
                    .disabled(blocked)
                    .on_click(cx.listener(|panel, _, _, cx| {
                        panel.with_app(cx, |app, cx| {
                            app.theme_designer_action(ThemeIntent::Toggle, cx)
                        });
                    })),
            )
            .child(div().flex_1().child(t!("themeDesigner.manage")));
        for (id, label, intent) in [
            ("import", "themeDesigner.import", ThemeIntent::Import),
            ("create", "themeDesigner.create", ThemeIntent::Copy(copy_id)),
            ("reload", "themeDesigner.reload", ThemeIntent::Reload),
        ] {
            header = header.child(
                NyaButton::new(format!("theme-manager-{id}"), t!(label))
                    .disabled(blocked)
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.with_app(cx, |app, cx| app.theme_designer_action(intent.clone(), cx));
                    })),
            );
        }
        section = section.child(header).child(
            div()
                .text_sm()
                .text_color(rgb(palette.text_muted))
                .child(t!("themeDesigner.description")),
        );
        if state.pending.is_some() {
            let mut prompt = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .p_3()
                .rounded_md()
                .bg(rgb(palette.section_header))
                .child(t!("themeDesigner.unsaved"));
            for (id, label, choice) in [
                ("save", "themeDesigner.save", Some(true)),
                ("discard", "themeDesigner.discard", Some(false)),
                ("back", "themeDesigner.back", None),
            ] {
                prompt = prompt.child(
                    NyaButton::new(format!("theme-pending-{id}"), t!(label))
                        .disabled(
                            state.submitting || (choice == Some(true) && !state.errors.is_empty()),
                        )
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            panel.with_app(cx, |app, cx| {
                                app.resolve_theme_designer_pending(choice, cx)
                            });
                        })),
                );
            }
            section = section.child(prompt);
        }
        if state.close_confirmation {
            section = section.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(t!("settings.discardChangesDesc"))
                    .child(
                        NyaButton::new(
                            "theme-close-discard-settings",
                            t!("settings.discardChanges"),
                        )
                        .variant(NyaButtonVariant::Danger)
                        .on_click(cx.listener(|panel, _, _, cx| {
                            panel.with_app(cx, |app, cx| app.cancel_settings(cx));
                        })),
                    )
                    .child(
                        NyaButton::new("theme-close-keep-settings", t!("themeDesigner.back"))
                            .on_click(cx.listener(|panel, _, _, cx| {
                                panel.with_app(cx, |app, cx| {
                                    app.settings.theme_designer_mut().close_confirmation = false;
                                    app.request_settings_panel_refresh(cx);
                                });
                            })),
                    ),
            );
        }
        if !state.message.is_empty() {
            section = section.child(div().text_sm().child(state.message.clone()));
        }
        let summary = self.settings.summary();
        let mut list = div().flex().flex_col().gap_2().w_full().min_w_0();
        for (builtin, label) in [
            (true, "themeDesigner.builtins"),
            (false, "themeDesigner.mine"),
        ] {
            list = list.child(div().text_sm().child(t!(label)));
            let themes: Vec<_> = if builtin {
                builtins().to_vec()
            } else {
                summary.custom_themes.available()
            };
            for theme in themes {
                let id = theme.id.clone();
                let swatch = parse_color(&theme.swatch)
                    .map(|color| rgba(u32::from_be_bytes(color)))
                    .unwrap_or(rgb(palette.primary));
                let mut content = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w_0()
                    .child(div().size(px(14.)).flex_none().rounded_sm().bg(swatch))
                    .child(div().flex_1().min_w_0().truncate().child(theme.name));
                if summary.theme == id {
                    content = content.child(div().text_xs().child(t!("themeDesigner.currentUi")));
                }
                if summary.terminal_theme.as_deref() == Some(&id)
                    || (summary.terminal_theme.is_none() && summary.theme == id)
                {
                    content =
                        content.child(div().text_xs().child(t!("themeDesigner.currentTerminal")));
                }
                list = list.child(
                    NyaButton::new(format!("theme-browse-{id}"), "")
                        .content(content)
                        .full_width()
                        .selected(current == id)
                        .disabled(blocked)
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            panel.with_app(cx, |app, cx| {
                                app.theme_designer_action(ThemeIntent::Select(id.clone()), cx)
                            });
                        })),
                );
            }
        }
        for diagnostic in self.settings.theme_diagnostics.iter() {
            list = list.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.warning))
                    .child(diagnostic.clone()),
            );
        }
        let mut editor = div().flex().flex_col().gap_3().w_full().min_w_0();
        if let Some(theme) = state.draft.as_ref() {
            let builtin = is_builtin(&theme.id);
            let panel = cx.entity().downgrade();
            let rename_panel = panel.clone();
            let copy_panel = panel.clone();
            let export_panel = panel.clone();
            let delete_panel = panel;
            let copy_id = theme.id.clone();
            let items = vec![
                NyaMenuItem::action(t!("themeDesigner.rename"))
                    .disabled(builtin)
                    .on_click(move |_, window, cx| {
                        let _ = rename_panel.update(cx, |panel, cx| {
                            panel.with_app(cx, |app, cx| {
                                if !app.settings.theme_designer().editing {
                                    app.theme_designer_action(ThemeIntent::Edit, cx);
                                }
                                app.focus_text_input_if_present("theme.designer.name", window, cx);
                            })
                        });
                    }),
                NyaMenuItem::action(t!("themeDesigner.copy")).on_click(move |_, _, cx| {
                    let _ = copy_panel.update(cx, |panel, cx| {
                        panel.with_app(cx, |app, cx| {
                            app.theme_designer_action(ThemeIntent::Copy(copy_id.clone()), cx);
                        })
                    });
                }),
                NyaMenuItem::action(t!("themeDesigner.export"))
                    .disabled(!state.errors.is_empty())
                    .on_click(move |_, _, cx| {
                        let _ = export_panel.update(cx, |panel, cx| {
                            panel.with_app(cx, |app, cx| app.export_theme_designer(cx))
                        });
                    }),
                NyaMenuItem::action(t!("themeDesigner.delete"))
                    .danger()
                    .disabled(builtin || !summary.custom_themes.contains_id(&theme.id))
                    .on_click(move |_, _, cx| {
                        let _ = delete_panel.update(cx, |panel, cx| {
                            panel.with_app(cx, |app, cx| {
                                app.theme_designer_action(ThemeIntent::Delete, cx)
                            })
                        });
                    }),
            ];
            let edit_intent = if builtin {
                ThemeIntent::Copy(theme.id.clone())
            } else {
                ThemeIntent::Edit
            };
            editor = editor.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(theme.name.clone()))
                    .child(
                        NyaButton::new(
                            "theme-edit",
                            t!(if builtin {
                                "themeDesigner.createCopy"
                            } else {
                                "themeDesigner.edit"
                            }),
                        )
                        .disabled(blocked || state.editing)
                        .on_click(cx.listener(
                            move |panel, _, window, cx| {
                                panel.with_app(cx, |app, cx| {
                                    app.theme_designer_action(edit_intent.clone(), cx);
                                    app.focus_text_input_if_present(
                                        "theme.designer.name",
                                        window,
                                        cx,
                                    );
                                });
                            },
                        )),
                    )
                    .child(
                        NyaDropdownMenu::new("theme-operations")
                            .label(t!("themeDesigner.operations"))
                            .disabled(blocked)
                            .items(items),
                    ),
            );
            if let Some(preview) = state.preview {
                editor = editor.child(theme_preview(preview, &theme.name));
            }
            if !state.errors.is_empty() {
                editor = editor.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.warning))
                        .child(t!("themeDesigner.previewUnchanged")),
                );
            }
            if state.low_contrast {
                editor = editor.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.warning))
                        .child(t!("themeDesigner.contrastWarning")),
                );
            }
            if state.editing && !state.submitting {
                for (field, label) in [
                    ("name", "themeDesigner.name"),
                    ("label", "themeDesigner.label"),
                    ("swatch", "themeDesigner.swatch"),
                ] {
                    editor =
                        editor.child(self.theme_color_input(field, t!(label).to_string(), &state));
                }
                let mut tabs = div().flex().gap_2();
                for (terminal, label) in [
                    (false, "themeDesigner.uiColors"),
                    (true, "themeDesigner.terminalColors"),
                ] {
                    tabs = tabs.child(
                        NyaButton::new(format!("theme-tab-{terminal}"), t!(label))
                            .selected(state.terminal_tab == terminal)
                            .disabled(blocked)
                            .on_click(cx.listener(move |panel, _, _, cx| {
                                panel.with_app(cx, |app, cx| {
                                    app.settings.theme_designer_mut().terminal_tab = terminal;
                                    app.request_settings_panel_refresh(cx);
                                })
                            })),
                    );
                }
                editor = editor.child(tabs);
                let mut grid = div().flex().flex_wrap().gap_3().w_full();
                for field in COLOR_FIELDS
                    .iter()
                    .filter(|field| field.terminal() == state.terminal_tab)
                {
                    grid =
                        grid.child(div().flex_1().min_w(px(190.)).child(self.theme_color_input(
                            field.path(),
                            t!(format!("themeDesigner.colors.{}", field.path())).to_string(),
                            &state,
                        )));
                }
                editor = editor.child(grid);
                editor = editor.child(
                    NyaButton::new("theme-save", t!("themeDesigner.save"))
                        .variant(NyaButtonVariant::Primary)
                        .disabled(blocked || !state.dirty() || !state.errors.is_empty())
                        .on_click(cx.listener(|panel, _, _, cx| {
                            panel.with_app(cx, |app, cx| {
                                app.save_theme_designer(None, cx);
                            })
                        })),
                );
                editor = editor.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.text_muted))
                        .child(t!("themeDesigner.saveActive")),
                );
            }
            let mut actions = div().flex().flex_wrap().gap_2();
            for (id, target, clean, dirty) in [
                (
                    "ui",
                    ThemeApplyTarget::Ui,
                    "themeDesigner.applyUi",
                    "themeDesigner.useUi",
                ),
                (
                    "terminal",
                    ThemeApplyTarget::Terminal,
                    "themeDesigner.applyTerminal",
                    "themeDesigner.useTerminal",
                ),
                (
                    "both",
                    ThemeApplyTarget::BothFollowUi,
                    "themeDesigner.applyBoth",
                    "themeDesigner.saveBoth",
                ),
            ] {
                actions = actions.child(
                    NyaButton::new(
                        format!("theme-apply-{id}"),
                        t!(if state.dirty() { dirty } else { clean }),
                    )
                    .disabled(blocked || !state.errors.is_empty())
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.with_app(cx, |app, cx| {
                            app.save_theme_designer(Some(target), cx);
                        })
                    })),
                );
            }
            editor = editor.child(actions).child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.text_muted))
                    .child(t!("themeDesigner.followExplanation")),
            );
            if state.delete_confirmation {
                editor = editor.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_3()
                        .border_1()
                        .border_color(rgb(palette.danger))
                        .child(t!(
                            "themeDesigner.deleteConfirmation",
                            name = theme.name.clone()
                        ))
                        .child(
                            NyaButton::new("theme-delete-confirm", t!("themeDesigner.delete"))
                                .variant(NyaButtonVariant::Danger)
                                .disabled(blocked)
                                .on_click(cx.listener(|panel, _, _, cx| {
                                    panel.with_app(cx, |app, cx| app.confirm_theme_delete(cx))
                                })),
                        )
                        .child(
                            NyaButton::new("theme-delete-cancel", t!("common.cancel"))
                                .disabled(blocked)
                                .on_click(cx.listener(|panel, _, _, cx| {
                                    panel.with_app(cx, |app, cx| {
                                        app.settings.theme_designer_mut().delete_confirmation =
                                            false;
                                        app.request_settings_panel_refresh(cx);
                                    })
                                })),
                        ),
                );
            }
        }
        let narrow = self.viewport_width < 850.;
        section
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .gap_4()
                    .when(narrow, |body| body.flex_col())
                    .child(
                        div()
                            .min_h_0()
                            .when(narrow, |list| list.h(px(180.)))
                            .when(!narrow, |list| list.w(px(260.)).flex_none().h_full())
                            .child(list.h_full().min_h_0().overflow_y_scrollbar()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .child(editor.h_full().min_h_0().overflow_y_scrollbar()),
                    ),
            )
            .into_any_element()
    }

    fn theme_color_input(
        &self,
        field: &str,
        label: String,
        state: &ThemeDesignerState,
    ) -> AnyElement {
        let palette = self.theme_palette();
        let mut row = div().flex().items_center().gap_2().w_full();
        if let Some((picker, _, _)) = self.theme_picker_controls.fields.get(field) {
            row = row.child(NyaColorPicker::new(picker).accessibility_label(label.clone()));
        }
        row = row.child(
            div()
                .flex_1()
                .min_w_0()
                .h(px(32.))
                .child(self.existing_text_input_box(format!("theme.designer.{field}"), false)),
        );
        let mut field_view = div()
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .w_full()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.text_muted))
                    .child(label),
            )
            .child(row);
        if state.errors.iter().any(|error| error == field) {
            field_view = field_view.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.danger))
                    .child(t!("themeDesigner.invalidColor")),
            );
        }
        field_view.into_any_element()
    }
}

fn theme_preview(p: nyaterm_ui::ThemePalette, name: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(rgb(p.border))
        .bg(rgb(p.bg))
        .text_color(rgb(p.text))
        .shadow(vec![
            BoxShadow::new(px(0.), px(2.), rgba(p.shadow).into()).blur_radius(px(8.)),
        ])
        .child(
            div()
                .flex()
                .justify_between()
                .gap_2()
                .child(name.to_string())
                .child(
                    div()
                        .px_2()
                        .rounded_sm()
                        .bg(rgb(p.primary))
                        .text_color(rgb(p.on_primary))
                        .child(t!("themeDesigner.previewButton")),
                ),
        )
        .child(
            div()
                .p_2()
                .rounded_sm()
                .bg(rgb(p.surface))
                .child(t!("themeDesigner.previewPanel")),
        )
        .child(
            div().p_2().rounded_sm().bg(rgb(p.ui_terminal_bg)).child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .bg(rgb(p.terminal_bg))
                    .text_color(rgb(p.terminal_fg))
                    .child(
                        div()
                            .text_color(rgb(p.terminal_ansi[10]))
                            .child("$ cargo build"),
                    )
                    .child(
                        div()
                            .text_color(rgb(p.terminal_fg_intense.unwrap_or(p.terminal_fg)))
                            .child("NyaTerm — UTF-8 你好"),
                    )
                    .child(
                        div()
                            .bg(rgb(p.terminal_line_highlight))
                            .child(t!("themeDesigner.previewLine")),
                    )
                    .child(
                        div()
                            .bg(rgb(p.terminal_selection))
                            .child(t!("themeDesigner.previewSelection")),
                    )
                    .child(
                        div()
                            .bg(rgb(p.terminal_find_bg))
                            .border_1()
                            .border_color(rgb(p.terminal_find_border))
                            .child(t!("themeDesigner.previewSearch")),
                    )
                    .child(div().w(px(9.)).h(px(18.)).bg(rgb(p.terminal_cursor))),
            ),
        )
        .child(
            div().flex().gap_1().children(
                p.terminal_ansi
                    .into_iter()
                    .map(|color| div().h(px(16.)).flex_1().bg(rgb(color))),
            ),
        )
        .into_any_element()
}
