use std::collections::{HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use nyaterm_mcp_protocol::MCP_TOOL_REGISTRY;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, RwLock, oneshot};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::config::{AiAgentKind, AiBackendKind, AiModelSource, AiSettings, CodexThreadMode};
use crate::core::SessionManager;
use crate::core::mcp::{McpTurnGuard, SessionMcpCredential};
use crate::error::{AppError, AppResult};
use crate::utils::process::hide_window;

use super::agent::{AgentApprovalManager, run_external_agent_command_step};
use super::history::{
    append_ai_audit, append_message, get_session_backend_metadata, save_user_message,
    set_session_backend_metadata, set_session_external_session_id,
};
use super::prompt::build_agent_prompt;
use super::redaction::{redact_marker_values, redact_request, redact_sensitive_text};
use super::stream::{active_streams, emit_stream_event};
use super::types::{
    AiChatRequest, AiMessage, AiMessageRole, AiModelDiscovery, AiSessionBackendMetadata,
    AiStreamEventPayload, AppendAiAuditRequest, CommandObservation, now_rfc3339, uuid,
};

const CODEX_TERMINAL_TOOLS_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexRuntimeState {
    Stopped,
    Starting,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexCliStatus {
    pub installed: bool,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub checked_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountStatus {
    pub connected: bool,
    #[serde(default)]
    pub auth_mode: Option<String>,
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub requires_openai_auth: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLoginStart {
    pub login_id: Option<String>,
    pub login_type: String,
    #[serde(default)]
    pub auth_url: Option<String>,
    #[serde(default)]
    pub verification_url: Option<String>,
    #[serde(default)]
    pub user_code: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CodexLoginFlow {
    Browser,
    DeviceCode,
}

pub struct CodexAppServerManager {
    state: RwLock<CodexRuntimeState>,
    writer: Mutex<Option<BufWriter<ChildStdin>>>,
    child: Mutex<Option<Child>>,
    pending: StdMutex<HashMap<u64, oneshot::Sender<AppResult<Value>>>>,
    active_turns: StdMutex<HashMap<String, Arc<CodexTurnContext>>>,
    starting_turns: StdMutex<HashMap<String, Arc<CodexTurnContext>>>,
    next_request_id: AtomicU64,
    last_account: Mutex<Option<CodexAccountStatus>>,
    sessions: StdMutex<HashMap<String, Arc<CodexSession>>>,
    epoch: AtomicU64,
    runtime_cancel: StdMutex<CancellationToken>,
    #[cfg(test)]
    test_events: StdMutex<Option<tokio::sync::mpsc::UnboundedSender<(String, Value)>>>,
}

#[derive(Default)]
struct CodexSession {
    thread: Mutex<Option<LoadedCodexThread>>,
    credential: StdMutex<Option<Arc<SessionMcpCredential>>>,
    cancellation: CancellationToken,
}

struct LoadedCodexThread {
    id: String,
    epoch: u64,
    ephemeral: bool,
    mcp: bool,
}

impl CodexSession {
    fn close_credential(&self) {
        if let Some(credential) = self.credential.lock().unwrap().take() {
            credential.close();
        }
    }

    fn close(&self) {
        self.cancellation.cancel();
        self.close_credential();
    }
}

struct CodexRequestGuard<'a> {
    manager: &'a CodexAppServerManager,
    id: u64,
}
impl Drop for CodexRequestGuard<'_> {
    fn drop(&mut self) {
        self.manager.pending.lock().unwrap().remove(&self.id);
    }
}

struct CodexTurnRegistration<'a> {
    manager: &'a CodexAppServerManager,
    thread_id: &'a str,
    turn_id: Option<String>,
}
impl Drop for CodexTurnRegistration<'_> {
    fn drop(&mut self) {
        if let Some(id) = &self.turn_id {
            self.manager.active_turns.lock().unwrap().remove(id);
        }
        self.manager
            .starting_turns
            .lock()
            .unwrap()
            .remove(self.thread_id);
    }
}

struct CodexCompletionGuard(CancellationToken);
impl Drop for CodexCompletionGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct CodexTurnContext {
    completed: CancellationToken,
    mcp_turn: StdMutex<Option<McpTurnGuard>>,
    app: AppHandle,
    session_manager: Arc<SessionManager>,
    approval_manager: Arc<AgentApprovalManager>,
    stream_id: String,
    session_id: String,
    request: AiChatRequest,
    settings: AiSettings,
    step_counter: Mutex<u16>,
    dynamic_tool_call_count: AtomicU64,
    text_accumulator: Mutex<String>,
}

