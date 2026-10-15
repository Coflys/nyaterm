//! Theme editing is separate from the settings draft until the user saves it.
use gpui::{Context, PathPromptOptions};
use nyaterm_core::themes::{COLOR_FIELDS, ThemeConfig, contrast_ratio, resolve_theme};
use nyaterm_ui::ThemePalette;
use rust_i18n::t;

use crate::features::{NyaTermApp, runtime_jobs::await_blocking_job, text_inputs::TextInputSetup};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::features) enum ThemeIntent {
    Toggle,
    Select(String),
    Copy(String),
    Import,
    Delete,
    Apply(bool),
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(in crate::features) struct ThemeDesignerState {
    pub open: bool,
    pub draft: Option<ThemeConfig>,
    baseline: Option<ThemeConfig>,
    pub terminal_tab: bool,
    pub errors: Vec<String>,
    pub preview: Option<ThemePalette>,
    pub low_contrast: bool,
    pub pending: Option<ThemeIntent>,
    pub message: String,
    pub generation: u64,
}

impl ThemeDesignerState {
    pub fn dirty(&self) -> bool {
        self.draft != self.baseline
    }
    pub fn load(&mut self, theme: ThemeConfig) {
        self.generation += 1;
        self.open = true;
        self.baseline = Some(theme.clone());
        self.draft = Some(theme);
        self.pending = None;
        self.message.clear();
        self.recompute();
    }
    pub fn reset(&mut self) {
        let generation = self.generation + 1;
        *self = Self {
            generation,
            ..Default::default()
        };
    }
    pub fn edit(&mut self, field: &str, value: String) {
        let Some(theme) = self.draft.as_mut() else {
            return;
        };
        match field {
            "name" => theme.name = value,
            "label" => theme.label = value.chars().take(14).collect(),
            "swatch" => theme.swatch = value,
            path => {
                if let Some(field) = COLOR_FIELDS.iter().find(|f| f.path() == path) {
                    theme.set_color(*field, value);
                }
            }
        }
        self.generation += 1;
        self.recompute();
    }
    fn recompute(&mut self) {
        let Some(theme) = self.draft.as_ref() else {
            return;
        };
        self.errors = theme.validate();
        self.low_contrast = contrast_ratio(&theme.colors.text, &theme.colors.bg)
            .is_some_and(|r| r < 4.5)
            || contrast_ratio(
                &theme.colors.terminal.foreground,
                &theme.colors.terminal.background,
            )
            .is_some_and(|r| r < 4.5);
        if self.errors.is_empty() {
            self.preview = Some(nyaterm_ui::palette_from_theme(theme));
        }
    }
    pub fn saved(&mut self) {
        self.baseline = self.draft.clone();
    }
}

impl NyaTermApp {
    pub(in crate::features) fn theme_designer_action(
        &mut self,
        intent: ThemeIntent,
        cx: &mut Context<Self>,
    ) {
        if self.settings.theme_designer().dirty() {
            self.settings.theme_designer_mut().pending = Some(intent);
            self.focus_settings_tab(crate::models::SettingsTab::Appearance, cx);
            self.request_settings_panel_refresh(cx);
            return;
        }
        self.perform_theme_intent(intent, cx);
    }

    pub(in crate::features) fn resolve_theme_designer_pending(
        &mut self,
        save: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let pending = self.settings.theme_designer_mut().pending.take();
        if let Some(save) = save {
            if save && !self.save_theme_designer(None, cx) {
                self.settings.theme_designer_mut().pending = pending;
            } else if let Some(intent) = pending {
                if !save {
                    let baseline = self.settings.theme_designer().baseline.clone();
                    let editor = self.settings.theme_designer_mut();
                    editor.draft = baseline;
                    editor.generation += 1;
                    editor.recompute();
                }
                self.perform_theme_intent(intent, cx);
            }
        }
        self.request_settings_panel_refresh(cx);
    }

    fn perform_theme_intent(&mut self, intent: ThemeIntent, cx: &mut Context<Self>) {
        match intent {
            ThemeIntent::Toggle => {
                if self.settings.theme_designer().open {
                    self.settings.theme_designer_mut().reset();
                } else {
                    let summary = self.settings.summary();
                    let theme = summary
                        .custom_themes
                        .available()
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| {
                            resolve_theme(&summary.theme, &summary.custom_themes).copy_as_custom()
                        });
                    self.settings.theme_designer_mut().load(theme);
                }
            }
            ThemeIntent::Select(id) => {
                if let Some(theme) = self.settings.summary().custom_themes.resolve(&id) {
                    self.settings.theme_designer_mut().load(theme);
                }
            }
            ThemeIntent::Copy(id) => {
                let theme =
                    resolve_theme(&id, &self.settings.summary().custom_themes).copy_as_custom();
                self.settings.theme_designer_mut().load(theme);
                self.settings.theme_designer_mut().baseline = None;
            }
            ThemeIntent::Import => {
                self.prompt_theme_import(cx);
            }
            ThemeIntent::Delete => {
                if let Some(theme) = self.settings.theme_designer().draft.clone() {
                    self.settings.delete_custom_theme(&theme.id);
                    self.settings.theme_designer_mut().reset();
                    self.custom_themes_changed(cx);
                    self.perform_theme_intent(ThemeIntent::Toggle, cx);
                }
            }
            ThemeIntent::Apply(close) => self.apply_settings_draft(close, cx),
        }
        self.ensure_theme_designer_inputs(cx);
        self.request_settings_panel_refresh(cx);
        cx.notify();
    }

    pub(in crate::features) fn edit_theme_designer(
        &mut self,
        field: &str,
        value: String,
        cx: &mut Context<Self>,
    ) {
        self.settings.theme_designer_mut().edit(field, value);
        self.request_settings_panel_refresh(cx);
    }

    pub(in crate::features) fn ensure_theme_designer_inputs(&mut self, cx: &mut Context<Self>) {
        if !self.settings.theme_designer().open {
            return;
        }
        let Some(theme) = self.settings.theme_designer().draft.clone() else {
            return;
        };
        let mut fields = vec![
            ("name", theme.name.as_str()),
            ("label", theme.label.as_str()),
            ("swatch", theme.swatch.as_str()),
        ];
        fields.extend(COLOR_FIELDS.iter().map(|f| (f.path(), theme.color(*f))));
        for (field, value) in fields {
            let id = format!("theme.designer.{field}");
            self.ensure_text_input(id.clone(), value, TextInputSetup::default(), cx);
            if let Some(input) = self.existing_text_input(id)
                && input.read(cx).value(cx) != value
            {
                input.update(cx, |input, cx| input.set_content_silent(value, cx));
            }
        }
    }

    pub(in crate::features) fn save_theme_designer(
        &mut self,
        target: Option<bool>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(theme) = self.settings.theme_designer().draft.clone() else {
            return false;
        };
        if let Err(error) = self.settings.store_custom_theme(theme.clone()) {
            self.settings.theme_designer_mut().message = error;
            self.request_settings_panel_refresh(cx);
            return false;
        }
        self.settings.theme_designer_mut().saved();
        self.settings.theme_designer_mut().message = t!("themeDesigner.staged").to_string();
        match target {
            Some(false) => self.settings.set_appearance_theme(theme.id),
            Some(true) => self.settings.set_terminal_theme(Some(theme.id)),
            None => {}
        }
        self.custom_themes_changed(cx);
        true
    }

    fn custom_themes_changed(&mut self, cx: &mut Context<Self>) {
        self.invalidate_paint_theme_caches();
        self.sync_component_theme(cx);
        self.refresh_visible_terminal_surfaces(cx);
        self.defer_connection_panel_snapshot_flush(cx);
        self.defer_remote_panel_snapshot_flush(cx);
        self.defer_transfer_panel_snapshot_flush(cx);
        self.defer_ai_panel_snapshot_flush(cx);
        self.save_appearance_settings(cx);
        self.request_settings_panel_refresh(cx);
        cx.notify();
    }

    fn prompt_theme_import(&mut self, cx: &mut Context<Self>) {
        self.settings.theme_designer_mut().generation += 1;
        let generation = self.settings.theme_designer().generation;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t!("themeDesigner.import").into()),
        });
        let scheduler = self.blocking_jobs.clone();
        cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => await_blocking_job(
                        scheduler.submit_task("theme-import", move |_| read_theme(&path)),
                    )
                    .await
                    .and_then(|r| r)
                    .map(Some),
                    None => Ok(None),
                },
                Ok(Ok(None)) => Ok(None),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.finish_theme_import(generation, result, cx);
            });
        })
        .detach();
    }

    pub(in crate::features) fn finish_theme_import(
        &mut self,
        generation: u64,
        result: Result<Option<ThemeConfig>, String>,
        cx: &mut Context<Self>,
    ) {
        if self.settings.theme_designer().generation != generation {
            return;
        }
        match result {
            Ok(Some(mut theme)) => {
                theme.normalize_import(&self.settings.summary().custom_themes);
                match self.settings.store_custom_theme(theme.clone()) {
                    Ok(()) => {
                        self.settings.theme_designer_mut().load(theme);
                        self.settings.theme_designer_mut().message =
                            t!("themeDesigner.staged").to_string();
                        self.ensure_theme_designer_inputs(cx);
                        self.custom_themes_changed(cx);
                    }
                    Err(error) => self.settings.theme_designer_mut().message = error,
                }
            }
            Ok(None) => {}
            Err(error) => self.settings.theme_designer_mut().message = error,
        }
        self.request_settings_panel_refresh(cx);
    }

    pub(in crate::features) fn export_theme_designer(&mut self, cx: &mut Context<Self>) {
        let Some(theme) = self
            .settings
            .theme_designer()
            .draft
            .clone()
            .filter(|t| t.validate().is_empty())
        else {
            return;
        };
        self.settings.theme_designer_mut().generation += 1;
        let generation = self.settings.theme_designer().generation;
        let receiver =
            cx.prompt_for_new_path(self.runtime.config_dir(), Some("nyaterm-theme.json"));
        let scheduler = self.blocking_jobs.clone();
        cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(Some(path))) => await_blocking_job(
                    scheduler.submit_task("theme-export", move |_| write_theme(&path, &theme)),
                )
                .await
                .and_then(|r| r),
                Ok(Ok(None)) => return,
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                if this.settings.theme_designer().generation != generation {
                    return;
                }
                this.settings.theme_designer_mut().message = match result {
                    Ok(()) => t!("themeDesigner.exported").to_string(),
                    Err(error) => error,
                };
                this.request_settings_panel_refresh(cx);
            });
        })
        .detach();
    }
}

