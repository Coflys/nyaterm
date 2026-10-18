use nyaterm_core::network_cli_prompt::{
    resolve_ssh_execution_profile, ssh_allows_background_execution,
};
use nyaterm_core::raw_cli_capture::RawCliEnd;

use std::time::{Duration, Instant};

use gpui::Context;
use nyaterm_core::{
    AgentApprovalOutcome, AgentCapturedOutput, AiAction, AiChatRequest, AiCommandCard,
    AiExecutionProfile, AiMode, AppendAiAuditRequest, CommandObservation,
    build_agent_capture_command, build_observation_message, truncate_preview, uuid,
};
use nyaterm_store::StoreDomain;
use nyaterm_transport::{SessionKind, SshProcessService, run_local_command};

use crate::features::{
    NyaTermApp, runtime_jobs::AiAgentBackgroundTarget, runtime_jobs::AiAgentLoopState,
    runtime_jobs::AiAgentStepStatus, runtime_jobs::AiChatJobResult,
    runtime_jobs::AiChatWorkerEvent, runtime_jobs::await_blocking_job,
};
use crate::models::SessionLaunchConfig;

use super::ai_jobs::{
    AiJobRunOptions, ai_job_cancelled, observation_summary, remote_command_observation,
    run_ai_ask_job,
};
use super::presentation::AiAgentStepKind;
use super::state::AiAgentObservationPoll;
use super::{AGENT_DEFAULT_STEP_TIMEOUT, AGENT_OBSERVATION_POLL_INTERVAL};

impl NyaTermApp {
    pub(in crate::features) fn upsert_ai_agent_step(
        &mut self,
        step_index: u16,
        status: AiAgentStepStatus,
        kind: AiAgentStepKind,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.ai
            .upsert_agent_step(step_index, status, kind, title, detail);
    }

    pub(in crate::features) fn toggle_ai_agent_thought_expanded(
        &mut self,
        step_index: u16,
        cx: &mut Context<Self>,
    ) {
        self.ai.toggle_agent_thought_expanded(step_index);
        self.defer_ai_panel_snapshot_flush(cx);
    }

    pub(in crate::features) fn toggle_ai_agent_output_expanded(
        &mut self,
        step_index: u16,
        cx: &mut Context<Self>,
    ) {
        self.ai.toggle_agent_output_expanded(step_index);
        self.defer_ai_panel_snapshot_flush(cx);
    }

    pub(in crate::features) fn record_ai_command_card_audit(
        &mut self,
        card: &AiCommandCard,
        execute: bool,
        inserted_to_terminal: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .ai
            .native_pending_call()
            .is_some_and(|call| call.id == card.id)
        {
            return;
        }
        let target_session_id = card.target_terminal_session_id.as_deref().or_else(|| {
            card.target
                .as_ref()
                .map(|target| target.terminal_session_id.as_str())
        });
        let request = AppendAiAuditRequest {
            connection_id: target_session_id
                .and_then(|id| self.session.metadata(id))
                .and_then(|metadata| metadata.source_connection_id.clone()),
            action: if execute {
                "ai.command_card_run".to_string()
            } else {
                "ai.command_card_insert".to_string()
            },
            user_input: Some(self.ai.chat_response_preview().to_string()),
            generated_command: Some(card.command.clone()),
            risk_level: card.risk_level.clone(),
            inserted_to_terminal,
            executed: execute,
            blocked: false,
            ..Default::default()
        };
        self.submit_ai_command_audit(request, cx);
    }

    fn submit_ai_command_audit(&mut self, request: AppendAiAuditRequest, cx: &mut Context<Self>) {
        let store = self.store_blocking_client();
        let write_lock = self.ai.history_audit_write_lock();
        let task = self.blocking_jobs.submit_task("ai-audit-save", move |_| {
            let _guard = write_lock
                .lock()
                .map_err(|_| "AI audit write lock poisoned".to_string())?;
            store
                .request_fn(StoreDomain::Ai, move |database| {
                    database.append_ai_audit(request)
                })
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            if let Err(error) = await_blocking_job(task).await.and_then(|result| result) {
                let _ = this.update(cx, |this, cx| {
                    this.settings
                        .update_store_status(format!("AI audit save failed: {error}"), false);
                    this.request_settings_panel_refresh(cx);
                    this.defer_ai_panel_snapshot_flush(cx);
                });
            } else {
                let _ = this.update(cx, |this, cx| {
                    this.refresh_ai_usage_counts(cx);
                });
            }
        })
        .detach();
    }