impl CodexAppServerManager {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(CodexRuntimeState::Stopped),
            writer: Mutex::new(None),
            child: Mutex::new(None),
            pending: StdMutex::new(HashMap::new()),
            active_turns: StdMutex::new(HashMap::new()),
            starting_turns: StdMutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            last_account: Mutex::new(None),
            sessions: StdMutex::new(HashMap::new()),
            epoch: AtomicU64::new(0),
            runtime_cancel: StdMutex::new(CancellationToken::new()),
            #[cfg(test)]
            test_events: StdMutex::new(None),
        }
    }

    fn session(&self, id: &str) -> Arc<CodexSession> {
        self.sessions
            .lock()
            .unwrap()
            .entry(id.to_owned())
            .or_default()
            .clone()
    }

    pub async fn release_session(&self, id: &str) {
        let session = self.sessions.lock().unwrap().remove(id);
        if let Some(session) = session {
            session.close();
            let _finished = session.thread.lock().await;
        }
    }

    pub async fn release_all_sessions(&self) {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .drain()
            .map(|(_, session)| session)
            .collect();
        for session in &sessions {
            session.close();
        }
        for session in sessions {
            let _finished = session.thread.lock().await;
        }
    }

    pub async fn detect_cli(path: Option<String>) -> CodexCliStatus {
        let candidates = discover_codex_candidates(path.as_deref()).await;
        let checked_paths = candidates
            .iter()
            .map(|candidate| candidate.executable.clone())
            .collect::<Vec<_>>();
        let mut errors = Vec::new();

        for candidate in &candidates {
            match probe_codex_cli(&candidate.executable).await {
                Ok(version) => {
                    return CodexCliStatus {
                        installed: true,
                        path: Some(candidate.executable.clone()),
                        version: Some(version),
                        error: None,
                        source: Some(candidate.source.to_string()),
                        checked_paths,
                    };
                }
                Err(error) => {
                    errors.push(format!("{}: {error}", candidate.executable));
                }
            }
        }

        CodexCliStatus {
            installed: false,
            path: path
                .as_deref()
                .map(|value| codex_executable(Some(value)))
                .or_else(|| Some("codex".to_string())),
            version: None,
            error: Some(detect_error_message(&errors)),
            source: None,
            checked_paths,
        }
    }

    pub async fn ensure_started(self: &Arc<Self>, path: Option<String>) -> AppResult<()> {
        if *self.state.read().await == CodexRuntimeState::Ready {
            return Ok(());
        }

        let mut child_guard = self.child.lock().await;
        if *self.state.read().await == CodexRuntimeState::Ready {
            return Ok(());
        }

        *self.state.write().await = CodexRuntimeState::Starting;
        let executable = resolve_codex_executable(path).await?;
        let mut command = Command::new(&executable);
        hide_window(&mut command);
        let mut child = command
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                AppError::Config(format!("Failed to start codex app-server: {error}"))
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Channel("codex stdin unavailable".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Channel("codex stdout unavailable".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Channel("codex stderr unavailable".to_string()))?;

        *self.writer.lock().await = Some(BufWriter::new(stdin));
        *child_guard = Some(child);
        self.epoch.fetch_add(1, Ordering::SeqCst);
        *self.runtime_cancel.lock().unwrap() = CancellationToken::new();

        let reader_manager = self.clone();
        tauri::async_runtime::spawn(async move {
            reader_manager.read_stdout(stdout).await;
        });
        tauri::async_runtime::spawn(async move {
            read_stderr(stderr).await;
        });

        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "nyaterm",
                    "title": "NyaTerm",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true
                }
            }),
        )
        .await?;
        self.notify("initialized", json!({})).await?;
        *self.state.write().await = CodexRuntimeState::Ready;
        Ok(())
    }

    pub async fn account_read(
        self: &Arc<Self>,
        settings: &AiSettings,
    ) -> AppResult<CodexAccountStatus> {
        self.ensure_started(settings.codex.executable_path.clone())
            .await?;
        let value = self
            .request("account/read", json!({ "refreshToken": false }))
            .await?;
        let status = parse_account_status(&value);
        *self.last_account.lock().await = Some(status.clone());
        Ok(status)
    }

    pub async fn login_start(
        self: &Arc<Self>,
        settings: &AiSettings,
        flow: CodexLoginFlow,
    ) -> AppResult<CodexLoginStart> {
        self.ensure_started(settings.codex.executable_path.clone())
            .await?;
        let params = match flow {
            CodexLoginFlow::Browser => json!({
                "type": "chatgpt",
                "useHostedLoginSuccessPage": true,
                "appBrand": "codex"
            }),
            CodexLoginFlow::DeviceCode => json!({ "type": "chatgptDeviceCode" }),
        };
        let value = self.request("account/login/start", params).await?;
        Ok(CodexLoginStart {
            login_id: value
                .get("loginId")
                .and_then(Value::as_str)
                .map(str::to_string),
            login_type: value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("chatgpt")
                .to_string(),
            auth_url: value
                .get("authUrl")
                .and_then(Value::as_str)
                .map(str::to_string),
            verification_url: value
                .get("verificationUrl")
                .and_then(Value::as_str)
                .map(str::to_string),
            user_code: value
                .get("userCode")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    pub async fn login_cancel(
        self: &Arc<Self>,
        settings: &AiSettings,
        login_id: String,
    ) -> AppResult<()> {
        self.ensure_started(settings.codex.executable_path.clone())
            .await?;
        self.request("account/login/cancel", json!({ "loginId": login_id }))
            .await?;
        Ok(())
    }

    pub async fn logout(self: &Arc<Self>, settings: &AiSettings) -> AppResult<()> {
        self.ensure_started(settings.codex.executable_path.clone())
            .await?;
        self.request("account/logout", json!({})).await?;
        *self.last_account.lock().await = Some(CodexAccountStatus::default());
        Ok(())
    }

    pub async fn list_models(
        self: &Arc<Self>,
        settings: &AiSettings,
    ) -> AppResult<Vec<AiModelDiscovery>> {
        if !settings.codex.enabled {
            return Ok(Vec::new());
        }
        self.ensure_started(settings.codex.executable_path.clone())
            .await?;
        let value = self
            .request("model/list", json!({ "includeHidden": false }))
            .await?;
        let items = value
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(items
            .into_iter()
            .filter_map(|item| {
                let model = item
                    .get("model")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)?;
                Some(AiModelDiscovery {
                    id: format!("codex:{model}"),
                    name: model.to_string(),
                    backend: AiBackendKind::Codex,
                    provider_kind: None,
                    credential_id: None,
                    source: AiModelSource::RustGenai,
                })
            })
            .collect())
    }

    async fn request(&self, method: &str, params: Value) -> AppResult<Value> {
        let id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let _registration = CodexRequestGuard { manager: self, id };
        self.write_message(json!({ "method": method, "id": id, "params": params }))
            .await?;
        timeout(Duration::from_secs(30), rx)
            .await
            .map_err(|_| AppError::Channel(format!("Codex {method} timed out")))?
            .map_err(|_| AppError::Channel("codex response channel closed".to_string()))?
    }

    async fn notify(&self, method: &str, params: Value) -> AppResult<()> {
        self.write_message(json!({ "method": method, "params": params }))
            .await
    }

    async fn write_message(&self, value: Value) -> AppResult<()> {
        let mut writer = self.writer.lock().await;
        let Some(writer) = writer.as_mut() else {
            return Err(AppError::Channel(
                "codex app-server is not started".to_string(),
            ));
        };
        let line = serde_json::to_string(&value)?;
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(())
    }

    async fn read_stdout(self: Arc<Self>, stdout: tokio::process::ChildStdout) {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                tracing::warn!("Ignoring invalid codex app-server JSONL line");
                continue;
            };
            self.handle_message(value).await;
        }
        self.runtime_cancel.lock().unwrap().cancel();
        for session in self.sessions.lock().unwrap().values() {
            session.close_credential();
        }
        {
            let mut pending = self.pending.lock().unwrap();
            for (_, sender) in pending.drain() {
                let _ = sender.send(Err(AppError::Channel(
                    "codex app-server exited".to_string(),
                )));
            }
        }
        *self.state.write().await = CodexRuntimeState::Failed;
    }

    async fn handle_message(&self, value: Value) {
        if let Some(id) = value.get("id").and_then(Value::as_u64) {
            if value.get("method").is_some() {
                self.handle_server_request(id, value).await;
                return;
            }
            let sender = self.pending.lock().unwrap().remove(&id);
            if let Some(sender) = sender {
                let result = if let Some(error) = value.get("error") {
                    let message = error
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| error.to_string());
                    Err(AppError::Config(format!(
                        "Codex app-server error: {message}"
                    )))
                } else {
                    Ok(value.get("result").cloned().unwrap_or(Value::Null))
                };
                let _ = sender.send(result);
            }
            return;
        }

        if let Some(method) = value.get("method").and_then(Value::as_str) {
            self.handle_notification(method, value.get("params").cloned().unwrap_or(Value::Null))
                .await;
        }
    }

    async fn handle_server_request(&self, id: u64, value: Value) {
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        let result = match method.as_str() {
            "item/tool/call" => self.handle_dynamic_tool_call(params).await,
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                Ok(json!({ "decision": "decline" }))
            }
            "item/permissions/requestApproval" => Ok(json!({ "decision": "decline" })),
            "mcpServer/elicitation/request" => Ok(json!({ "action": "decline", "content": null })),
            _ => Ok(json!({})),
        };

        let response = match result {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(error) => json!({
                "id": id,
                "error": { "code": -32000, "message": error.to_string() }
            }),
        };
        let _ = self.write_message(response).await;
    }

    async fn handle_dynamic_tool_call(&self, params: Value) -> AppResult<Value> {
        let turn_id = params
            .get("turnId")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Config("Codex tool call missing turnId".to_string()))?;
        let namespace = params
            .get("namespace")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tool = params
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let context = self
            .turn_context(turn_id, &params)
            .ok_or_else(|| AppError::Config("No active Codex turn for tool call".to_string()))?;
        context
            .dynamic_tool_call_count
            .fetch_add(1, Ordering::SeqCst);

        if namespace != "nyaterm_terminal" {
            return Ok(dynamic_text(false, "Unsupported dynamic tool namespace"));
        }

        match tool {
            "get_context" => Ok(dynamic_text(true, &terminal_context_text(&context.request))),
            "execute_command" => {
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                let command = args
                    .get("command")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| AppError::Config("terminal command is required".to_string()))?
                    .to_string();
                let target = args
                    .get("targetTerminalSessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let reason = args
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let step_index = {
                    let mut counter = context.step_counter.lock().await;
                    *counter = counter.saturating_add(1);
                    *counter
                };
                match run_external_agent_command_step(
                    &context.app,
                    context.session_manager.clone(),
                    context.approval_manager.clone(),
                    &context.stream_id,
                    &context.session_id,
                    &context.request,
                    &context.settings,
                    step_index,
                    command,
                    reason,
                    target,
                )
                .await
                {
                    Ok(observation) => Ok(dynamic_text(true, &observation_text(&observation))),
                    Err(error) => Ok(dynamic_text(false, &error.to_string())),
                }
            }
            _ => Ok(dynamic_text(false, "Unsupported dynamic tool")),
        }
    }

    fn turn_context(&self, turn_id: &str, params: &Value) -> Option<Arc<CodexTurnContext>> {
        let active = self.active_turns.lock().unwrap();
        active.get(turn_id).cloned().or_else(|| {
            let thread_id = params.get("threadId")?.as_str()?;
            self.starting_turns.lock().unwrap().get(thread_id).cloned()
        })
    }

    async fn handle_notification(&self, method: &str, params: Value) {
        #[cfg(test)]
        if let Some(events) = self.test_events.lock().unwrap().as_ref() {
            let _ = events.send((method.to_string(), params.clone()));
        }
        match method {
            "account/updated" => {
                let status = CodexAccountStatus {
                    connected: params
                        .get("authMode")
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.is_empty()),
                    auth_mode: params
                        .get("authMode")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    plan_type: params
                        .get("planType")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    email: None,
                    requires_openai_auth: true,
                };
                *self.last_account.lock().await = Some(status);
            }
            "item/agentMessage/delta" => {
                if let Some(turn_id) = params.get("turnId").and_then(Value::as_str) {
                    let context = self.turn_context(turn_id, &params);
                    if let Some(context) = context
                        && let Some(delta) = params.get("delta").and_then(Value::as_str)
                        && !delta.is_empty()
                    {
                        context.text_accumulator.lock().await.push_str(delta);
                        emit_stream_event(
                            &context.app,
                            &context.stream_id,
                            AiStreamEventPayload {
                                event_type: "delta".to_string(),
                                stream_id: context.stream_id.clone(),
                                session_id: Some(context.session_id.clone()),
                                text_delta: Some(delta.to_string()),
                                reasoning_delta: None,
                                message: None,
                                command_cards: vec![],
                                usage: None,
                                error: None,
                            },
                        );
                    }
                }
            }
            "item/reasoning/delta" | "item/reasoning/summaryDelta" => {
                if let Some(turn_id) = params.get("turnId").and_then(Value::as_str) {
                    let context = self.turn_context(turn_id, &params);
                    if let Some(context) = context {
                        let delta = params
                            .get("delta")
                            .or_else(|| params.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if !delta.is_empty() {
                            emit_stream_event(
                                &context.app,
                                &context.stream_id,
                                AiStreamEventPayload {
                                    event_type: "reasoning_delta".to_string(),
                                    stream_id: context.stream_id.clone(),
                                    session_id: Some(context.session_id.clone()),
                                    text_delta: None,
                                    reasoning_delta: Some(delta.to_string()),
                                    message: None,
                                    command_cards: vec![],
                                    usage: None,
                                    error: None,
                                },
                            );
                        }
                    }
                }
            }
            "turn/completed" => {
                let turn_id = params
                    .get("turn")
                    .and_then(|turn| turn.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(turn_id) = turn_id {
                    self.complete_turn(&turn_id, params).await;
                }
            }
            "error" => {
                let message = params
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("Codex request failed")
                    .to_string();
                if params.get("willRetry").and_then(Value::as_bool) == Some(true) {
                    return;
                }
                let contexts: Vec<Arc<CodexTurnContext>> = {
                    let mut active = self.active_turns.lock().unwrap();
                    let mut starting = self.starting_turns.lock().unwrap();
                    if let Some(turn_id) = params.get("turnId").and_then(Value::as_str) {
                        active
                            .remove(turn_id)
                            .or_else(|| {
                                params
                                    .get("threadId")
                                    .and_then(Value::as_str)
                                    .and_then(|id| starting.remove(id))
                            })
                            .into_iter()
                            .collect()
                    } else {
                        active
                            .drain()
                            .map(|(_, context)| context)
                            .chain(starting.drain().map(|(_, context)| context))
                            .collect()
                    }
                };
                for context in contexts {
                    let _completion = CodexCompletionGuard(context.completed.clone());
                    context.mcp_turn.lock().unwrap().take();
                    active_streams().lock().unwrap().remove(&context.stream_id);
                    emit_stream_event(
                        &context.app,
                        &context.stream_id,
                        AiStreamEventPayload {
                            event_type: "error".to_string(),
                            stream_id: context.stream_id.clone(),
                            session_id: Some(context.session_id.clone()),
                            text_delta: None,
                            reasoning_delta: None,
                            message: None,
                            command_cards: vec![],
                            usage: None,
                            error: Some(message.clone()),
                        },
                    );
                }
            }
            _ => {}
        }
    }

    async fn complete_turn(&self, turn_id: &str, params: Value) {
        let context = {
            let mut active = self.active_turns.lock().unwrap();
            active.remove(turn_id).or_else(|| {
                let thread_id = params.get("threadId")?.as_str()?;
                self.starting_turns.lock().unwrap().remove(thread_id)
            })
        };
        let Some(context) = context else {
            return;
        };
        let _completion = CodexCompletionGuard(context.completed.clone());
        context.mcp_turn.lock().unwrap().take();
        active_streams().lock().unwrap().remove(&context.stream_id);

        let turn = params.get("turn").cloned().unwrap_or(Value::Null);
        let status = turn.get("status").and_then(Value::as_str).unwrap_or("");
        if status == "failed" {
            let error = turn
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Codex turn failed")
                .to_string();
            emit_stream_event(
                &context.app,
                &context.stream_id,
                AiStreamEventPayload {
                    event_type: "error".to_string(),
                    stream_id: context.stream_id.clone(),
                    session_id: Some(context.session_id.clone()),
                    text_delta: None,
                    reasoning_delta: None,
                    message: None,
                    command_cards: vec![],
                    usage: None,
                    error: Some(error),
                },
            );
            return;
        }

        let content = {
            let final_text = final_agent_text(&turn);
            if final_text.is_empty() {
                context.text_accumulator.lock().await.clone()
            } else {
                final_text
            }
        };
        if context.dynamic_tool_call_count.load(Ordering::SeqCst) == 0
            && looks_like_codex_command_plan(&content)
        {
            let error = "Codex returned a command plan as text instead of calling the NyaTerm terminal tools. The command was not executed. Please start a new Codex Agent session or check Codex app-server dynamic tool compatibility.".to_string();
            emit_stream_event(
                &context.app,
                &context.stream_id,
                AiStreamEventPayload {
                    event_type: "error".to_string(),
                    stream_id: context.stream_id.clone(),
                    session_id: Some(context.session_id.clone()),
                    text_delta: None,
                    reasoning_delta: None,
                    message: None,
                    command_cards: vec![],
                    usage: turn.get("usage").cloned(),
                    error: Some(error),
                },
            );
            return;
        }
        let message = AiMessage {
            id: format!("msg-{}", uuid()),
            session_id: context.session_id.clone(),
            role: AiMessageRole::Assistant,
            content,
            created_at: now_rfc3339(),
            reasoning_content: None,
            command_cards: vec![],
        };
        if context.settings.record_history {
            let _ = append_message(&context.app, message.clone());
        }
        emit_stream_event(
            &context.app,
            &context.stream_id,
            AiStreamEventPayload {
                event_type: "done".to_string(),
                stream_id: context.stream_id.clone(),
                session_id: Some(context.session_id.clone()),
                text_delta: None,
                reasoning_delta: None,
                message: Some(message),
                command_cards: vec![],
                usage: turn.get("usage").cloned(),
                error: None,
            },
        );
    }
}

