use gpui::Context;
use nyaterm_core::{AiCommandCard, AppendAiAuditRequest, QuickCommand, QuickCommandCategory, uuid};
use nyaterm_store::StoreDomain;

use crate::features::NyaTermApp;
use crate::models::QuickCommandVariablePromptState;

use super::escapes::{QuickCommandChunk, plan_quick_command, quick_command_needs_staging};
use super::helpers::{
    ai_command_card_category_name, unique_quick_command_category_id, validated_quick_command_text,
};
use super::variables::parse_quick_command_variables;

impl NyaTermApp {
    pub(in crate::features) fn save_ai_command_card_by_id(
        &mut self,
        card_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.find_ai_command_card(&card_id) else {
            self.ai
                .set_panel_status("AI command card is no longer available");
            self.defer_ai_panel_snapshot_flush(cx);
            return;
        };
        self.save_ai_command_card_value(card, cx);
    }

    pub(in crate::features) fn save_ai_command_card_value(
        &mut self,
        card: AiCommandCard,
        cx: &mut Context<Self>,
    ) {
        let command_text = card.command.trim().to_string();
        if command_text.is_empty() {
            self.ai.set_panel_status("AI command card has no command");
            self.defer_ai_panel_snapshot_flush(cx);
            return;
        }

        let connection_id = self.session.active_id_owned();
        let response_preview = self.ai.chat_response_preview().to_string();
        self.submit_store_request(
            0,
            nyaterm_store::store_mutation(StoreDomain::Commands, move |store| {
                let config = store.load_quick_commands()?;
                let category_name = ai_command_card_category_name(&card);
                let existing_category = config
                    .categories
                    .iter()
                    .find(|category| category.name == category_name)
                    .cloned();
                let (category_id, new_category) = match existing_category {
                    Some(category) => (category.id, None),
                    None => {
                        let id =
                            unique_quick_command_category_id(&config.categories, &category_name);
                        (
                            id.clone(),
                            Some(QuickCommandCategory {
                                id,
                                name: category_name,
                                parent_id: None,
                                sort_order: config
                                    .categories
                                    .iter()
                                    .filter(|category| category.parent_id.is_none())
                                    .map(|category| category.sort_order)
                                    .max()
                                    .unwrap_or(-1)
                                    .saturating_add(1),
                            }),
                        )
                    }
                };
                let label = if card.title.trim().is_empty() {
                    "AI Command".to_string()
                } else {
                    card.title.trim().to_string()
                };
                let description = if card.explanation.trim().is_empty() {
                    None
                } else {
                    Some(card.explanation.trim().to_string())
                };
                store.upsert_quick_command(
                    QuickCommand {
                        id: format!("ai-{}", uuid()),
                        label: label.clone(),
                        command: command_text,
                        category_id: Some(category_id),
                        description,
                        color_tag: Some("blue".to_string()),
                        icon_tag: Some("terminal".to_string()),
                        pinned: Some(false),
                        execution_mode: Some("append".to_string()),
                        source: Some("ai".to_string()),
                        risk_level: card.risk_level.clone(),
                        updated_at: None,
                        created_at: None,
                        use_count: None,
                        sort_order: None,
                    },
                    new_category,
                )?;
                store
                    .append_ai_audit(AppendAiAuditRequest {
                        connection_id,
                        action: "ai.save_quick_command".to_string(),
                        user_input: Some(response_preview),
                        generated_command: Some(card.command.clone()),
                        risk_level: card.risk_level.clone(),
                        inserted_to_terminal: false,
                        executed: false,
                        blocked: false,
                        ..Default::default()
                    })
                    .map(|_| label)
            }),
            |this, event, cx| {
                match event.outcome {
                    Ok(label) => {
                        this.refresh_ai_usage_counts(cx);
                        this.refresh_quick_commands(cx);
                        this.ai.set_panel_status(format!(
                            "Saved AI command card '{}' to Quick Commands",
                            label
                        ));
                        this.settings
                            .update_store_status(this.ai.panel_status().to_string(), true);
                    }
                    Err(error) => {
                        this.ai
                            .set_panel_status(format!("Quick command save failed: {error}"));
                        this.settings
                            .update_store_status(this.ai.panel_status().to_string(), false);
                    }
                }
                this.request_settings_panel_refresh(cx);
                this.defer_ai_panel_snapshot_flush(cx);
            },
            cx,
        );
    }

    pub(in crate::features) fn run_quick_command_by_id(
        &mut self,
        command_id: String,
        cx: &mut Context<Self>,
    ) {
        self.apply_quick_command_by_id(&command_id, false, cx);
    }

    pub(in crate::features) fn send_quick_command_to_all_by_id(
        &mut self,
        command_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(command) = self
            .commands
            .quick_commands()
            .iter()
            .find(|command| command.id == command_id)
            .cloned()
        else {
            self.shell
                .set_status("quick command is no longer available".to_string());
            cx.notify();
            return;
        };
        self.apply_quick_command_by_id(&command.id, true, cx);
    }

