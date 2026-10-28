use gpui::{AnyElement, Context, FontWeight, KeyDownEvent, SharedString, div, prelude::*, px, rgb};
use nyaterm_transport::SshCredentialPromptReason;
use nyaterm_transport::ssh_authentication::SshAuthenticationMethod;
use nyaterm_ui::{
    NyaButton, NyaButtonVariant, NyaCheckbox, NyaScrollable, NyaSelectOption, NyaTabItem, NyaTabs,
};
use rust_i18n::t;

use super::state::{AuthMethod, SaveDestination};
use crate::features::{NyaTermApp, session::CredentialPromptState, text_inputs::TextInputSetup};

impl NyaTermApp {
    pub(in crate::features) fn runtime_authentication_view(
        &mut self,
        state: CredentialPromptState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let form = state
            .authentication
            .as_ref()
            .expect("structured authentication form");
        let palette = self.theme_palette();
        let reason = match state.prompt.reason {
            SshCredentialPromptReason::PasswordRejected => t!("sshAuth.passwordRejected"),
            SshCredentialPromptReason::PublickeyRequired => t!("sshAuth.publickeyRequired"),
            SshCredentialPromptReason::PublickeyRejected => t!("sshAuth.publickeyRejected"),
            SshCredentialPromptReason::KeyPassphraseRequired => t!("sshAuth.keyPassphraseRequired"),
            SshCredentialPromptReason::KeyPassphraseRejected => t!("sshAuth.keyPassphraseRejected"),
            _ => t!("sshAuth.missingPassword"),
        };
        let mut content = div()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight(700.))
                    .child(t!("sshAuth.title")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.text_muted))
                    .child(t!(
                        "sshAuth.description",
                        name = state.prompt.connection_name
                    )),
            )
            .child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(palette.border))
                    .p_3()
                    .child(div().text_sm().child(format!(
                        "{}@{}:{}",
                        state.prompt.username, state.prompt.host, state.prompt.port
                    )))
                    .child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(rgb(palette.text_muted))
                            .child(reason),
                    ),
            );
        if !form.is_passphrase() {
            let password_available = form.request.available_methods.iter().any(|method| {
                matches!(
                    method,
                    SshAuthenticationMethod::Password
                        | SshAuthenticationMethod::KeyboardInteractive
                )
            });
            if password_available
                && form
                    .request
                    .available_methods
                    .contains(&SshAuthenticationMethod::Publickey)
            {
                content = content.child(
                    NyaTabs::new("ssh-auth-method")
                        .items([
                            NyaTabItem::new(t!("sshAuth.passwordMethod")).disabled(form.busy),
                            NyaTabItem::new(t!("sshAuth.keyMethod")).disabled(form.busy),
                        ])
                        .selected_index(usize::from(form.method == AuthMethod::Key))
                        .on_select(cx.listener(|this, index, window, cx| {
                            this.change_runtime_auth_source(true, *index, window, cx)
                        })),
                );
            }
            if form.method == AuthMethod::Password {
                content = content.child(
                    NyaTabs::new("ssh-auth-password-source")
                        .items([
                            NyaTabItem::new(t!("dialog.directPassword")).disabled(form.busy),
                            NyaTabItem::new(t!("dialog.savedPassword")).disabled(form.busy),
                        ])
                        .selected_index(usize::from(form.saved_password))
                        .on_select(cx.listener(|this, index, window, cx| {
                            this.change_runtime_auth_source(false, *index, window, cx)
                        })),
                );
            }
        }
        if form.uses_secret_input() {
            let id = super::super::credential_text_input_id(&state.id);
            let setup = if form.is_passphrase() {
                TextInputSetup::secret()
            } else {
                TextInputSetup::masked()
            };
            self.text_input(id.clone(), &state.value, setup.clone(), cx)
                .update(cx, |input, cx| input.set_disabled(form.busy, cx));
            content = content
                .child(div().text_xs().child(if form.is_passphrase() {
                    t!("sshAuth.passphrase")
                } else {
                    t!("dialog.password")
                }))
                .child(self.text_input_box(id, &state.value, setup, cx));
        } else {
            let key = form.method == AuthMethod::Key;
            let options: Vec<NyaSelectOption> = if key {
                self.security
                    .ssh_keys()
                    .iter()
                    .filter(|entry| entry.has_key_data)
                    .map(|entry| NyaSelectOption::new(entry.id.clone(), entry.name.clone()))
                    .collect()
            } else {
                self.security
                    .passwords()
                    .iter()
                    .filter(|entry| entry.has_password)
                    .map(|entry| {
                        NyaSelectOption::new(entry.id.clone(), entry.name.clone())
                            .subtitle(entry.username.clone())
                    })
                    .collect()
            };
            let selected = if key {
                form.key_id.clone()
            } else {
                form.password_id.clone()
            };
            content = content.child(self.form_select_control(
                if key {
                    "ssh-auth-key"
                } else {
                    "ssh-auth-password"
                },
                options,
                selected,
                form.busy,
                cx,
            ));
            let mut actions = div().flex().gap_2().child(
                NyaButton::new("ssh-auth-refresh", t!("common.refresh"))
                    .small()
                    .disabled(form.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_security_catalog(cx))),
            );
            if key {
                let selected = form.key_id.clone();
                actions = actions.child(
                    NyaButton::new("ssh-auth-manage", t!("sshAuth.manageKeys"))
                        .small()
                        .disabled(form.busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_security_key_editor(selected.clone(), window, cx)
                        })),
                );
            }
            content = content.child(actions);
        }
        if form.can_remember() {
            let mut remember = div()
                .flex()
                .flex_col()
                .gap_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(palette.border))
                .p_3()
                .child(
                    NyaCheckbox::new("ssh-auth-remember")
                        .label(t!("sshAuth.rememberCredential"))
                        .checked(form.remember)
                        .disabled(form.busy)
                        .on_click(cx.listener(|this, checked, _, cx| {
                            if let Some(form) = this
                                .session
                                .prompts
                                .active_credential_mut()
                                .and_then(|s| s.authentication.as_mut())
                            {
                                form.remember = *checked;
                            }
                            cx.notify();
                        })),
                );
            if form.remember && !form.is_passphrase() && !form.saved_password {
                let mut options = vec![
                    NyaSelectOption::new("connection", t!("sshAuth.saveToConnection")),
                    NyaSelectOption::new("new", t!("sshAuth.createSavedPassword")),
                ];
                if form.context.account_id.is_some() {
                    options.push(NyaSelectOption::new(
                        "linked",
                        t!("sshAuth.updateSavedPassword"),
                    ));
                }
                let selected = match form.destination {
                    SaveDestination::Connection => "connection",
                    SaveDestination::NewAccount => "new",
                    SaveDestination::LinkedAccount => "linked",
                };
                remember = remember.child(self.form_select_control(
                    "ssh-auth-save",
                    options,
                    Some(selected.into()),
                    form.busy,
                    cx,
                ));
                if form.destination == SaveDestination::NewAccount {
                    self.text_input(
                        "ssh.auth.name",
                        &form.name,
                        TextInputSetup::placeholder(t!("sshAuth.savedPasswordName")),
                        cx,
                    )
                    .update(cx, |input, cx| input.set_disabled(form.busy, cx));
                    remember = remember.child(self.text_input_box(
                        "ssh.auth.name",
                        &form.name,
                        TextInputSetup::placeholder(t!("sshAuth.savedPasswordName")),
                        cx,
                    ));
                } else if form.destination == SaveDestination::LinkedAccount {
                    remember = remember.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.text_muted))
                            .child(t!("sshAuth.sharedAccountWarning")),
                    );
                }
            }
            content = content.child(remember);
        }
        if let Some(error) = &form.error {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.text))
                    .child(error.clone()),
            );
        }
        let buttons =
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    NyaButton::new("ssh-auth-cancel", t!("common.cancel"))
                        .variant(NyaButtonVariant::Ghost)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_credential_prompt(window, cx)
                        })),
                )
                .child(
                    NyaButton::new("ssh-auth-submit", t!("sshAuth.submit"))
                        .variant(NyaButtonVariant::Primary)
                        .disabled(!form.can_submit(&state.value))
                        .loading(form.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.submit_runtime_authentication(window, cx)
                        })),
                );
        div()
            .id(SharedString::from(format!(
                "credential-dialog-{}",
                state.id
            )))
            .w_full()
            .max_h_full()
            .min_h_0()
            .rounded_md()
            .border_1()
            .border_color(rgb(palette.border))
            .bg(self.shell_surface_color(palette.bg))
            .shadow_lg()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .on_click(|_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.handle_credential_key_down(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(content.max_h(px(550.)).overflow_y_scrollbar())
            .child(buttons)
            .into_any_element()
    }
}