pub async fn run_codex_stream(
    app: AppHandle,
    session_manager: Arc<SessionManager>,
    approval_manager: Arc<AgentApprovalManager>,
    manager: Arc<CodexAppServerManager>,
    stream_id: String,
    session_id: String,
    mut request: AiChatRequest,
    settings: AiSettings,
    mut cancel_rx: oneshot::Receiver<()>,
) {
    let result = run_codex_stream_inner(
        app.clone(),
        session_manager,
        approval_manager,
        manager.clone(),
        stream_id.clone(),
        session_id.clone(),
        &mut request,
        settings,
        &mut cancel_rx,
    )
    .await;

    if let Err(error) = result {
        active_streams().lock().unwrap().remove(&stream_id);
        emit_stream_event(
            &app,
            &stream_id,
            AiStreamEventPayload {
                event_type: "error".to_string(),
                stream_id: stream_id.clone(),
                session_id: Some(session_id),
                text_delta: None,
                reasoning_delta: None,
                message: None,
                command_cards: vec![],
                usage: None,
                error: Some(error.to_string()),
            },
        );
    }
}

async fn run_codex_stream_inner(
    app: AppHandle,
    session_manager: Arc<SessionManager>,
    approval_manager: Arc<AgentApprovalManager>,
    manager: Arc<CodexAppServerManager>,
    stream_id: String,
    session_id: String,
    request: &mut AiChatRequest,
    settings: AiSettings,
    cancel_rx: &mut oneshot::Receiver<()>,
) -> AppResult<()> {
    if !settings.codex.enabled {
        return Err(AppError::Config(
            "Codex integration is disabled".to_string(),
        ));
    }
    let selected_model_name = resolve_codex_model_name(&settings, request);
    let session = manager.session(&session_id);
    let mut thread = session
        .thread
        .try_lock()
        .map_err(|_| AppError::Config("A Codex request is already running in this chat.".into()))?;
    if session.cancellation.is_cancelled() {
        return Err(AppError::Cancelled("Codex session closed".into()));
    }

    manager
        .ensure_started(settings.codex.executable_path.clone())
        .await?;
    let runtime_cancel = manager.runtime_cancel.lock().unwrap().clone();

    emit_stream_event(
        &app,
        &stream_id,
        AiStreamEventPayload {
            event_type: "start".to_string(),
            stream_id: stream_id.clone(),
            session_id: Some(session_id.clone()),
            text_delta: None,
            reasoning_delta: None,
            message: None,
            command_cards: vec![],
            usage: None,
            error: None,
        },
    );

    if session.cancellation.is_cancelled() || runtime_cancel.is_cancelled() {
        session.close_credential();
        return Err(AppError::Cancelled("Codex session closed".into()));
    }
    if settings.redaction_enabled {
        redact_request(request);
    }
    if settings.record_history {
        save_user_message(&app, &session_id, request)?;
    }

    let metadata = get_session_backend_metadata(&app, &session_id)?;
    let epoch = manager.epoch.load(Ordering::SeqCst);
    let cached_thread_id = live_codex_thread_id(thread.as_ref(), epoch)?;
    let previous_thread_id = cached_thread_id.clone().or_else(|| {
        thread
            .as_ref()
            .map(|thread| thread.id.clone())
            .or_else(|| reusable_codex_thread_id(metadata.as_ref()))
    });
    if cached_thread_id.is_none()
        && metadata
            .as_ref()
            .is_some_and(|metadata| metadata.codex_thread_mode == Some(CodexThreadMode::Ephemeral))
        && previous_thread_id.is_some()
    {
        return Err(AppError::Config(
            "The temporary Codex thread is no longer available. Start a new chat.".into(),
        ));
    }
    let use_nyaterm_mcp = thread
        .as_ref()
        .map(|thread| thread.mcp)
        .or_else(|| {
            metadata
                .as_ref()
                .and_then(|metadata| metadata.codex_tool_integration_mode.as_deref())
                .map(|mode| mode == "nyaterm_mcp")
        })
        .unwrap_or(settings.codex.tool_integration_mode.as_deref() == Some("nyaterm_mcp"));
    let ephemeral = thread
        .as_ref()
        .map(|thread| thread.ephemeral)
        .or_else(|| {
            metadata
                .as_ref()
                .and_then(|metadata| metadata.codex_thread_mode.as_ref())
                .map(|mode| *mode == CodexThreadMode::Ephemeral)
        })
        .unwrap_or(settings.codex.thread_mode == CodexThreadMode::Ephemeral);
    let mut mcp_credential = session.credential.lock().unwrap().clone();
    if use_nyaterm_mcp && mcp_credential.is_none() {
        let result = if let Some(mcp) = app.try_state::<Arc<crate::core::mcp::McpManager>>() {
            mcp.create_session_credential().await
        } else {
            Err(AppError::Config(
                "The NyaTerm MCP manager is unavailable.".into(),
            ))
        };
        match result {
            Ok(credential) => {
                let credential = Arc::new(credential);
                *session.credential.lock().unwrap() = Some(credential.clone());
                mcp_credential = Some(credential);
            }
            Err(error) if previous_thread_id.is_none() => {
                emit_codex_mcp_fallback(&app, &stream_id, &session_id, request, &error.to_string());
            }
            Err(error) => return Err(error),
        }
    }
    let mut mcp_turn = if let Some(credential) = mcp_credential.as_ref() {
        let (scope, default_id) = codex_request_terminal_scope(request);
        let owner = if let Some(id) = default_id.as_deref().or(scope.first().map(String::as_str)) {
            session_manager
                .session_info(id)
                .await
                .ok()
                .and_then(|info| info.owner_window_label)
        } else {
            None
        };
        Some(credential.activate(
            scope,
            default_id,
            request.permission_mode.clone(),
            owner,
            codex_terminal_presentation_max_lines(&settings),
        )?)
    } else {
        None
    };
    if session.cancellation.is_cancelled() || runtime_cancel.is_cancelled() {
        session.close_credential();
        return Err(AppError::Cancelled("Codex session closed".into()));
    }

    let response = request_codex_thread(
        &manager,
        &mut thread,
        previous_thread_id.as_deref(),
        selected_model_name.as_deref(),
        ephemeral,
        mcp_credential.as_deref(),
    )
    .await;
    let thread_id = match response {
        Ok(id) => id,
        Err(error) if previous_thread_id.is_none() && mcp_credential.is_some() => {
            emit_codex_mcp_fallback(&app, &stream_id, &session_id, request, &error.to_string());
            mcp_turn.take();
            session.close_credential();
            mcp_credential.take();
            request_codex_thread(
                &manager,
                &mut thread,
                None,
                selected_model_name.as_deref(),
                ephemeral,
                None,
            )
            .await?
        }
        Err(error) => return Err(error),
    };
    if session.cancellation.is_cancelled() || runtime_cancel.is_cancelled() {
        session.close_credential();
        return Err(AppError::Cancelled("Codex session closed".into()));
    }
    if cached_thread_id.is_none() {
        set_session_backend_metadata(
            &app,
            &session_id,
            AiSessionBackendMetadata {
                backend: AiBackendKind::Codex,
                external_thread_id: Some(thread_id.clone()),
                codex_terminal_tools_version: Some(CODEX_TERMINAL_TOOLS_VERSION),
                codex_thread_mode: Some(if ephemeral {
                    CodexThreadMode::Ephemeral
                } else {
                    CodexThreadMode::Persistent
                }),
                codex_tool_integration_mode: Some(
                    if mcp_credential.is_some() {
                        "nyaterm_mcp"
                    } else {
                        "dynamic_tools"
                    }
                    .into(),
                ),
            },
        )?;
        set_session_external_session_id(&app, &session_id, AiAgentKind::Codex, thread_id.clone())?;
    }

    let prompt = build_codex_agent_prompt(request, &settings);
    let completed = CancellationToken::new();
    manager.starting_turns.lock().unwrap().insert(
        thread_id.clone(),
        Arc::new(CodexTurnContext {
            completed: completed.clone(),
            mcp_turn: StdMutex::new(mcp_turn.take()),
            app,
            session_manager,
            approval_manager,
            stream_id: stream_id.clone(),
            session_id,
            request: request.clone(),
            settings,
            step_counter: Mutex::new(0),
            dynamic_tool_call_count: AtomicU64::new(0),
            text_accumulator: Mutex::new(String::new()),
        }),
    );
    let mut registration = CodexTurnRegistration {
        manager: &manager,
        thread_id: &thread_id,
        turn_id: None,
    };
    let response = manager
        .request(
            "turn/start",
            codex_turn_start_params(&thread_id, &prompt, selected_model_name.as_deref()),
        )
        .await?;
    let turn_id = response
        .get("turn")
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Config("Codex turn/start returned no turn id".to_string()))?
        .to_string();
    {
        let mut active = manager.active_turns.lock().unwrap();
        if let Some(context) = manager.starting_turns.lock().unwrap().remove(&thread_id) {
            active.insert(turn_id.clone(), context);
        }
    }
    registration.turn_id = Some(turn_id.clone());

    tokio::select! {
        _ = wait_for_codex_cancellation(cancel_rx, &session.cancellation, &runtime_cancel) => {
            if let Some(context) = manager.active_turns.lock().unwrap().remove(&turn_id) {
                context.mcp_turn.lock().unwrap().take();
            }
            let _ = timeout(Duration::from_secs(5), manager
                .request(
                    "turn/interrupt",
                    json!({ "threadId": thread_id.clone(), "turnId": turn_id.clone() }),
                ))
                .await;
            manager.active_turns.lock().unwrap().remove(&turn_id);
            Err(AppError::Cancelled("AI stream cancelled".to_string()))
        }
        _ = completed.cancelled() => Ok(())
    }
}