    pub(in crate::features) fn apply_quick_command_by_id(
        &mut self,
        command_id: &str,
        send_to_all: bool,
        cx: &mut Context<Self>,
    ) {
        if send_to_all {
            if self.session.live_session_count() == 0 {
                self.shell.set_status(
                    "start a terminal session before using a quick command".to_string(),
                );
                cx.notify();
                return;
            }
        } else if self.session.active_id().is_none() {
            self.shell
                .set_status("start a terminal session before using a quick command".to_string());
            cx.notify();
            return;
        }
        let Some(command) = self
            .commands
            .quick_commands()
            .iter()
            .find(|command| command.id == command_id)
            .cloned()
        else {
            self.shell
                .set_status("quick command is no longer available".to_string());
            cx.notify();
            return;
        };
        let Some(command_text) = validated_quick_command_text(&command.command) else {
            self.shell
                .set_status("quick command has no command text".to_string());
            cx.notify();
            return;
        };
        let execute = quick_command_executes(&command);
        let variables = parse_quick_command_variables(&command_text);
        if !variables.is_empty() {
            self.commands
                .request_quick_variable_prompt(QuickCommandVariablePromptState {
                    command_id: command.id,
                    label: command.label,
                    command: command_text,
                    execute,
                    send_to_all,
                    variables,
                });
            self.shell
                .set_status("fill quick command variables".to_string());
            cx.notify();
            return;
        }
        self.send_resolved_quick_command(
            command.id,
            command.label,
            command_text,
            execute,
            send_to_all,
            cx,
        );
    }

    pub(in crate::features) fn send_resolved_quick_command(
        &mut self,
        command_id: String,
        label: String,
        command_text: String,
        execute: bool,
        send_to_all: bool,
        cx: &mut Context<Self>,
    ) {
        // `\r \n \t \p \\` are interpreted here rather than at save time: the
        // stored script keeps the escapes, so what the panel shows is what the
        // user wrote.
        let mut chunks = plan_quick_command(&command_text, execute);
        self.queue_quick_command_use_count(command_id);

        if quick_command_needs_staging(&chunks) {
            self.send_staged_quick_command(label, chunks, execute, send_to_all, cx);
            return;
        }

        // The common case is one chunk, which goes through the ordinary input
        // path exactly as before. The staging gate above already excluded any
        // plan that carries a pause, so this chunk's delay is zero.
        let Some(chunk) = chunks.pop() else {
            return;
        };
        let command_bytes = chunk.bytes;

        if send_to_all {
            let sessions = self
                .session
                .ordered_sessions()
                .into_iter()
                .filter(|session| !self.session.is_disconnected(&session.id))
                .collect::<Vec<_>>();
            if sessions.is_empty() {
                self.shell.set_status(
                    "start a terminal session before using a quick command".to_string(),
                );
                cx.notify();
                return;
            }
            let mut sent = 0usize;
            let mut failed = 0usize;
            let mut ok_sessions = Vec::new();
            for session in sessions {
                match self.write_session_input_recorded(&session.id, &command_bytes) {
                    Ok(()) => {
                        sent += 1;
                        ok_sessions.push(session.id);
                    }
                    Err(_) => failed += 1,
                }
            }
            let session_refs: Vec<&str> = ok_sessions.iter().map(String::as_str).collect();
            self.record_command_history_for_sessions(&session_refs, &command_bytes);
            self.shell.set_status(if failed == 0 {
                format!("sent quick command '{label}' to {sent} session(s)")
            } else {
                format!("sent quick command '{label}' to {sent} session(s), {failed} failed")
            });
            cx.notify();
            return;
        }

        if self.send_terminal_input(command_bytes, cx) {
            self.shell.set_status(if execute {
                format!("ran quick command '{label}'")
            } else {
                format!("inserted quick command '{label}'")
            });
            cx.notify();
        }
    }

