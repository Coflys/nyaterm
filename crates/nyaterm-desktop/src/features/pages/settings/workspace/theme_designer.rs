use gpui::{
    AnyElement, AppContext as _, BoxShadow, Context, Entity, IntoElement, ParentElement as _, Rgba,
    Styled as _, Subscription, Window, div, prelude::*, px, rgb, rgba,
};
use nyaterm_core::themes::{COLOR_FIELDS, parse_color};
use nyaterm_ui::{
    NyaButton, NyaButtonVariant, NyaColorPicker, NyaColorPickerEvent, NyaColorPickerState,
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
        let Some(theme) = state.draft.as_ref().filter(|_| state.open) else {
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
        let mut section = div().flex().flex_col().gap_3().w_full().min_w_0().child(
            NyaButton::new("theme-designer-toggle", t!("themeDesigner.manage")).on_click(
                cx.listener(|panel, _, _, cx| {
                    panel.with_app(cx, |app, cx| {
                        app.theme_designer_action(ThemeIntent::Toggle, cx)
                    })
                }),
            ),
        );
        if !state.open {
            return section.into_any_element();
        }

        if state.pending.is_some() {
            let mut prompt = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .p_3()
                .rounded_md()
                .bg(rgb(palette.section_header))
                .text_color(rgb(palette.warning))
                .child(t!("themeDesigner.unsaved"));
            for (id, label, choice) in [
                ("save", "themeDesigner.save", Some(true)),
                ("discard", "themeDesigner.discard", Some(false)),
                ("back", "themeDesigner.back", None),
            ] {
                prompt = prompt.child(
                    NyaButton::new(format!("theme-pending-{id}"), t!(label)).on_click(cx.listener(
                        move |panel, _, _, cx| {
                            panel.with_app(cx, |app, cx| {
                                app.resolve_theme_designer_pending(choice, cx)
                            })
                        },
                    )),
                );
            }
            section = section.child(prompt);
        }
        section = section.child(
            div()
                .text_sm()
                .text_color(rgb(palette.text_muted))
                .child(t!("themeDesigner.description")),
        );

        let catalog = self.settings.summary().custom_themes.clone();
        let mut list = div().flex().flex_col().gap_2().min_w_0().w_full();
        list = list.child(
            NyaButton::new("theme-import", t!("themeDesigner.import")).on_click(cx.listener(
                |panel, _, _, cx| {
                    panel.with_app(cx, |app, cx| {
                        app.theme_designer_action(ThemeIntent::Import, cx)
                    })
                },
            )),
        );
        let current = self.settings.summary().theme.clone();
        list = list.child(
            NyaButton::new("theme-copy-current", t!("themeDesigner.copyCurrent")).on_click(
                cx.listener(move |panel, _, _, cx| {
                    panel.with_app(cx, |app, cx| {
                        app.theme_designer_action(ThemeIntent::Copy(current.clone()), cx)
                    })
                }),
            ),
        );
        list = list.child(div().text_sm().child(t!("themeDesigner.copyFrom")));
        // The same selector catalog is used by UI/terminal theme settings and menus.
        let selected_source = self.settings.summary().theme.clone();
        let options = self
            .settings
            .theme_options
            .iter()
            .cloned()
            .map(|(id, name)| nyaterm_ui::NyaSelectOption::new(id, name))
            .collect();
        list = list.child(self.select_control(
            "theme-designer-source",
            options,
            Some(selected_source),
            false,
            cx,
        ));
        let custom_options: Vec<_> = self
            .settings
            .theme_options
            .iter()
            .filter(|(id, _)| !nyaterm_core::themes::is_builtin(id))
            .cloned()
            .collect();
        for (theme_id, name) in &custom_options {
            let id = theme_id.clone();
            let selected = state.draft.as_ref().is_some_and(|d| d.id == id);
            list = list.child(
                NyaButton::new(format!("theme-custom-{theme_id}"), name.clone())
                    .full_width()
                    .selected(selected)
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.with_app(cx, |app, cx| {
                            app.theme_designer_action(ThemeIntent::Select(id.clone()), cx)
                        })
                    })),
            );
        }
        let unsupported = catalog.records().len().saturating_sub(custom_options.len());
        if unsupported > 0 || !catalog.0.is_array() {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.warning))
                    .child(t!("themeDesigner.unsupported", count = unsupported)),
            );
        }
        for diagnostic in self.settings.theme_diagnostics.iter() {
            list = list.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.warning))
                    .child(diagnostic.clone()),
            );
        }
        let mut editor = div().flex().flex_col().gap_3().flex_1().min_w_0().w_full();
        if let Some(theme) = state.draft.as_ref() {
            for (field, label) in [
                ("name", "themeDesigner.name"),
                ("label", "themeDesigner.label"),
                ("swatch", "themeDesigner.swatch"),
            ] {
                editor = editor.child(self.theme_color_input(field, t!(label).to_string(), &state));
            }
            if let Some(preview) = state.preview {
                editor = editor.child(theme_preview(preview, &theme.name));
            }
            if state.low_contrast {
                editor = editor.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.warning))
                        .child(t!("themeDesigner.contrastWarning")),
                );
            }
            let mut tabs = div().flex().gap_2();
            for (terminal, label) in [
                (false, "themeDesigner.uiColors"),
                (true, "themeDesigner.terminalColors"),
            ] {
                tabs = tabs.child(
                    NyaButton::new(format!("theme-tab-{terminal}"), t!(label))
                        .selected(state.terminal_tab == terminal)
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
                .filter(|f| f.terminal() == state.terminal_tab)
            {
                grid = grid.child(div().flex_1().min_w(px(190.)).child(self.theme_color_input(
                    field.path(),
                    t!(format!("themeDesigner.colors.{}", field.path())).to_string(),
                    &state,
                )));
            }
            editor = editor.child(grid);
            let valid = state.errors.is_empty();
            let mut actions = div().flex().flex_wrap().gap_2();
            for (id, label, target) in [
                ("save", "themeDesigner.save", None),
                ("ui", "themeDesigner.useUi", Some(false)),
                ("terminal", "themeDesigner.useTerminal", Some(true)),
            ] {
                actions = actions.child(
                    NyaButton::new(format!("theme-{id}"), t!(label))
                        .variant(NyaButtonVariant::Primary)
                        .disabled(!valid)
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            panel.with_app(cx, |app, cx| {
                                app.save_theme_designer(target, cx);
                            })
                        })),
                );
            }
            actions = actions
                .child(
                    NyaButton::new("theme-export", t!("themeDesigner.export"))
                        .disabled(!valid)
                        .on_click(cx.listener(|panel, _, _, cx| {
                            panel.with_app(cx, |app, cx| app.export_theme_designer(cx))
                        })),
                )
                .child(
                    NyaButton::new("theme-delete", t!("themeDesigner.delete"))
                        .variant(NyaButtonVariant::Danger)
                        .disabled(!catalog.contains_id(&theme.id))
                        .on_click(cx.listener(|panel, _, _, cx| {
                            panel.with_app(cx, |app, cx| {
                                app.theme_designer_action(ThemeIntent::Delete, cx)
                            })
                        })),
                );
            editor = editor.child(actions);
        }
        if !state.message.is_empty() {
            editor = editor.child(div().text_sm().child(state.message.clone()));
        }
        let narrow = self.viewport_width < 850.;
        section
            .child(
                div()
                    .flex()
                    .when(narrow, |d| d.flex_col())
                    .gap_4()
                    .w_full()
                    .min_w_0()
                    .child(
                        div()
                            .when(!narrow, |d| d.w(px(210.)).flex_none())
                            .child(list),
                    )
                    .child(editor),
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