async fn wait_for_codex_cancellation(
    cancel_rx: &mut oneshot::Receiver<()>,
    session: &CancellationToken,
    runtime: &CancellationToken,
) {
    tokio::select! {
        // Completion removes the sender from active_streams. Only an explicit
        // cancellation message, rather than a dropped sender, interrupts a turn.
        Ok(()) = cancel_rx => {},
        _ = session.cancelled() => {},
        _ = runtime.cancelled() => {},
    }
}

fn live_codex_thread_id(
    thread: Option<&LoadedCodexThread>,
    epoch: u64,
) -> AppResult<Option<String>> {
    match thread {
        Some(thread) if thread.epoch == epoch => Ok(Some(thread.id.clone())),
        Some(thread) if thread.ephemeral => Err(AppError::Config(
            "The temporary Codex thread was lost. Start a new chat.".into(),
        )),
        _ => Ok(None),
    }
}

async fn request_codex_thread(
    manager: &CodexAppServerManager,
    loaded: &mut Option<LoadedCodexThread>,
    previous_thread_id: Option<&str>,
    model: Option<&str>,
    ephemeral: bool,
    credential: Option<&SessionMcpCredential>,
) -> AppResult<String> {
    let epoch = manager.epoch.load(Ordering::SeqCst);
    if let Some(id) = live_codex_thread_id(loaded.as_ref(), epoch)? {
        return Ok(id);
    }
    let mut params = codex_thread_start_params_with_mcp(model, ephemeral, credential);
    let method = if let Some(id) = previous_thread_id {
        params.as_object_mut().unwrap().remove("ephemeral");
        params.as_object_mut().unwrap().remove("dynamicTools");
        params["threadId"] = json!(id);
        if credential.is_none() {
            params["config"] = json!({ "mcp_servers": { "nyaterm": { "enabled": false } } });
        }
        "thread/resume"
    } else {
        "thread/start"
    };
    let response = manager.request(method, params).await?;
    let id = response
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .or_else(|| response.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| AppError::Config(format!("Codex {method} returned no thread id")))?;
    if previous_thread_id.is_some_and(|previous| previous != id) {
        return Err(AppError::Config("Codex resumed a different thread.".into()));
    }
    *loaded = Some(LoadedCodexThread {
        id: id.clone(),
        epoch,
        ephemeral,
        mcp: credential.is_some(),
    });
    Ok(id)
}

const CODEX_DETECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
struct CodexCliCandidate {
    executable: String,
    source: &'static str,
}

fn codex_executable(path: Option<&str>) -> String {
    path.map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("codex")
        .to_string()
}

async fn resolve_codex_executable(path: Option<String>) -> AppResult<String> {
    let status = CodexAppServerManager::detect_cli(path).await;
    if !status.installed {
        return Err(AppError::Config(status.error.unwrap_or_else(|| {
            "Codex CLI was not detected in PATH or common install locations".to_string()
        })));
    }

    status
        .path
        .ok_or_else(|| AppError::Config("Codex CLI detection returned no executable path".into()))
}