fn read_theme(path: &std::path::Path) -> Result<ThemeConfig, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Theme JSON exceeds 1 MiB".into());
    }
    let theme: ThemeConfig = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(theme)
}

fn write_theme(path: &std::path::Path, theme: &ThemeConfig) -> Result<(), String> {
    use std::io::Write;
    let errors = theme.validate();
    if !errors.is_empty() {
        return Err(errors.join(", "));
    }
    let parent = path.parent().ok_or("Invalid theme path")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, theme).map_err(|e| e.to_string())?;
    file.write_all(b"\n").map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ThemeDesignerState, read_theme, write_theme};
    use nyaterm_core::themes::builtins;
    #[test]
    fn invalid_edits_keep_the_last_valid_preview_and_never_mark_saved() {
        let mut state = ThemeDesignerState::default();
        state.load(builtins()[0].copy_as_custom());
        let preview = state.preview;
        let generation = state.generation;
        state.edit("primary", "broken".into());
        assert!(state.dirty());
        assert!(state.errors.iter().any(|field| field == "primary"));
        assert_eq!(state.preview, preview);
        assert!(state.generation > generation);
        state.edit("primary", "#123456".into());
        assert!(state.errors.is_empty());
        assert_eq!(state.preview.unwrap().primary, 0x123456);
        state.saved();
        assert!(!state.dirty());
        let generation = state.generation;
        state.reset();
        assert!(state.generation > generation);
        assert!(!state.open);
    }
    #[test]
    fn theme_files_round_trip_without_saving_settings_and_reject_invalid_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("theme.json");
        let theme = builtins()[0].copy_as_custom();
        write_theme(&path, &theme).unwrap();
        assert_eq!(read_theme(&path).unwrap(), theme);
        let before = std::fs::read(&path).unwrap();
        let mut invalid = theme.clone();
        invalid.colors.primary = "invalid".into();
        assert!(write_theme(&path, &invalid).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::write(&path, b"not JSON").unwrap();
        assert!(read_theme(&path).is_err());
        std::fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
        assert!(read_theme(&path).unwrap_err().contains("1 MiB"));
    }
}