    /// Send a script that `\p` split into several writes.
    ///
    /// The target sessions are pinned once, here, when the user runs the
    /// command: the rest of the script must keep arriving at what was clicked
    /// on rather than following the focus during the pauses.
    fn send_staged_quick_command(
        &mut self,
        label: String,
        chunks: Vec<QuickCommandChunk>,
        execute: bool,
        send_to_all: bool,
        cx: &mut Context<Self>,
    ) {
        // The ordinary input path also refuses to write while a session is
        // still coming up, so a staged script waits for it the same way.
        let starting = !send_to_all
            && (self.session.start_has_active_pending() || self.session.start_has_active_failed());
        let targets = if send_to_all {
            self.session
                .ordered_sessions()
                .into_iter()
                .filter(|session| !self.session.is_disconnected(&session.id))
                .map(|session| session.id)
                .collect::<Vec<_>>()
        } else if starting {
            Vec::new()
        } else {
            let primary = self
                .session
                .active_id_owned()
                .filter(|session_id| !self.session.is_disconnected(session_id));
            match primary {
                Some(session_id) => {
                    let peers = self.sync_peer_session_ids(&session_id);
                    std::iter::once(session_id).chain(peers).collect()
                }
                None => Vec::new(),
            }
        };
        if targets.is_empty() {
            if starting {
                // Nothing to report: the panel already shows the connection.
                return;
            }
            self.shell
                .set_status("start a terminal session before using a quick command".to_string());
            cx.notify();
            return;
        }

        let steps = chunks.len();
        self.shell
            .set_status(format!("sending quick command '{label}' in {steps} steps"));
        cx.notify();

        cx.spawn(async move |this, cx| {
            let mut live = targets;
            let total = live.len();
            for chunk in chunks {
                if !chunk.delay.is_zero() {
                    cx.background_executor().timer(chunk.delay).await;
                }
                let Ok(survivors) = this.update(cx, |this, cx| {
                    let survivors = this.write_quick_command_chunk(&live, &chunk.bytes, cx);
                    cx.notify();
                    survivors
                }) else {
                    // The window went away, so there is no one left to report to.
                    return;
                };
                live = survivors;
                if live.is_empty() {
                    break;
                }
            }
            let failed = total - live.len();
            let delivered = live.len();
            let _ = this.update(cx, |this, cx| {
                this.shell.set_status(quick_command_delivery_status(
                    &label,
                    execute,
                    send_to_all,
                    delivered,
                    failed,
                ));
                cx.notify();
            });
        })
        .detach();
    }

    /// Write one staged chunk to every target that is still connected, and
    /// return the ones that took it.
    fn write_quick_command_chunk(
        &mut self,
        targets: &[String],
        bytes: &[u8],
        cx: &mut Context<Self>,
    ) -> Vec<String> {
        let mut survivors = Vec::with_capacity(targets.len());
        for session_id in targets {
            if self.session.is_disconnected(session_id) {
                continue;
            }
            if self.write_session_input_recorded(session_id, bytes).is_ok() {
                survivors.push(session_id.clone());
            }
        }
        if !survivors.is_empty() {
            // A chunk without a return key records nothing on its own, so a
            // paused script still lands in history as one command line.
            let session_refs: Vec<&str> = survivors.iter().map(String::as_str).collect();
            self.record_command_history_for_sessions(&session_refs, bytes);
            self.arm_terminal_input_wake(cx);
        }
        survivors
    }
}

fn quick_command_delivery_status(
    label: &str,
    execute: bool,
    send_to_all: bool,
    delivered: usize,
    failed: usize,
) -> String {
    if send_to_all {
        return if failed == 0 {
            format!("sent quick command '{label}' to {delivered} session(s)")
        } else {
            format!("sent quick command '{label}' to {delivered} session(s), {failed} failed")
        };
    }
    if failed > 0 {
        return format!("quick command '{label}' failed on {failed} session(s)");
    }
    if execute {
        format!("ran quick command '{label}'")
    } else {
        format!("inserted quick command '{label}'")
    }
}

fn quick_command_executes(command: &QuickCommand) -> bool {
    command.execution_mode.as_deref() != Some("append")
}

#[cfg(test)]
mod tests {
    use nyaterm_core::QuickCommand;

    use super::{quick_command_delivery_status, quick_command_executes};

    fn command_with_execution_mode(execution_mode: Option<&str>) -> QuickCommand {
        QuickCommand {
            id: "command-1".to_string(),
            label: "Command".to_string(),
            command: "pwd".to_string(),
            category_id: None,
            description: None,
            color_tag: None,
            icon_tag: None,
            pinned: None,
            execution_mode: execution_mode.map(ToOwned::to_owned),
            source: None,
            risk_level: None,
            updated_at: None,
            created_at: None,
            use_count: None,
            sort_order: None,
        }
    }

    #[test]
    fn append_mode_inserts_without_execution() {
        assert!(!quick_command_executes(&command_with_execution_mode(Some(
            "append"
        ))));
    }

    #[test]
    fn execute_and_legacy_modes_execute() {
        assert!(quick_command_executes(&command_with_execution_mode(Some(
            "execute"
        ))));
        assert!(quick_command_executes(&command_with_execution_mode(None)));
    }

    #[test]
    fn staged_delivery_reports_broadcast_reach() {
        assert_eq!(
            quick_command_delivery_status("Login", true, true, 4, 0),
            "sent quick command 'Login' to 4 session(s)"
        );
        assert_eq!(
            quick_command_delivery_status("Login", true, true, 2, 1),
            "sent quick command 'Login' to 2 session(s), 1 failed"
        );
    }

    #[test]
    fn staged_delivery_keeps_the_single_session_wording() {
        assert_eq!(
            quick_command_delivery_status("Login", true, false, 1, 0),
            "ran quick command 'Login'"
        );
        assert_eq!(
            quick_command_delivery_status("Login", false, false, 1, 0),
            "inserted quick command 'Login'"
        );
        assert_eq!(
            quick_command_delivery_status("Login", true, false, 0, 1),
            "quick command 'Login' failed on 1 session(s)"
        );
    }
}