async fn discover_codex_candidates(path: Option<&str>) -> Vec<CodexCliCandidate> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    if let Some(path) = path.map(str::trim).filter(|value| !value.is_empty()) {
        add_codex_candidate(&mut candidates, &mut seen, path, "configured");
    }
    add_codex_candidate(&mut candidates, &mut seen, "codex", "path");
    add_common_codex_candidates(&mut candidates, &mut seen);
    for discovered in discover_codex_with_path_command().await {
        add_codex_candidate(&mut candidates, &mut seen, discovered, "path_lookup");
    }

    candidates
}

fn add_codex_candidate(
    candidates: &mut Vec<CodexCliCandidate>,
    seen: &mut HashSet<String>,
    executable: impl AsRef<str>,
    source: &'static str,
) {
    let executable = executable.as_ref().trim();
    if executable.is_empty() {
        return;
    }
    let key = codex_candidate_key(executable);
    if seen.insert(key) {
        candidates.push(CodexCliCandidate {
            executable: executable.to_string(),
            source,
        });
    }
}

fn add_existing_codex_candidate(
    candidates: &mut Vec<CodexCliCandidate>,
    seen: &mut HashSet<String>,
    path: PathBuf,
    source: &'static str,
) {
    if path.exists() {
        add_codex_candidate(candidates, seen, path.to_string_lossy(), source);
    }
}

fn codex_candidate_key(executable: &str) -> String {
    #[cfg(windows)]
    {
        executable.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        executable.to_string()
    }
}

fn add_common_codex_candidates(
    candidates: &mut Vec<CodexCliCandidate>,
    seen: &mut HashSet<String>,
) {
    #[cfg(windows)]
    {
        if let Ok(appdata) = env::var("APPDATA") {
            let npm = Path::new(&appdata).join("npm");
            for name in ["codex.cmd", "codex.exe", "codex"] {
                add_existing_codex_candidate(candidates, seen, npm.join(name), "common");
            }
        }
        if let Ok(nvm_symlink) = env::var("NVM_SYMLINK") {
            let nodejs = Path::new(&nvm_symlink);
            for name in ["codex.cmd", "codex.exe", "codex"] {
                add_existing_codex_candidate(candidates, seen, nodejs.join(name), "common");
            }
        }
        if let Ok(local_appdata) = env::var("LOCALAPPDATA") {
            let pnpm = Path::new(&local_appdata).join("pnpm");
            for name in ["codex.cmd", "codex.exe", "codex"] {
                add_existing_codex_candidate(candidates, seen, pnpm.join(name), "common");
            }
        }
        if let Ok(userprofile) = env::var("USERPROFILE") {
            let home = Path::new(&userprofile);
            for dir in [
                home.join("scoop").join("shims"),
                home.join(".bun").join("bin"),
            ] {
                for name in ["codex.cmd", "codex.exe", "codex"] {
                    add_existing_codex_candidate(candidates, seen, dir.join(name), "common");
                }
            }
        }
    }

    #[cfg(not(windows))]
    {
        if let Ok(home) = env::var("HOME") {
            let home = Path::new(&home);
            for path in [
                home.join(".local").join("bin").join("codex"),
                home.join(".npm-global").join("bin").join("codex"),
                home.join(".bun").join("bin").join("codex"),
                home.join(".cargo").join("bin").join("codex"),
            ] {
                add_existing_codex_candidate(candidates, seen, path, "common");
            }
        }
        for path in [
            PathBuf::from("/opt/homebrew/bin/codex"),
            PathBuf::from("/usr/local/bin/codex"),
            PathBuf::from("/usr/bin/codex"),
        ] {
            add_existing_codex_candidate(candidates, seen, path, "common");
        }
    }
}

async fn discover_codex_with_path_command() -> Vec<String> {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("where.exe");
        command.arg("codex");
        command
    } else {
        let mut command = Command::new("which");
        command.args(["-a", "codex"]);
        command
    };
    hide_window(&mut command);
    let output = command.output().await;

    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

async fn probe_codex_cli(executable: &str) -> Result<String, String> {
    let mut command = Command::new(executable);
    command.arg("--version");
    hide_window(&mut command);
    let output = timeout(CODEX_DETECT_TIMEOUT, command.output())
        .await
        .map_err(|_| "timed out while running --version".to_string())?
        .map_err(|error| error.to_string())?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Ok(if stdout.is_empty() { stderr } else { stdout });
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let details = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        output.status.to_string()
    };
    Err(details)
}

fn detect_error_message(errors: &[String]) -> String {
    if errors.is_empty() {
        return "Codex CLI was not found in PATH or common install locations".to_string();
    }

    let mut message = "Codex CLI was not detected".to_string();
    for error in errors.iter().take(4) {
        message.push_str("; ");
        message.push_str(error);
    }
    if errors.len() > 4 {
        message.push_str(&format!("; {} more candidates failed", errors.len() - 4));
    }
    message
}

fn parse_account_status(value: &Value) -> CodexAccountStatus {
    let account = value.get("account").unwrap_or(&Value::Null);
    let auth_mode = account
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string);
    CodexAccountStatus {
        connected: auth_mode.is_some(),
        auth_mode,
        plan_type: account
            .get("planType")
            .and_then(Value::as_str)
            .map(str::to_string),
        email: account
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_string),
        requires_openai_auth: value
            .get("requiresOpenaiAuth")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn reusable_codex_thread_id(metadata: Option<&AiSessionBackendMetadata>) -> Option<String> {
    let metadata = metadata?;
    if metadata.backend != AiBackendKind::Codex
        || metadata.codex_terminal_tools_version != Some(CODEX_TERMINAL_TOOLS_VERSION)
    {
        return None;
    }
    metadata.external_thread_id.clone()
}

fn resolve_codex_model_name(settings: &AiSettings, request: &AiChatRequest) -> Option<String> {
    request
        .model_name
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .or_else(|| {
            settings
                .codex
                .default_model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
        })
        .map(ToOwned::to_owned)
}

fn codex_terminal_presentation_max_lines(settings: &AiSettings) -> Option<u16> {
    (!settings.agent_background_execution_enabled).then_some(settings.terminal_output_lines)
}

#[cfg(test)]
fn codex_thread_start_params(model: Option<&str>, ephemeral: bool) -> Value {
    codex_thread_start_params_with_mcp(model, ephemeral, None)
}

fn codex_thread_start_params_with_mcp(
    model: Option<&str>,
    ephemeral: bool,
    credential: Option<&SessionMcpCredential>,
) -> Value {
    let mut params = json!({
        "cwd": null,
        "ephemeral": ephemeral,
        "approvalPolicy": {
            "granular": {
                "rules": false,
                "mcp_elicitations": false,
                "request_permissions": false,
                "sandbox_approval": false
            }
        },
        "approvalsReviewer": "user",
        "sandbox": "read-only",
        "developerInstructions": codex_developer_instructions()
    });
    if let Some(credential) = credential {
        params["config"] = codex_nyaterm_mcp_config(credential.sidecar_path(), credential.env());
    } else {
        params["dynamicTools"] = json!([terminal_tool_namespace()]);
    }
    if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
        params["model"] = json!(model);
    }
    params
}

fn nyaterm_mcp_tool_names() -> Vec<&'static str> {
    MCP_TOOL_REGISTRY
        .iter()
        .map(|definition| definition.tool)
        .collect()
}

fn codex_nyaterm_mcp_config(sidecar_path: &Path, environment: HashMap<String, String>) -> Value {
    json!({
        "mcp_servers": {
            "nyaterm": {
                "command": sidecar_path,
                "args": [],
                "env": environment,
                "required": true,
                "enabled_tools": nyaterm_mcp_tool_names(),
                "default_tools_approval_mode": "approve",
            }
        }
    })
}

fn emit_codex_mcp_fallback(
    app: &AppHandle,
    stream_id: &str,
    session_id: &str,
    request: &AiChatRequest,
    reason: &str,
) {
    let reason = redact_sensitive_text(reason);
    tracing::warn!(
        stream_id,
        session_id,
        permission_mode = ?request.permission_mode,
        reason,
        "Codex MCP integration failed; using the NyaTerm dynamic terminal tool"
    );
    emit_stream_event(
        app,
        stream_id,
        AiStreamEventPayload {
            event_type: "warning".to_string(),
            stream_id: stream_id.to_string(),
            session_id: Some(session_id.to_string()),
            text_delta: None,
            reasoning_delta: None,
            message: None,
            command_cards: vec![],
            usage: None,
            error: Some("codex_mcp_fallback".to_string()),
        },
    );
    let _ = append_ai_audit(
        app,
        AppendAiAuditRequest {
            connection_id: request.connection_id.clone(),
            action: "ai.codex_mcp_fallback".to_string(),
            user_input: None,
            generated_command: None,
            risk_level: None,
            inserted_to_terminal: false,
            executed: false,
            blocked: false,
            source: Some("codex_mcp".to_string()),
            client: Some("Codex".to_string()),
            capability: None,
            session_id: request.terminal_session_id.clone(),
            permission_mode: Some(request.permission_mode.clone()),
            approval_decision: Some("fallback".to_string()),
            success: Some(false),
            duration_ms: None,
            error: Some(reason),
        },
    );
}