    pub(in crate::features) fn begin_ai_agent_observation(
        &mut self,
        card: &AiCommandCard,
        terminal_session_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<Option<String>, String> {
        let command = card.command.as_str();
        let before = self.ai_header_presentation();
        let terminal_session_id = terminal_session_id.to_string();
        if self.session.session_info(&terminal_session_id).is_none()
            || self.session.is_disconnected(&terminal_session_id)
        {
            return Err("AI Agent target terminal is unavailable".to_string());
        }
        let max_steps = self.ai.settings_max_agent_steps();
        let (_, step_index) = match self.ai.begin_agent_step(max_steps) {
            Ok(step) => step,
            Err(error) => {
                self.ai.set_panel_status(error);
                return Err(self.ai.panel_status().to_string());
            }
        };
        let now = Instant::now();
        let timeout = self
            .ai
            .settings_config()
            .agent_step_timeout_ms
            .map(|timeout| self.ai.native_step_timeout().unwrap_or(timeout))
            .or_else(|| self.ai.native_step_timeout())
            .map(Duration::from_millis)
            .unwrap_or(AGENT_DEFAULT_STEP_TIMEOUT);
        let profile = self.ai_execution_profile_for_session(&terminal_session_id);
        if profile == AiExecutionProfile::Disabled {
            return Err("AI Agent command execution is disabled for this session".to_string());
        }
        let marker_id = format!("agent-{}", uuid());
        let (marker_id, wrapped_command) =
            match build_agent_capture_command(profile, &marker_id, command.trim()) {
                Some(wrapped) => {
                    self.ai.register_agent_capture(marker_id.clone());
                    (Some(marker_id), Some(wrapped))
                }
                None => (None, None),
            };
        let available_target_ids = self.ai_effective_target_session_ids();
        let available_targets = self.ai_terminal_targets_for_sessions(&available_target_ids);
        self.ai.set_agent_loop(AiAgentLoopState {
            command_card_id: Some(card.id.clone()),
            ai_session_id: self.ai.chat_session_id().to_string(),
            terminal_session_id: terminal_session_id.clone(),
            available_targets,
            default_target_session_id: Some(terminal_session_id.clone()),
            command: command.trim().to_string(),
            marker_id,
            background_job_id: None,
            step_index,
            max_steps,
            started_at: now,
            timeout_at: now + timeout,
        });
        if profile == AiExecutionProfile::SendOnly {
            // Only a small initial hint is copied; command data comes from session events.
            let tail = self.terminal_buffer_tail_for_session(&terminal_session_id);
            let mut start = tail.len().saturating_sub(4096);
            while !tail.is_char_boundary(start) {
                start += 1;
            }
            let initial = tail[start..].to_string();
            self.ai.register_raw_capture(&initial, now);
        }
        self.ensure_ai_agent_loop_clock(cx);
        self.sync_session_event_bridge_policy();
        self.ai.set_panel_status(format!(
            "AI Agent observing command output for step {}/{}",
            step_index + 1,
            max_steps
        ));
        self.upsert_ai_agent_step(
            step_index,
            AiAgentStepStatus::Running,
            AiAgentStepKind::Command,
            "Running",
            truncate_preview(command.trim(), 140),
        );
        self.notify_root_if_ai_header_changed(before, cx);
        Ok(wrapped_command)
    }

    /// Shared by native/legacy Agent and external MCP execution. All of these
    /// paths open an exec channel rather than use the interactive terminal.
    pub(in crate::features) fn validate_ai_background_execution(
        &self,
        terminal_session_id: &str,
    ) -> Result<(), String> {
        let session = self
            .session
            .session_info(terminal_session_id)
            .ok_or_else(|| "Target terminal session is unavailable".to_string())?;
        let profile = self.ai_execution_profile_for_session(terminal_session_id);
        if profile == AiExecutionProfile::Disabled {
            return Err("AI Agent command execution is disabled for this session".to_string());
        }
        if profile == AiExecutionProfile::SendOnly
            || (session.kind == SessionKind::Ssh
                && !ssh_allows_background_execution(
                    profile,
                    self.ssh_session_is_configured_network_device(terminal_session_id),
                    self.terminal_buffer_tail_for_session(terminal_session_id),
                ))
        {
            return Err("AI Agent background execution is unavailable for raw CLI sessions; disable Background Execution to use the existing terminal".to_string());
        }
        Ok(())
    }

    pub(in crate::features) fn begin_ai_agent_background_execution(
        &mut self,
        card: &AiCommandCard,
        terminal_session_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let command = card.command.as_str();
        let before = self.ai_header_presentation();
        let terminal_session_id = terminal_session_id.to_string();
        let session = self
            .session
            .session_info(&terminal_session_id)
            .filter(|_| !self.session.is_disconnected(&terminal_session_id))
            .ok_or_else(|| "Active terminal session was not found".to_string())?;
        self.validate_ai_background_execution(&terminal_session_id)?;
        let (target, target_label) = match session.kind {
            SessionKind::Ssh => {
                let config = self
                    .session
                    .metadata(&terminal_session_id)
                    .and_then(|metadata| match &metadata.launch_config {
                        SessionLaunchConfig::Ssh(config) => Some(config.as_ref().clone()),
                        _ => None,
                    })
                    .or_else(|| {
                        (self.session.active_id() == Some(terminal_session_id.as_str()))
                            .then(|| self.session.active_ssh_config_owned())
                            .flatten()
                    })
                    .ok_or_else(|| "Target SSH session is missing its exec config".to_string())?;
                (AiAgentBackgroundTarget::Ssh(Box::new(config)), "SSH")
            }
            SessionKind::LocalPty => (
                AiAgentBackgroundTarget::Local {
                    working_dir: session.working_dir.clone(),
                },
                "local",
            ),
            SessionKind::Telnet
            | SessionKind::RawTcp
            | SessionKind::Serial
            | SessionKind::Rdp
            | SessionKind::Vnc => {
                return Err(format!(
                    "AI Agent background execution is not supported for {:?} sessions",
                    session.kind
                ));
            }
        };
        let max_steps = self.ai.settings_max_agent_steps();
        let (_, step_index) = self.ai.begin_agent_step(max_steps)?;
        let now = Instant::now();
        let timeout = self
            .ai
            .settings_config()
            .agent_step_timeout_ms
            .map(|timeout| self.ai.native_step_timeout().unwrap_or(timeout))
            .or_else(|| self.ai.native_step_timeout())
            .map(Duration::from_millis)
            .unwrap_or(AGENT_DEFAULT_STEP_TIMEOUT);
        let launch = self.ai.begin_chat_job();
        let job_id = launch.job_id;
        let cancel = launch.cancel;
        let available_target_ids = self.ai_effective_target_session_ids();
        let available_targets = self.ai_terminal_targets_for_sessions(&available_target_ids);
        let state = AiAgentLoopState {
            command_card_id: Some(card.id.clone()),
            ai_session_id: self.ai.chat_session_id().to_string(),
            terminal_session_id: terminal_session_id.clone(),
            available_targets,
            default_target_session_id: Some(terminal_session_id),
            command: command.trim().to_string(),
            marker_id: None,
            background_job_id: Some(job_id),
            step_index,
            max_steps,
            started_at: now,
            timeout_at: now + timeout,
        };
        self.ai.set_agent_loop(state.clone());
        self.ensure_ai_agent_loop_clock(cx);
        self.ai.set_panel_status(format!(
            "AI Agent running {target_label} background command for step {}/{}",
            step_index + 1,
            max_steps
        ));
        self.upsert_ai_agent_step(
            step_index,
            AiAgentStepStatus::Running,
            AiAgentStepKind::Command,
            format!("{target_label} background"),
            truncate_preview(command.trim(), 140),
        );
        let tx = launch.tx;
        let command = state.command.clone();
        let rejected_tx = tx.clone();
        let rejected_state = state.clone();
        if let Err(error) = self.blocking_jobs.submit_detached(
            "ai-agent-background-command",
            move |scheduler_cancel| {
                let started = Instant::now();
                let result = if scheduler_cancel.is_cancelled() || ai_job_cancelled(&cancel) {
                    Err("AI Agent background command cancelled".to_string())
                } else {
                    match target {
                        AiAgentBackgroundTarget::Ssh(config) => SshProcessService::new(*config)
                            .run_command(&command, timeout)
                            .map(|output| remote_command_observation(output, started))
                            .map_err(|error| error.to_string()),
                        AiAgentBackgroundTarget::Local { working_dir } => {
                            run_local_command(&command, working_dir, timeout)
                                .map(|output| remote_command_observation(output, started))
                                .map_err(|error| error.to_string())
                        }
                    }
                };
                if !scheduler_cancel.is_cancelled() && !ai_job_cancelled(&cancel) {
                    let _ = tx.unbounded_send(AiChatWorkerEvent::AgentBackgroundFinished {
                        job_id,
                        state,
                        result,
                    });
                }
            },
        ) {
            let _ = rejected_tx.unbounded_send(AiChatWorkerEvent::AgentBackgroundFinished {
                job_id,
                state: rejected_state,
                result: Err(error.to_string()),
            });
        }
        self.defer_ai_panel_snapshot_flush(cx);
        self.notify_root_if_ai_header_changed(before, cx);
        Ok(())
    }

    /// Poll only while observing a command: prompt settlement and deadlines
    /// require a clock even when no further session event arrives.
    pub(in crate::features) fn ensure_ai_agent_loop_clock(&mut self, cx: &mut Context<Self>) {
        if self.ai.agent_loop_clock_is_armed() || self.ai.agent_loop_snapshot().is_none() {
            return;
        }
        let scope = self.ai.active_scope_key().to_string();
        self.ai.set_agent_loop_clock_armed(true);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(AGENT_OBSERVATION_POLL_INTERVAL)
                    .await;
                let Ok(keep_running) = this.update(cx, |this, cx| {
                    let before = this.ai_header_presentation();
                    let visible_scope = this.ai.active_scope_key().to_string();
                    this.ai.switch_scope(&scope);
                    if this.drive_ai_agent_loop(cx) {
                        this.defer_ai_panel_snapshot_flush(cx);
                    }
                    let running = this.ai.agent_loop_snapshot().is_some();
                    if !running {
                        this.ai.set_agent_loop_clock_armed(false);
                    }
                    this.ai.switch_scope(&visible_scope);
                    this.notify_root_if_ai_header_changed(before, cx);
                    running
                }) else {
                    break;
                };
                if !keep_running {
                    break;
                }
            }
        })
        .detach();
    }

    pub(in crate::features) fn drive_ai_agent_loop(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(snapshot) = self.ai.agent_loop_snapshot() else {
            return false;
        };
        if self.ai.chat_is_pending() || snapshot.background_job_id.is_some() {
            return false;
        }
        let terminal_session_id = snapshot.terminal_session_id.clone();
        if self.session.session_info(&terminal_session_id).is_none()
            || self.session.is_disconnected(&terminal_session_id)
        {
            self.ai.stop_agent_for_closed_target();
            let _ = cx;
            return true;
        }
        let now = Instant::now();
        match self.ai.poll_agent_observation(now) {
            AiAgentObservationPoll::Waiting => false,
            AiAgentObservationPoll::Raw(state, mut output, end) => {
                self.sync_session_event_bridge_policy();
                output.push_str("\n(");
                output.push_str(end.description());
                output.push(')');
                let observation = CommandObservation {
                    output,
                    exit_code: None,
                    duration_ms: now
                        .duration_since(state.started_at)
                        .as_millis()
                        .try_into()
                        .unwrap_or(u64::MAX),
                };
                if end != RawCliEnd::Prompt {
                    // Do not automatically send another command into a pager,
                    // confirmation prompt or a command that may still be running.
                    self.ai
                        .complete_native_terminal(Err(nyaterm_mcp_protocol::RpcError {
                            code: "completion_unconfirmed".into(),
                            message: observation.output.clone(),
                        }));
                    self.cancel_ai_chat(cx);
                    self.ai.set_panel_status(end.description());
                }
                self.ai.record_agent_observation(
                    state.step_index,
                    &observation,
                    observation_summary(&observation),
                );
                if end == RawCliEnd::Prompt {
                    if !self.complete_native_terminal_observation(&observation, false, false) {
                        self.start_ai_agent_continuation(state, observation, cx);
                    }
                } else {
                    self.upsert_ai_agent_step(
                        state.step_index,
                        AiAgentStepStatus::Failed,
                        AiAgentStepKind::Diagnostic,
                        "Completion unconfirmed",
                        end.description(),
                    );
                }
                true
            }
            AiAgentObservationPoll::TimedOut(state) => {
                self.sync_session_event_bridge_policy();
                let duration_ms = now
                    .duration_since(state.started_at)
                    .as_millis()
                    .try_into()
                    .unwrap_or(u64::MAX);
                let observation = CommandObservation {
                    output:
                        "(command timed out; capture markers were not detected in terminal output)"
                            .to_string(),
                    exit_code: None,
                    duration_ms,
                };
                self.upsert_ai_agent_step(
                    state.step_index,
                    AiAgentStepStatus::Failed,
                    AiAgentStepKind::Diagnostic,
                    "Timed out",
                    observation_summary(&observation),
                );
                if !self.complete_native_terminal_observation(&observation, true, false) {
                    self.start_ai_agent_continuation(state, observation, cx);
                }
                true
            }
        }
    }

    pub(in crate::features) fn handle_ai_agent_captured_output(
        &mut self,
        captured: AgentCapturedOutput,
        cx: &mut Context<Self>,
    ) {
        let Some(scope) = self.ai.scope_for_agent_marker(&captured.marker_id) else {
            return;
        };
        let visible_scope = self.ai.active_scope_key().to_string();
        self.ai.switch_scope(&scope);
        let Some(state) = self.ai.take_agent_loop_for_marker(&captured.marker_id) else {
            self.ai.switch_scope(&visible_scope);
            return;
        };
        let observation = CommandObservation {
            output: captured.output,
            exit_code: captured.exit_code,
            duration_ms: captured.duration_ms,
        };
        self.ai.set_panel_status(match observation.exit_code {
            Some(code) => format!("AI Agent captured command output with exit code {code}"),
            None => "AI Agent captured command output".to_string(),
        });
        self.ai.record_agent_observation(
            state.step_index,
            &observation,
            observation_summary(&observation),
        );
        if !self.complete_native_terminal_observation(&observation, false, false) {
            self.start_ai_agent_continuation(state, observation, cx);
        }
        self.defer_ai_panel_snapshot_flush(cx);
        self.ai.switch_scope(&visible_scope);
    }

    pub(in crate::features) fn note_ai_agent_output_discontinuity(
        &mut self,
        session_id: &str,
        dropped_bytes: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(scope) = self.ai.scope_for_agent_target(session_id) else {
            return false;
        };
        let visible_scope = self.ai.active_scope_key().to_string();
        self.ai.switch_scope(&scope);
        let Some((state, raw_output)) = self.ai.take_agent_loop_for_session(session_id) else {
            self.ai.switch_scope(&visible_scope);
            return false;
        };
        self.sync_session_event_bridge_policy();
        let duration_ms = state
            .started_at
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        let mut output = raw_output.unwrap_or_default();
        output.push_str(&format!(
            "\n(terminal output dropped {dropped_bytes} byte(s); command output is incomplete)"
        ));
        let observation = CommandObservation {
            output,
            exit_code: None,
            duration_ms,
        };
        self.ai.set_panel_status(
            "AI Agent command observation stopped because terminal output was dropped".to_string(),
        );
        self.upsert_ai_agent_step(
            state.step_index,
            AiAgentStepStatus::Failed,
            AiAgentStepKind::Diagnostic,
            "Output dropped",
            observation_summary(&observation),
        );
        if state.marker_id.is_none() {
            self.ai
                .complete_native_terminal(Err(nyaterm_mcp_protocol::RpcError {
                    code: "output_incomplete".into(),
                    message: observation.output.clone(),
                }));
            self.cancel_ai_chat(cx);
            self.ai.set_panel_status(
                "AI Agent stopped because command output was lost; review the terminal",
            );
            self.upsert_ai_agent_step(
                state.step_index,
                AiAgentStepStatus::Failed,
                AiAgentStepKind::Diagnostic,
                "Output dropped",
                observation_summary(&observation),
            );
        } else if !self.complete_native_terminal_observation(&observation, false, true) {
            self.start_ai_agent_continuation(state, observation, cx);
        }
        self.defer_ai_panel_snapshot_flush(cx);
        self.ai.switch_scope(&visible_scope);
        true
    }

    fn ai_execution_profile_for_session(&self, session_id: &str) -> AiExecutionProfile {
        let configured = if self.session.active_id() == Some(session_id) {
            self.session.active_ai_execution_profile()
        } else {
            self.session
                .metadata(session_id)
                .map(|metadata| metadata.ai_execution_profile)
                .unwrap_or(AiExecutionProfile::SendOnly)
        };
        if configured != AiExecutionProfile::Auto {
            return configured;
        }
        self.session
            .session_info(session_id)
            .filter(|_| !self.session.is_disconnected(session_id))
            .map(|session| match session.kind {
                SessionKind::Ssh => resolve_ssh_execution_profile(
                    configured,
                    self.ssh_session_is_configured_network_device(session_id),
                    self.terminal_buffer_tail_for_session(session_id),
                ),
                SessionKind::LocalPty | SessionKind::Telnet | SessionKind::RawTcp => {
                    AiExecutionProfile::Posix
                }
                SessionKind::Serial | SessionKind::Rdp | SessionKind::Vnc => {
                    AiExecutionProfile::SendOnly
                }
            })
            .unwrap_or(AiExecutionProfile::SendOnly)
    }

    fn ssh_session_is_configured_network_device(&self, session_id: &str) -> bool {
        self.session.metadata(session_id).is_some_and(|metadata| {
            matches!(&metadata.launch_config, SessionLaunchConfig::Ssh(config) if config.is_network_device())
        }) || (self.session.active_id() == Some(session_id)
            && self.session.active_ssh_config().is_some_and(|config| config.is_network_device()))
    }

    pub(in crate::features) fn start_ai_agent_continuation(
        &mut self,
        state: AiAgentLoopState,
        observation: CommandObservation,
        cx: &mut Context<Self>,
    ) {
        if self.complete_native_terminal_observation(&observation, false, false) {
            return;
        }
        let observation_message = build_observation_message(
            &observation,
            &state.command,
            &self.settings.summary().language,
        );
        self.start_ai_agent_continuation_with_message(state, observation_message, cx);
    }

    pub(in crate::features) fn start_ai_agent_continuation_with_message(
        &mut self,
        state: AiAgentLoopState,
        observation_message: String,
        cx: &mut Context<Self>,
    ) {
        if self
            .ai
            .complete_native_terminal(Err(nyaterm_mcp_protocol::RpcError {
                code: "execution_failed".into(),
                message: observation_message.clone(),
            }))
        {
            return;
        }
        let conversation = self.ai.agent_conversation_snapshot();
        let Some(launch) = self
            .ai
            .begin_agent_continuation(&state, &observation_message)
        else {
            return;
        };
        let settings = self.ai.settings_config_cloned();
        let terminal_session_id = state.terminal_session_id.clone();
        let owner_terminal_id = self
            .ai
            .active_scope_key()
            .strip_prefix("terminal:")
            .map(str::to_string);
        let owner_connection_id = owner_terminal_id
            .as_deref()
            .and_then(|id| self.session.metadata(id))
            .and_then(|metadata| metadata.source_connection_id.clone());
        let context = self.ai_terminal_context_for_session(Some(&terminal_session_id));
        let targets = state.available_targets.clone();
        let target_contexts = targets
            .iter()
            .cloned()
            .map(|target| nyaterm_core::AiTargetContext {
                context: self.ai_terminal_context_for_session(Some(&target.terminal_session_id)),
                target: Some(target),
            })
            .collect();
        let request = AiChatRequest {
            stream_id: None,
            session_id: Some(state.ai_session_id.clone()),
            connection_id: owner_connection_id.clone(),
            terminal_session_id: Some(terminal_session_id.clone()),
            owner_scope: nyaterm_core::AiSessionScope {
                r#type: nyaterm_core::AiSessionScopeType::Terminal,
                target_id: owner_terminal_id.clone(),
                connection_ids: owner_connection_id.into_iter().collect(),
                label: owner_terminal_id
                    .as_deref()
                    .and_then(|id| self.session.display_name(id)),
            },
            targets,
            target_contexts,
            mode: AiMode::Agent,
            agent_kind: self.ai.chat_agent_kind(),
            permission_mode: settings.external_agent_permission_mode.clone(),
            model_id: self.ai_selected_model_id(),
            model_name: None,
            default_target_session_id: state.default_target_session_id.clone(),
            existing_external_session_id: None,
            attachments: Vec::new(),
            action: AiAction::GenerateCommand,
            user_input: observation_message,
            context,
            options: nyaterm_core::AiRequestOptions {
                history_turns: state.max_steps.saturating_mul(2).saturating_add(2),
                agent_json_protocol: self.ai.agent_uses_json_protocol(),
                ..Default::default()
            },
        };
        let store = self.store_blocking_client();
        let tx = launch.tx;
        let session_id = launch.session_id;
        let job_id = launch.job_id;
        let cancel = launch.cancel;
        let rejected_tx = tx.clone();
        let rejected_session_id = session_id.clone();
        if let Err(error) =
            self.blocking_jobs
                .submit_detached("ai-agent-continuation", move |scheduler_cancel| {
                    let result = if scheduler_cancel.is_cancelled() {
                        Err("AI Agent continuation cancelled".to_string())
                    } else {
                        run_ai_ask_job(
                            store,
                            settings,
                            request,
                            AiJobRunOptions {
                                mcp_credential: None,
                                stream_tx: Some(tx.clone()),
                                cancel,
                                job_id,
                                agent_history: Some(conversation),
                                persist_user_message: false,
                            },
                        )
                    };
                    let _ = tx.unbounded_send(AiChatWorkerEvent::Finished(AiChatJobResult {
                        job_id,
                        session_id,
                        result,
                    }));
                })
        {
            let _ = rejected_tx.unbounded_send(AiChatWorkerEvent::Finished(AiChatJobResult {
                job_id,
                session_id: rejected_session_id,
                result: Err(error.to_string()),
            }));
        }
        self.defer_ai_panel_snapshot_flush(cx);
    }

    pub(in crate::features) fn reject_ai_agent_command_card_by_id(
        &mut self,
        card_id: String,
        cx: &mut Context<Self>,
    ) {
        if self
            .ai
            .native_pending_call()
            .is_some_and(|call| call.id == card_id)
        {
            self.respond_to_mcp_approval(
                &format!(
                    "{}:{card_id}",
                    self.ai.native_owner().expect("native owner")
                ),
                crate::features::mcp::McpApprovalDecision::Deny,
                cx,
            );
            return;
        }
        if let Some(card) = self.ai.find_command_card(&card_id) {
            self.reject_ai_agent_command_card_value(card, cx);
        }
    }

    fn reject_ai_agent_command_card_value(&mut self, card: AiCommandCard, cx: &mut Context<Self>) {
        if !super::ai_jobs::is_agent_command_card(&card)
            || !self.ai.current_agent_command_card(&card.id)
            || !self
                .ai
                .chat_command_cards()
                .iter()
                .any(|current| current.id == card.id)
            || self.ai.chat_is_pending()
            || self.ai.agent_loop_snapshot().is_some()
        {
            return;
        }
        let Some(terminal_session_id) = card.target_terminal_session_id.clone() else {
            return;
        };
        let max_steps = self.ai.settings_max_agent_steps();
        let Ok((_, step_index)) = self.ai.begin_agent_step(max_steps) else {
            self.ai
                .set_panel_status("AI Agent reached its step limit".to_string());
            self.defer_ai_panel_snapshot_flush(cx);
            return;
        };
        self.ai.upsert_agent_step(
            step_index,
            AiAgentStepStatus::Rejected,
            AiAgentStepKind::Diagnostic,
            "Rejected",
            "User denied command execution",
        );
        self.ai.associate_agent_command(step_index, &card.id);
        self.ai.clear_chat_command_cards();
        self.submit_ai_command_audit(
            AppendAiAuditRequest {
                connection_id: self
                    .session
                    .metadata(&terminal_session_id)
                    .and_then(|metadata| metadata.source_connection_id.clone()),
                action: "ai.agent_reject_execute".to_string(),
                generated_command: Some(card.command.clone()),
                risk_level: card.risk_level.clone(),
                blocked: true,
                session_id: Some(self.ai.chat_session_id().to_string()),
                approval_decision: Some(AgentApprovalOutcome::Rejected.as_str().to_string()),
                ..Default::default()
            },
            cx,
        );
        let now = Instant::now();
        let target_ids = self.ai_effective_target_session_ids();
        let state = AiAgentLoopState {
            command_card_id: Some(card.id.clone()),
            ai_session_id: self.ai.chat_session_id().to_string(),
            terminal_session_id: terminal_session_id.clone(),
            available_targets: self.ai_terminal_targets_for_sessions(&target_ids),
            default_target_session_id: Some(terminal_session_id),
            command: card.command.clone(),
            marker_id: None,
            background_job_id: None,
            step_index,
            max_steps,
            started_at: now,
            timeout_at: now,
        };
        self.start_ai_agent_continuation_with_message(
            state,
            format!("The user rejected command `{}`. It was not executed. Choose a safe alternative or finish the task.", card.command),
            cx,
        );
    }
}