fn codex_request_terminal_scope(request: &AiChatRequest) -> (Vec<String>, Option<String>) {
    let mut scope = request
        .targets
        .iter()
        .map(|target| target.terminal_session_id.clone())
        .collect::<HashSet<_>>();
    if let Some(id) = request.terminal_session_id.as_ref() {
        scope.insert(id.clone());
    }
    if let Some(id) = request.default_target_session_id.as_ref() {
        scope.insert(id.clone());
    }
    let mut scope = scope.into_iter().collect::<Vec<_>>();
    scope.sort();
    let default_id = request
        .default_target_session_id
        .clone()
        .or_else(|| request.terminal_session_id.clone())
        .filter(|id| scope.contains(id));
    (scope, default_id)
}

fn codex_turn_start_params(thread_id: &str, prompt: &str, model: Option<&str>) -> Value {
    let mut params = json!({
        "threadId": thread_id,
        "clientUserMessageId": format!("msg-{}", uuid()),
        "input": [{ "type": "text", "text": prompt, "text_elements": [] }],
    });
    if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
        params["model"] = json!(model);
    }
    params
}

fn terminal_tool_namespace() -> Value {
    json!({
        "type": "namespace",
        "name": "nyaterm_terminal",
        "description": "Operate the active NyaTerm terminal sessions. Use these tools instead of local shell or file tools when working with remote terminals.",
        "tools": [
            {
                "type": "function",
                "name": "get_context",
                "description": "Read the available NyaTerm terminal targets and recent terminal context.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "type": "function",
                "name": "execute_command",
                "description": "Execute a shell command in a NyaTerm terminal session after NyaTerm approval policy is applied.",
                "inputSchema": {
                    "type": "object",
                    "required": ["targetTerminalSessionId", "command"],
                    "properties": {
                        "targetTerminalSessionId": { "type": "string" },
                        "command": { "type": "string" },
                        "reason": { "type": "string" }
                    }
                }
            }
        ]
    })
}

fn codex_developer_instructions() -> &'static str {
    "You are running inside NyaTerm as a terminal automation agent. Use the nyaterm_terminal namespace for terminal work. Call nyaterm_terminal.get_context when target context is unclear. Call nyaterm_terminal.execute_command for commands that need to run in a NyaTerm terminal session. Do not output command-plan JSON such as {\"commands\":[...]}; if a command is needed, call the tool. Do not use local shell or file tools for the user's remote terminal. After observations are sufficient, answer the user in normal assistant text."
}

fn build_codex_agent_prompt(request: &AiChatRequest, settings: &AiSettings) -> String {
    format!(
        "{}\n\nCodex Agent protocol:\n- Use the nyaterm_terminal namespace for all NyaTerm terminal actions.\n- If you need to inspect or change the terminal, call nyaterm_terminal.execute_command with targetTerminalSessionId and command.\n- If the target is unclear, call nyaterm_terminal.get_context first.\n- Do not return command cards, protocol JSON, or {{\"commands\":[...]}} in assistant text.\n- When finished, reply with a normal user-facing final answer.",
        build_agent_prompt(request, settings)
    )
}

fn terminal_context_text(request: &AiChatRequest) -> String {
    serde_json::to_string_pretty(&json!({
        "primaryContext": request.context,
        "targets": request.targets,
        "targetContexts": request.target_contexts,
        "instruction": "Use nyaterm_terminal.execute_command for remote terminal actions. Do not use local shell/file tools for the user's remote terminal."
    }))
    .unwrap_or_else(|_| "Terminal context unavailable".to_string())
}

fn observation_text(observation: &CommandObservation) -> String {
    serde_json::to_string_pretty(observation).unwrap_or_else(|_| observation.output.clone())
}

fn dynamic_text(success: bool, text: &str) -> Value {
    json!({
        "success": success,
        "contentItems": [{ "type": "inputText", "text": text }]
    })
}

fn final_agent_text(turn: &Value) -> String {
    turn.get("items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().rev().find_map(|item| {
                if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
                    return None;
                }
                item.get("text").and_then(Value::as_str).map(str::to_string)
            })
        })
        .unwrap_or_default()
}

fn looks_like_codex_command_plan(text: &str) -> bool {
    let trimmed = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return false;
    };
    value
        .get("commands")
        .and_then(Value::as_array)
        .is_some_and(|commands| !commands.is_empty())
}

async fn read_stderr(stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let sanitized = sanitize_codex_log_line(&line);
        if !sanitized.trim().is_empty() {
            tracing::debug!(target: "codex_app_server", message = %sanitized);
        }
    }
}

fn sanitize_codex_log_line(line: &str) -> String {
    redact_marker_values(
        line,
        &["access_token=", "refresh_token=", "id_token=", "code="],
    )
}

pub async fn manager_from_app(app: &AppHandle) -> AppResult<Arc<CodexAppServerManager>> {
    Ok(app.state::<Arc<CodexAppServerManager>>().inner().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn normal_completion_is_not_mistaken_for_cancellation() {
        let session = CancellationToken::new();
        let runtime = CancellationToken::new();
        let (sender, mut receiver) = oneshot::channel();
        drop(sender);
        assert!(
            timeout(
                Duration::from_millis(20),
                wait_for_codex_cancellation(&mut receiver, &session, &runtime)
            )
            .await
            .is_err()
        );
        let (sender, mut receiver) = oneshot::channel();
        sender.send(()).unwrap();
        timeout(
            Duration::from_secs(1),
            wait_for_codex_cancellation(&mut receiver, &session, &runtime),
        )
        .await
        .unwrap();
        let (_sender, mut receiver) = oneshot::channel();
        session.cancel();
        timeout(
            Duration::from_secs(1),
            wait_for_codex_cancellation(&mut receiver, &session, &runtime),
        )
        .await
        .unwrap();
        let session = CancellationToken::new();
        let (_sender, mut receiver) = oneshot::channel();
        runtime.cancel();
        timeout(
            Duration::from_secs(1),
            wait_for_codex_cancellation(&mut receiver, &session, &runtime),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires an authenticated Codex CLI and built nyaterm-mcp helper"]
    async fn real_codex_recalls_prior_turn_and_calls_mcp_twice() {
        let helper_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let root = tempfile::tempdir().unwrap();
        let host = crate::core::mcp::McpManager::new(
            Arc::new(SessionManager::new()),
            root.path(),
            helper_dir,
        );
        let bridge = host.start_test_bridge().await;
        let credential = Arc::new(host.create_session_credential().await.unwrap());
        let manager = Arc::new(CodexAppServerManager::new());
        let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
        *manager.test_events.lock().unwrap() = Some(events);
        let result = async {
            manager.ensure_started(None).await?;
            *manager.session("smoke").credential.lock().unwrap() = Some(credential.clone());
            let mut loaded = None;
            let phrase = format!("NYATERM_{}", uuid());
            let mut thread_ids = vec![];
            for prompt in [
                format!("Read-only regression test. Remember this exact phrase: {phrase}. Call the configured NyaTerm MCP get_environment tool once, then reply READY. Do not run commands or use other tools."),
                "Call the configured NyaTerm MCP get_environment tool once again, then reply with the exact phrase I gave you in the previous message. Do not run commands or use other tools.".to_string(),
            ] {
                let guard = credential.activate(vec![], None, crate::config::AiPermissionMode::Observer, None, None)?;
                let id = request_codex_thread(&manager, &mut loaded, None, None, true, Some(&credential)).await?;
                let response = manager.request("turn/start", codex_turn_start_params(&id, &prompt, None)).await?;
                let turn_id = response["turn"]["id"].as_str().unwrap().to_string();
                let (text, calls) = timeout(Duration::from_secs(120), async {
                    let mut calls = 0;
                    loop {
                        let (method, params) = received.recv().await.unwrap();
                        if method == "item/completed" && params["item"]["type"] == "mcpToolCall"
                            && params["item"]["status"] == "completed" {
                            calls += 1;
                        }
                        if method == "turn/completed" && params["turn"]["id"] == turn_id {
                            if params["turn"]["status"] != "completed" {
                                return Err(AppError::Config("Real Codex smoke turn did not complete successfully.".into()));
                            }
                            return Ok((final_agent_text(&params["turn"]), calls));
                        }
                    }
                }).await.map_err(|_| AppError::Config("Real Codex smoke turn timed out.".into()))??;
                drop(guard);
                if calls == 0 { return Err(AppError::Config("Real Codex did not complete an MCP call.".into())); }
                if !thread_ids.is_empty() && !text.contains(&phrase) {
                    return Err(AppError::Config("Real Codex did not recall the prior turn.".into()));
                }
                thread_ids.push(id);
            }
            assert_eq!(thread_ids[0], thread_ids[1]);
            Ok::<_, AppError>(())
        }.await;
        credential.close();
        if let Some(child) = manager.child.lock().await.as_mut() {
            let _ = child.kill().await;
        }
        host.stop_test_bridge();
        bridge.await.unwrap();
        result.unwrap();
    }

    // A real child process exercising the same JSONL transport as Codex, without model access.
    #[test]
    fn fake_codex_app_server() {
        use std::io::{BufRead, Write};
        let Some(root) = std::env::var_os("NYATERM_CODEX_TEST_SERVER") else {
            return;
        };
        let root = PathBuf::from(root);
        let state_path = root.join("threads.json");
        let mut threads: HashMap<String, Vec<String>> = std::fs::read(&state_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("requests.jsonl"))
            .unwrap();
        for line in std::io::stdin().lock().lines() {
            let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
            writeln!(log, "{request}").unwrap();
            log.flush().unwrap();
            let id = &request["id"];
            let params = &request["params"];
            let result = match request["method"].as_str().unwrap() {
                "thread/start"
                    if params.get("config").is_some()
                        && std::env::var_os("NYATERM_CODEX_TEST_REJECT_MCP").is_some() =>
                {
                    None
                }
                "thread/start" => {
                    let thread = format!("thread-{}", threads.len() + 1);
                    threads.insert(thread.clone(), vec![]);
                    Some(json!({ "thread": { "id": thread } }))
                }
                "thread/resume" => params["threadId"]
                    .as_str()
                    .filter(|id| threads.contains_key(*id))
                    .map(|thread| json!({ "thread": { "id": thread } })),
                "turn/start" => {
                    let thread = params["threadId"].as_str().unwrap();
                    let history = threads.get_mut(thread).unwrap();
                    history.push(params["input"][0]["text"].as_str().unwrap().to_string());
                    Some(
                        json!({ "turn": { "id": format!("turn-{}", history.len()) }, "history": history }),
                    )
                }
                _ => Some(json!({})),
            };
            std::fs::write(&state_path, serde_json::to_vec(&threads).unwrap()).unwrap();
            let response = if let Some(result) = result {
                json!({ "id": id, "result": result })
            } else {
                json!({ "id": id, "error": { "code": -32000, "message": "fixture rejected request" } })
            };
            println!("\n{response}");
            std::io::stdout().flush().unwrap();
        }
    }

    async fn fixture_server(
        root: &Path,
        reject_mcp: bool,
    ) -> (Arc<CodexAppServerManager>, tokio::task::JoinHandle<()>) {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "core::ai::codex::tests::fake_codex_app_server",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("NYATERM_CODEX_TEST_SERVER", root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if reject_mcp {
            command.env("NYATERM_CODEX_TEST_REJECT_MCP", "1");
        }
        let mut child = command.spawn().unwrap();
        let manager = Arc::new(CodexAppServerManager::new());
        *manager.writer.lock().await = Some(BufWriter::new(child.stdin.take().unwrap()));
        let stdout = child.stdout.take().unwrap();
        *manager.child.lock().await = Some(child);
        manager.epoch.store(1, Ordering::SeqCst);
        *manager.state.write().await = CodexRuntimeState::Ready;
        let reader = manager.clone();
        let task = tokio::spawn(async move {
            reader.read_stdout(stdout).await;
        });
        (manager, task)
    }

    async fn stop_fixture(manager: &CodexAppServerManager, task: tokio::task::JoinHandle<()>) {
        manager
            .child
            .lock()
            .await
            .as_mut()
            .unwrap()
            .kill()
            .await
            .unwrap();
        timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
    }

    fn fixture_requests(root: &Path) -> Vec<Value> {
        std::fs::read_to_string(root.join("requests.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    async fn fixture_mcp(
        root: &Path,
    ) -> (Arc<crate::core::mcp::McpManager>, Arc<SessionMcpCredential>) {
        std::fs::write(
            root.join(if cfg!(windows) {
                "nyaterm-mcp.exe"
            } else {
                "nyaterm-mcp"
            }),
            b"fixture",
        )
        .unwrap();
        let host = crate::core::mcp::McpManager::new(Arc::new(SessionManager::new()), root, root);
        // initialize() requires a Tauri window. The host fixture tests cover activation;
        // this transport test only needs an isolated, structurally valid MCP environment.
        let credential = host.create_test_session_credential().await;
        (host, Arc::new(credential))
    }

    #[tokio::test]
    async fn persistent_and_ephemeral_threads_keep_three_turns_on_one_thread() {
        for ephemeral in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (manager, task) = fixture_server(root.path(), false).await;
            let (_host, credential) = fixture_mcp(root.path()).await;
            let mut loaded = None;
            let mut ids = vec![];
            for prompt in ["remember alpha", "recall alpha", "recall both messages"] {
                let id = request_codex_thread(
                    &manager,
                    &mut loaded,
                    None,
                    None,
                    ephemeral,
                    Some(&credential),
                )
                .await
                .unwrap();
                let result = manager
                    .request("turn/start", codex_turn_start_params(&id, prompt, None))
                    .await
                    .unwrap();
                ids.push(id);
                assert_eq!(result["history"][0], "remember alpha");
                assert_eq!(result["history"].as_array().unwrap().len(), ids.len());
            }
            assert!(ids.iter().all(|id| id == &ids[0]));
            stop_fixture(&manager, task).await;
            let requests = fixture_requests(root.path());
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request["method"] == "thread/start")
                    .count(),
                1
            );
            assert!(
                !requests
                    .iter()
                    .any(|request| request["method"] == "thread/resume")
            );
            assert_eq!(requests[0]["params"]["ephemeral"], ephemeral);
        }
    }

    #[tokio::test]
    async fn persistent_resume_uses_new_mcp_identity_and_retains_history() {
        let root = tempfile::tempdir().unwrap();
        let (manager, task) = fixture_server(root.path(), false).await;
        let (_host, credential) = fixture_mcp(root.path()).await;
        let session = manager.session("chat");
        *session.credential.lock().unwrap() = Some(credential.clone());
        let id = request_codex_thread(
            &manager,
            &mut *session.thread.lock().await,
            None,
            None,
            false,
            Some(&credential),
        )
        .await
        .unwrap();
        manager
            .request(
                "turn/start",
                codex_turn_start_params(&id, "remember before restart", None),
            )
            .await
            .unwrap();
        stop_fixture(&manager, task).await;
        assert!(
            credential
                .activate(
                    vec![],
                    None,
                    crate::config::AiPermissionMode::Confirm,
                    None,
                    None
                )
                .is_err()
        );

        let (manager, task) = fixture_server(root.path(), false).await;
        let (_host, replacement) = fixture_mcp(root.path()).await;
        assert_ne!(credential.env(), replacement.env());
        let mut loaded = None;
        let resumed = request_codex_thread(
            &manager,
            &mut loaded,
            Some(&id),
            None,
            false,
            Some(&replacement),
        )
        .await
        .unwrap();
        assert_eq!(resumed, id);
        let result = manager
            .request(
                "turn/start",
                codex_turn_start_params(&resumed, "recall", None),
            )
            .await
            .unwrap();
        assert_eq!(result["history"][0], "remember before restart");
        stop_fixture(&manager, task).await;
        let requests = fixture_requests(root.path());
        let resume = requests
            .iter()
            .find(|request| request["method"] == "thread/resume")
            .unwrap();
        assert_eq!(
            resume["params"]["config"],
            codex_nyaterm_mcp_config(replacement.sidecar_path(), replacement.env())
        );
        assert_eq!(resume["params"]["sandbox"], "read-only");
        assert!(resume["params"].get("ephemeral").is_none());
    }

    #[tokio::test]
    async fn fallback_is_reused_and_failed_resume_preserves_the_original_thread() {
        let root = tempfile::tempdir().unwrap();
        let (manager, task) = fixture_server(root.path(), true).await;
        let (_host, credential) = fixture_mcp(root.path()).await;
        let mut loaded = None;
        assert!(
            request_codex_thread(&manager, &mut loaded, None, None, false, Some(&credential))
                .await
                .is_err()
        );
        assert!(loaded.is_none());
        let id = request_codex_thread(&manager, &mut loaded, None, None, false, None)
            .await
            .unwrap();
        assert!(!loaded.as_ref().unwrap().mcp);
        for _ in 0..2 {
            assert_eq!(
                request_codex_thread(&manager, &mut loaded, None, None, false, None)
                    .await
                    .unwrap(),
                id
            );
        }
        loaded.as_mut().unwrap().epoch = 0;
        assert!(
            request_codex_thread(
                &manager,
                &mut loaded,
                Some("missing-thread"),
                None,
                false,
                Some(&credential)
            )
            .await
            .is_err()
        );
        assert_eq!(loaded.as_ref().unwrap().id, id);
        assert!(manager.pending.lock().unwrap().is_empty());
        stop_fixture(&manager, task).await;
        let requests = fixture_requests(root.path());
        assert_eq!(
            requests
                .iter()
                .filter(|request| request["method"] == "thread/start")
                .count(),
            2
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request["method"] == "thread/resume")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn chat_locks_cleanup_and_ephemeral_process_loss_are_isolated() {
        let manager = CodexAppServerManager::new();
        let first = manager.session("first");
        let second = manager.session("second");
        let _lock = first.thread.try_lock().unwrap();
        assert!(manager.session("first").thread.try_lock().is_err());
        assert!(second.thread.try_lock().is_ok());
        let root = tempfile::tempdir().unwrap();
        let (_host, credential) = fixture_mcp(root.path()).await;
        let guard = credential
            .activate(
                vec![],
                None,
                crate::config::AiPermissionMode::Confirm,
                None,
                None,
            )
            .unwrap();
        *first.credential.lock().unwrap() = Some(credential.clone());
        drop(_lock);
        manager.release_session("first").await;
        assert!(first.cancellation.is_cancelled());
        assert!(!second.cancellation.is_cancelled());
        assert!(
            credential
                .activate(
                    vec![],
                    None,
                    crate::config::AiPermissionMode::Confirm,
                    None,
                    None
                )
                .is_err()
        );
        drop(guard);
        manager.release_all_sessions().await;
        assert!(second.cancellation.is_cancelled());
        assert!(manager.sessions.lock().unwrap().is_empty());
        let thread = LoadedCodexThread {
            id: "temporary".into(),
            epoch: 1,
            ephemeral: true,
            mcp: true,
        };
        assert_eq!(
            live_codex_thread_id(Some(&thread), 1).unwrap().as_deref(),
            Some("temporary")
        );
        assert!(live_codex_thread_id(Some(&thread), 2).is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn resolves_windows_cmd_launcher_before_starting_app_server() {
        let path = std::env::temp_dir().join(format!("nyaterm-codex-{}.cmd", uuid()));
        std::fs::write(&path, "@echo off\r\necho codex-test\r\n").unwrap();

        let resolved = resolve_codex_executable(Some(path.to_string_lossy().into_owned()))
            .await
            .unwrap();

        assert_eq!(resolved, path.to_string_lossy());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sanitizes_codex_auth_material_from_logs() {
        let line =
            r#"login access_token=abc123 refresh_token=def456 id_token=ghi789 code=xyz&state=ok"#;

        let sanitized = sanitize_codex_log_line(line);

        assert!(!sanitized.contains("abc123"));
        assert!(!sanitized.contains("def456"));
        assert!(!sanitized.contains("ghi789"));
        assert!(!sanitized.contains("code=xyz"));
        assert!(sanitized.contains("access_token=[redacted]"));
        assert!(sanitized.contains("refresh_token=[redacted]"));
        assert!(sanitized.contains("id_token=[redacted]"));
        assert!(sanitized.contains("code=[redacted]&state=ok"));
    }

    #[test]
    fn terminal_presentation_uses_configured_lines_only_for_foreground_execution() {
        let mut settings = AiSettings {
            terminal_output_lines: 23,
            ..AiSettings::default()
        };

        assert_eq!(codex_terminal_presentation_max_lines(&settings), Some(23));

        settings.agent_background_execution_enabled = true;
        assert_eq!(codex_terminal_presentation_max_lines(&settings), None);
    }

    #[test]
    fn extracts_final_agent_text_from_turn_items() {
        let turn = json!({
            "items": [
                { "type": "reasoning", "text": "thinking" },
                { "type": "agentMessage", "text": "first" },
                { "type": "agentMessage", "text": "final" }
            ]
        });

        assert_eq!(final_agent_text(&turn), "final");
    }

    #[test]
    fn thread_start_params_register_nyaterm_terminal_tools() {
        let params = codex_thread_start_params(Some("gpt-5-codex"), false);

        let namespace = params
            .get("dynamicTools")
            .and_then(Value::as_array)
            .and_then(|tools| tools.first())
            .expect("terminal tool namespace");

        assert_eq!(
            namespace.get("name").and_then(Value::as_str),
            Some("nyaterm_terminal")
        );
        let tools = namespace
            .get("tools")
            .and_then(Value::as_array)
            .expect("namespace tools");
        assert!(
            tools.iter().any(|tool| {
                tool.get("name").and_then(Value::as_str) == Some("execute_command")
            })
        );
        assert!(
            params
                .get("developerInstructions")
                .and_then(Value::as_str)
                .is_some_and(|value| value.contains("nyaterm_terminal.execute_command"))
        );
    }

    #[test]
    fn codex_preapproves_only_registered_nyaterm_mcp_tools() {
        let config = codex_nyaterm_mcp_config(
            Path::new("nyaterm-mcp"),
            HashMap::from([("NYATERM_MCP_TOKEN".to_string(), "secret".to_string())]),
        );
        let server = config
            .pointer("/mcp_servers/nyaterm")
            .expect("nyaterm MCP server config");
        let enabled_tools = server["enabled_tools"]
            .as_array()
            .expect("enabled tool allow-list")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();

        assert_eq!(
            server["default_tools_approval_mode"].as_str(),
            Some("approve")
        );
        assert_eq!(enabled_tools, nyaterm_mcp_tool_names());

        let thread = codex_thread_start_params(None, false);
        assert_eq!(thread["sandbox"].as_str(), Some("read-only"));
        assert_eq!(
            thread["approvalPolicy"]["granular"]["sandbox_approval"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn codex_model_is_optional_in_app_server_payloads() {
        let thread_params = codex_thread_start_params(None, false);
        let turn_params = codex_turn_start_params("thread-1", "hello", None);

        assert!(thread_params.get("model").is_none());
        assert!(turn_params.get("model").is_none());

        let thread_params = codex_thread_start_params(Some("gpt-5-codex"), false);
        let turn_params = codex_turn_start_params("thread-1", "hello", Some("gpt-5-codex"));

        assert_eq!(
            thread_params.get("model").and_then(Value::as_str),
            Some("gpt-5-codex")
        );
        assert_eq!(
            turn_params.get("model").and_then(Value::as_str),
            Some("gpt-5-codex")
        );
    }

    #[test]
    fn codex_agent_prompt_does_not_request_command_cards() {
        let request = AiChatRequest {
            stream_id: None,
            session_id: None,
            connection_id: None,
            terminal_session_id: Some("term-1".to_string()),
            owner_scope: Default::default(),
            targets: vec![],
            target_contexts: vec![],
            mode: crate::config::AiMode::Agent,
            agent_kind: AiAgentKind::Codex,
            permission_mode: crate::config::AiPermissionMode::Confirm,
            model_id: None,
            model_name: None,
            default_target_session_id: Some("term-1".to_string()),
            existing_external_session_id: None,
            attachments: vec![],
            action: super::super::types::AiAction::GenerateCommand,
            user_input: "check load".to_string(),
            context: Default::default(),
            options: Default::default(),
        };

        let prompt = build_codex_agent_prompt(&request, &AiSettings::default());

        assert!(prompt.contains("nyaterm_terminal.execute_command"));
        assert!(prompt.contains("must be non-interactive"));
        assert!(prompt.contains("git --no-pager"));
        assert!(!prompt.contains("commandCards"));
        assert!(!prompt.contains("必须返回 JSON 对象"));
    }

    #[test]
    fn reusable_codex_thread_requires_matching_terminal_tool_version() {
        let old_metadata = AiSessionBackendMetadata {
            backend: AiBackendKind::Codex,
            external_thread_id: Some("thread-old".to_string()),
            codex_terminal_tools_version: None,
            ..Default::default()
        };
        let current_metadata = AiSessionBackendMetadata {
            backend: AiBackendKind::Codex,
            external_thread_id: Some("thread-current".to_string()),
            codex_terminal_tools_version: Some(CODEX_TERMINAL_TOOLS_VERSION),
            ..Default::default()
        };
        let other_backend_metadata = AiSessionBackendMetadata {
            backend: AiBackendKind::Genai,
            external_thread_id: Some("thread-other".to_string()),
            codex_terminal_tools_version: Some(CODEX_TERMINAL_TOOLS_VERSION),
            ..Default::default()
        };

        assert_eq!(reusable_codex_thread_id(Some(&old_metadata)), None);
        assert_eq!(
            reusable_codex_thread_id(Some(&current_metadata)),
            Some("thread-current".to_string())
        );
        assert_eq!(
            reusable_codex_thread_id(Some(&other_backend_metadata)),
            None
        );
    }

    #[test]
    fn detects_codex_command_plan_text() {
        assert!(looks_like_codex_command_plan(
            r#"{"commands":[{"command":"uptime"}]}"#
        ));
        assert!(looks_like_codex_command_plan(
            "```json\n{\"commands\":[{\"command\":\"uptime\"}]}\n```"
        ));
        assert!(!looks_like_codex_command_plan(
            r#"{"text":"ok","commandCards":[]}"#
        ));
        assert!(!looks_like_codex_command_plan("load is normal"));
    }
}
