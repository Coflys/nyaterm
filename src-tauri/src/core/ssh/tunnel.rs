//! Independent SSH tunnel supervisors for local, remote and SOCKS forwarding.

use super::RemoteForwardOpen;
use super::client::TunnelDisconnect;
use super::session::{TunnelConnections, create_ssh_handle_for_tunnel};
use crate::config::{self, TunnelConfig};
use crate::error::{AppError, AppResult};
use crate::observability::{StructuredLog, StructuredLogLevel, log_rate_limited};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TunnelRuntimeStatus {
    Stopped,
    Starting,
    Running,
    Reconnecting,
    Disconnected,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TunnelRuntimeState {
    pub tunnel_id: String,
    pub status: TunnelRuntimeStatus,
    pub error: Option<String>,
    pub updated_at: Option<i64>,
    pub revision: u64,
    pub retry_attempt: u32,
    pub next_retry_at: Option<i64>,
}

struct TunnelRuntimeEntry {
    generation: u64,
    state: TunnelRuntimeState,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl TunnelRuntimeEntry {
    fn has_live_task(&self) -> bool {
        !self.cancel.is_cancelled()
            && matches!(
                self.state.status,
                TunnelRuntimeStatus::Starting
                    | TunnelRuntimeStatus::Running
                    | TunnelRuntimeStatus::Reconnecting
            )
            && self.task.as_ref().is_some_and(|task| !task.is_finished())
    }
}

#[derive(Default)]
struct Runtime {
    entries: HashMap<String, TunnelRuntimeEntry>,
    suppressed_auto_open: HashSet<String>,
}

impl Runtime {
    fn apply(&mut self, generation: u64, state: TunnelRuntimeState) -> Option<&TunnelRuntimeState> {
        let entry = self.entries.get_mut(&state.tunnel_id)?;
        if entry.generation != generation {
            return None;
        }
        entry.state = state;
        Some(&entry.state)
    }
}

#[derive(Clone, Default)]
pub struct TunnelManager {
    runtime: Arc<Mutex<Runtime>>,
    generation: Arc<AtomicU64>,
    // Serialize config read/modify/write and activation, never network I/O.
    pub(crate) control: Arc<Mutex<()>>,
}

impl TunnelManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn active_count(&self) -> usize {
        self.runtime
            .lock()
            .await
            .entries
            .values()
            .filter(|entry| entry.has_live_task())
            .count()
    }

    pub async fn runtime_states(&self, tunnels: &[TunnelConfig]) -> Vec<TunnelRuntimeState> {
        let runtime = self.runtime.lock().await;
        tunnels
            .iter()
            .map(|tunnel| {
                runtime.entries.get(&tunnel.id).map_or_else(
                    || {
                        empty_state(
                            &tunnel.id,
                            if tunnel.is_open {
                                TunnelRuntimeStatus::Disconnected
                            } else {
                                TunnelRuntimeStatus::Stopped
                            },
                        )
                    },
                    |entry| entry.state.clone(),
                )
            })
            .collect()
    }

    pub async fn open(&self, tunnel: &TunnelConfig, app: &AppHandle) -> AppResult<()> {
        self.ensure_running(tunnel, app, true).await;
        Ok(())
    }

    async fn ensure_running(&self, tunnel: &TunnelConfig, app: &AppHandle, manual: bool) {
        let manager = self.clone();
        let task_app = app.clone();
        let config = tunnel.clone();
        self.ensure_task(
            &tunnel.id,
            manual,
            move |generation, cancel, previous| {
                tokio::spawn(async move {
                    if let Some(previous) = previous {
                        let _ = previous.await;
                    }
                    manager
                        .supervise(&task_app, &config, generation, cancel, manual)
                        .await;
                })
            },
            |state| emit_runtime_state(app, state),
        )
        .await;
    }

    async fn ensure_task<F, E>(&self, id: &str, manual: bool, spawn: F, emit: E)
    where
        F: FnOnce(u64, CancellationToken, Option<JoinHandle<()>>) -> JoinHandle<()>,
        E: FnOnce(&TunnelRuntimeState),
    {
        let mut runtime = self.runtime.lock().await;
        if manual {
            runtime.suppressed_auto_open.remove(id);
        } else if runtime.suppressed_auto_open.contains(id)
            || runtime
                .entries
                .get(id)
                .is_some_and(|entry| entry.state.status == TunnelRuntimeStatus::Error)
        {
            return;
        }
        if runtime
            .entries
            .get(id)
            .is_some_and(TunnelRuntimeEntry::has_live_task)
        {
            return;
        }
        let previous = runtime.entries.get_mut(id).and_then(|entry| {
            entry.cancel.cancel();
            entry.task.take()
        });
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = CancellationToken::new();
        let mut state = empty_state(id, TunnelRuntimeStatus::Starting);
        state.revision = generation;
        let task = spawn(generation, cancel.clone(), previous);
        emit(&state);
        runtime.entries.insert(
            id.to_string(),
            TunnelRuntimeEntry {
                generation,
                state,
                cancel,
                task: Some(task),
            },
        );
    }

    /// Request stop immediately. Retain the join handle as a resource-release barrier
    /// for a subsequent open, including delete/recreate of the same ID.
    pub async fn close(&self, app: &AppHandle, tunnel_id: &str) {
        self.stop(tunnel_id, true, |state| emit_runtime_state(app, state))
            .await;
    }

    pub async fn close_for_update(&self, app: &AppHandle, tunnel_id: &str) {
        self.stop(tunnel_id, false, |state| emit_runtime_state(app, state))
            .await;
    }

    async fn stop<E: FnOnce(&TunnelRuntimeState)>(
        &self,
        tunnel_id: &str,
        suppress_auto_open: bool,
        emit: E,
    ) {
        let mut runtime = self.runtime.lock().await;
        if suppress_auto_open {
            runtime.suppressed_auto_open.insert(tunnel_id.to_string());
        }
        if let Some(entry) = runtime.entries.get_mut(tunnel_id) {
            entry.cancel.cancel();
            entry.generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            entry.state = empty_state(tunnel_id, TunnelRuntimeStatus::Stopped);
            entry.state.revision = entry.generation;
            emit(&entry.state);
        } else {
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            let mut state = empty_state(tunnel_id, TunnelRuntimeStatus::Stopped);
            state.revision = generation;
            emit(&state);
            runtime.entries.insert(
                tunnel_id.to_string(),
                TunnelRuntimeEntry {
                    generation,
                    state,
                    cancel: CancellationToken::new(),
                    task: None,
                },
            );
        }
    }

    pub async fn delete_runtime_state(&self, app: &AppHandle, tunnel_id: &str) {
        // Keep a tombstone until cleanup completes so ID reuse cannot race a listener.
        self.close(app, tunnel_id).await;
    }

    async fn publish(&self, app: &AppHandle, generation: u64, mut state: TunnelRuntimeState) {
        let mut runtime = self.runtime.lock().await;
        state.revision = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(state) = runtime.apply(generation, state) {
            emit_runtime_state(app, state);
        }
    }

    async fn supervise(
        &self,
        app: &AppHandle,
        tunnel: &TunnelConfig,
        generation: u64,
        cancel: CancellationToken,
        interactive: bool,
    ) {
        drive_supervisor(
            &tunnel.id,
            tunnel.auto_reconnect,
            cancel.clone(),
            interactive,
            |interactive| {
                self.execute_attempt(app, tunnel, generation, interactive, cancel.clone())
            },
            |state| self.publish(app, generation, state),
        )
        .await;
    }

    async fn execute_attempt(
        &self,
        app: &AppHandle,
        tunnel: &TunnelConfig,
        generation: u64,
        interactive: bool,
        cancel: CancellationToken,
    ) -> AppResult<TunnelTaskExit> {
        let connections = TunnelConnections::default();
        let mut children = JoinSet::new();
        let attempt_cancel = CancellationToken::new();
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => Ok(TunnelTaskExit::Stopped),
            result = self.run_attempt(app, tunnel, generation, interactive, &connections, &mut children, attempt_cancel.clone()) => result,
        };
        children.abort_all();
        while children.join_next().await.is_some() {}
        connections.close(tunnel, Some(&attempt_cancel)).await;
        result
    }

    async fn run_attempt(
        &self,
        app: &AppHandle,
        tunnel: &TunnelConfig,
        generation: u64,
        interactive: bool,
        connections: &TunnelConnections,
        children: &mut JoinSet<()>,
        cancel: CancellationToken,
    ) -> AppResult<TunnelTaskExit> {
        let connection_id = tunnel
            .connection_id
            .as_deref()
            .ok_or_else(|| AppError::Config("Tunnel has no connection_id".into()))?;
        if !matches!(tunnel.tunnel_type.as_str(), "local" | "remote" | "dynamic") {
            return Err(AppError::Config(format!(
                "Unknown tunnel type: {}",
                tunnel.tunnel_type
            )));
        }
        let (disconnect_tx, disconnect_rx) = mpsc::unbounded_channel();
        let (remote_tx, remote_rx) = mpsc::unbounded_channel();
        let handle = create_ssh_handle_for_tunnel(
            app,
            connection_id,
            disconnect_tx,
            (tunnel.tunnel_type == "remote").then_some(remote_tx),
            interactive,
            connections,
            cancel.clone(),
        )
        .await?;
        let target = handle.target_handle();
        let bind = if tunnel.bind_localhost {
            "127.0.0.1"
        } else {
            "0.0.0.0"
        };
        let listener = if tunnel.tunnel_type == "remote" {
            let result = target
                .lock()
                .await
                .tcpip_forward(bind, tunnel.listen_port.into())
                .await;
            result.map_err(|error| match error {
                russh::Error::RequestDenied => AppError::Channel(format!(
                    "Remote port forwarding rejected for {bind}:{}",
                    tunnel.listen_port
                )),
                error => AppError::Ssh(error),
            })?;
            None
        } else {
            Some(
                TcpListener::bind((bind, tunnel.listen_port))
                    .await
                    .map_err(|error| {
                        AppError::Channel(format!(
                            "Failed to bind local port {}: {error}",
                            tunnel.listen_port
                        ))
                    })?,
            )
        };
        self.publish(
            app,
            generation,
            empty_state(&tunnel.id, TunnelRuntimeStatus::Running),
        )
        .await;
        Ok(match tunnel.tunnel_type.as_str() {
            "local" => {
                Self::run_local_tunnel(
                    listener.expect("local listener"),
                    target,
                    tunnel.target_host.clone(),
                    tunnel.target_port,
                    cancel,
                    children,
                    disconnect_rx,
                )
                .await
            }
            "dynamic" => {
                Self::run_dynamic_tunnel(
                    listener.expect("SOCKS listener"),
                    target,
                    cancel,
                    children,
                    disconnect_rx,
                )
                .await
            }
            _ => {
                Self::run_remote_tunnel(
                    tunnel.target_host.clone(),
                    tunnel.target_port,
                    cancel,
                    children,
                    disconnect_rx,
                    remote_rx,
                )
                .await
            }
        })
    }

    async fn run_local_tunnel<H: russh::client::Handler + Send + 'static>(
        listener: TcpListener,
        ssh_handle: Arc<Mutex<russh::client::Handle<H>>>,
        target_host: String,
        target_port: u16,
        cancel: CancellationToken,
        children: &mut JoinSet<()>,
        mut disconnect_rx: mpsc::UnboundedReceiver<TunnelDisconnect>,
    ) -> TunnelTaskExit {
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return TunnelTaskExit::Stopped,
                message = disconnect_rx.recv() => {
                    return TunnelTaskExit::Disconnected(message.unwrap_or_else(|| TunnelDisconnect { message: "SSH connection disconnected".into(), retryable: true }));
                }
                _ = children.join_next(), if !children.is_empty() => {},
                accept = listener.accept() => {
                    match accept {
                        Ok((mut local_stream, peer_addr)) => {
                            let handle_mtx = ssh_handle.clone();
                            let host = target_host.clone();
                            children.spawn(async move {
                                let channel = {
                                    let handle = handle_mtx.lock().await;
                                    match handle.channel_open_direct_tcpip(
                                        &host,
                                        target_port.into(),
                                        &peer_addr.ip().to_string(),
                                        peer_addr.port().into(),
                                    ).await {
                                        Ok(ch) => ch,
                                        Err(e) => {
                                            log_rate_limited(StructuredLog {
                                                level: StructuredLogLevel::Warn,
                                                domain: "session.lifecycle".to_string(),
                                                event: "tunnel.direct_tcpip_failed".to_string(),
                                                message: "Tunnel direct-tcpip failed".to_string(),
                                                ids: None,
                                                data: Some(serde_json::json!({
                                                    "target_host": host,
                                                    "target_port": target_port,
                                                })),
                                                error: Some(serde_json::json!({ "message": e.to_string() })),
                                                client_timestamp: None,
                                            });
                                            return;
                                        }
                                    }
                                };
                                let mut stream = channel.into_stream();
                                let _ = tokio::io::copy_bidirectional(&mut local_stream, &mut stream).await;
                            });
                        }
                        Err(error) => {
                            let message = format!("Tunnel TCP accept failed: {error}");
                            log_rate_limited(StructuredLog {
                                level: StructuredLogLevel::Warn,
                                domain: "session.lifecycle".to_string(),
                                event: "tunnel.accept_failed".to_string(),
                                message: "Tunnel TCP accept failed".to_string(),
                                ids: None,
                                data: None,
                                error: Some(serde_json::json!({ "message": error.to_string() })),
                                client_timestamp: None,
                            });
                            return TunnelTaskExit::Error(message);
                        }
                    }
                }
            }
        }
    }

    async fn run_remote_tunnel(
        target_host: String,
        target_port: u16,
        cancel: CancellationToken,
        children: &mut JoinSet<()>,
        mut disconnect_rx: mpsc::UnboundedReceiver<TunnelDisconnect>,
        mut remote_forward_rx: mpsc::UnboundedReceiver<RemoteForwardOpen>,
    ) -> TunnelTaskExit {
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    return TunnelTaskExit::Stopped;
                }
                message = disconnect_rx.recv() => {
                    return TunnelTaskExit::Disconnected(message.unwrap_or_else(|| TunnelDisconnect { message: "SSH connection disconnected".into(), retryable: true }));
                }
                _ = children.join_next(), if !children.is_empty() => {},
                open = remote_forward_rx.recv() => {
                    let Some(open) = open else {
                        return TunnelTaskExit::Disconnected(TunnelDisconnect { message: "Remote forwarding channel closed".into(), retryable: true });
                    };
                    let host = target_host.clone();
                    children.spawn(async move {
                        Self::handle_remote_forward_connection(open, host, target_port).await;
                    });
                }
            }
        }
    }

    async fn handle_remote_forward_connection(
        open: RemoteForwardOpen,
        target_host: String,
        target_port: u16,
    ) {
        let mut channel_stream = open.channel.into_stream();
        match TcpStream::connect(format!("{}:{}", target_host, target_port)).await {
            Ok(mut target_stream) => {
                let _ =
                    tokio::io::copy_bidirectional(&mut channel_stream, &mut target_stream).await;
            }
            Err(error) => {
                log_rate_limited(StructuredLog {
                    level: StructuredLogLevel::Warn,
                    domain: "session.lifecycle".to_string(),
                    event: "tunnel.remote_target_connect_failed".to_string(),
                    message: "Remote tunnel target connection failed".to_string(),
                    ids: None,
                    data: Some(serde_json::json!({
                        "connected_address": open.connected_address,
                        "connected_port": open.connected_port,
                        "originator_address": open.originator_address,
                        "originator_port": open.originator_port,
                        "target_host": target_host,
                        "target_port": target_port,
                    })),
                    error: Some(serde_json::json!({ "message": error.to_string() })),
                    client_timestamp: None,
                });
            }
        }
    }

    async fn run_dynamic_tunnel<H: russh::client::Handler + Send + 'static>(
        listener: TcpListener,
        ssh_handle: Arc<Mutex<russh::client::Handle<H>>>,
        cancel: CancellationToken,
        children: &mut JoinSet<()>,
        mut disconnect_rx: mpsc::UnboundedReceiver<TunnelDisconnect>,
    ) -> TunnelTaskExit {
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return TunnelTaskExit::Stopped,
                message = disconnect_rx.recv() => {
                    return TunnelTaskExit::Disconnected(message.unwrap_or_else(|| TunnelDisconnect { message: "SSH connection disconnected".into(), retryable: true }));
                }
                _ = children.join_next(), if !children.is_empty() => {},
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, peer_addr)) => {
                            let handle = ssh_handle.clone();
                            children.spawn(Self::handle_socks5_connection(stream, handle, peer_addr));
                        }
                        Err(error) => {
                            let message = format!("SOCKS5 accept failed: {error}");
                            log_rate_limited(StructuredLog {
                                level: StructuredLogLevel::Warn,
                                domain: "session.lifecycle".to_string(),
                                event: "tunnel.socks_accept_failed".to_string(),
                                message: "SOCKS5 accept failed".to_string(),
                                ids: None,
                                data: None,
                                error: Some(serde_json::json!({ "message": error.to_string() })),
                                client_timestamp: None,
                            });
                            return TunnelTaskExit::Error(message);
                        }
                    }
                }
            }
        }
    }

    async fn handle_socks5_connection<H: russh::client::Handler + Send + 'static>(
        mut stream: tokio::net::TcpStream,
        ssh_handle: Arc<Mutex<russh::client::Handle<H>>>,
        peer_addr: std::net::SocketAddr,
    ) {
        let mut buf = [0u8; 2];
        if stream.read_exact(&mut buf).await.is_err() {
            return;
        }
        if buf[0] != 0x05 {
            return;
        }
        let nmethods = buf[1] as usize;
        let mut methods = vec![0u8; nmethods];
        if stream.read_exact(&mut methods).await.is_err() {
            return;
        }
        if stream.write_all(&[0x05, 0x00]).await.is_err() {
            return;
        }

        let mut header = [0u8; 4];
        if stream.read_exact(&mut header).await.is_err() {
            return;
        }
        if header[0] != 0x05 || header[1] != 0x01 {
            let _ = stream
                .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await;
            return;
        }

        let (target_host, target_port) = match header[3] {
            0x01 => {
                let mut addr = [0u8; 4];
                if stream.read_exact(&mut addr).await.is_err() {
                    return;
                }
                let host = format!("{}.{}.{}.{}", addr[0], addr[1], addr[2], addr[3]);
                let mut port_buf = [0u8; 2];
                if stream.read_exact(&mut port_buf).await.is_err() {
                    return;
                }
                (host, u16::from_be_bytes(port_buf))
            }
            0x03 => {
                let mut len = [0u8; 1];
                if stream.read_exact(&mut len).await.is_err() {
                    return;
                }
                let mut domain = vec![0u8; len[0] as usize];
                if stream.read_exact(&mut domain).await.is_err() {
                    return;
                }
                let host = String::from_utf8_lossy(&domain).to_string();
                let mut port_buf = [0u8; 2];
                if stream.read_exact(&mut port_buf).await.is_err() {
                    return;
                }
                (host, u16::from_be_bytes(port_buf))
            }
            0x04 => {
                let mut addr = [0u8; 16];
                if stream.read_exact(&mut addr).await.is_err() {
                    return;
                }
                let host = std::net::Ipv6Addr::from(addr).to_string();
                let mut port_buf = [0u8; 2];
                if stream.read_exact(&mut port_buf).await.is_err() {
                    return;
                }
                (host, u16::from_be_bytes(port_buf))
            }
            _ => return,
        };

        let channel = match {
            let handle = ssh_handle.lock().await;
            handle
                .channel_open_direct_tcpip(
                    &target_host,
                    target_port.into(),
                    &peer_addr.ip().to_string(),
                    peer_addr.port().into(),
                )
                .await
        } {
            Ok(ch) => ch,
            Err(e) => {
                log_rate_limited(StructuredLog {
                    level: StructuredLogLevel::Warn,
                    domain: "session.lifecycle".to_string(),
                    event: "tunnel.socks_direct_tcpip_failed".to_string(),
                    message: "SOCKS5 direct-tcpip failed".to_string(),
                    ids: None,
                    data: Some(serde_json::json!({
                        "target_host": target_host,
                        "target_port": target_port,
                    })),
                    error: Some(serde_json::json!({ "message": e.to_string() })),
                    client_timestamp: None,
                });
                let _ = stream
                    .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await;
                return;
            }
        };

        let _ = stream
            .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await;

        let mut ssh_stream = channel.into_stream();
        let _ = tokio::io::copy_bidirectional(&mut stream, &mut ssh_stream).await;
    }

    pub async fn auto_open_for_connection(&self, app: &AppHandle, connection_id: &str) {
        let _control = self.control.lock().await;
        let Ok(mut tunnels) = config::load_tunnels(app) else {
            return;
        };
        let mut changed = false;
        for tunnel in &mut tunnels {
            if tunnel.connection_id.as_deref() != Some(connection_id)
                || !(tunnel.auto_open || tunnel.is_open)
            {
                continue;
            }
            {
                let runtime = self.runtime.lock().await;
                if runtime.suppressed_auto_open.contains(&tunnel.id)
                    || runtime
                        .entries
                        .get(&tunnel.id)
                        .is_some_and(|entry| entry.state.status == TunnelRuntimeStatus::Error)
                {
                    continue;
                }
            }
            if !tunnel.is_open {
                tunnel.is_open = true;
                changed = true;
            }
        }
        // Persist before scheduling, so a concurrent close cannot be overwritten later.
        if changed && config::save_tunnels(app, &tunnels).is_err() {
            return;
        }
        for tunnel in tunnels
            .iter()
            .filter(|t| t.is_open && t.connection_id.as_deref() == Some(connection_id))
        {
            self.ensure_running(tunnel, app, false).await;
        }
    }
}

enum TunnelTaskExit {
    Stopped,
    Disconnected(TunnelDisconnect),
    Error(String),
}

/// The retry driver is independent of transport and UI, allowing deterministic
/// outage/cancellation tests while each attempt owns its complete cleanup.
async fn drive_supervisor<F, Fut, U, UFut>(
    id: &str,
    auto_reconnect: bool,
    cancel: CancellationToken,
    mut interactive: bool,
    mut run: F,
    mut publish: U,
) where
    F: FnMut(bool) -> Fut,
    Fut: std::future::Future<Output = AppResult<TunnelTaskExit>>,
    U: FnMut(TunnelRuntimeState) -> UFut,
    UFut: std::future::Future<Output = ()>,
{
    let mut attempt = 0u32;
    loop {
        if cancel.is_cancelled() {
            return;
        }
        let result = run(interactive).await;
        if cancel.is_cancelled() {
            return;
        }
        interactive = false;
        let (error, retryable, was_running) = match result {
            Ok(TunnelTaskExit::Stopped) => return,
            Ok(TunnelTaskExit::Disconnected(error)) => (error.message, error.retryable, true),
            Ok(TunnelTaskExit::Error(message)) => (message, false, true),
            Err(error) => (error.to_string(), is_retryable(&error), false),
        };
        if !retryable || !auto_reconnect {
            let mut state = empty_state(
                id,
                if retryable && was_running {
                    TunnelRuntimeStatus::Disconnected
                } else {
                    TunnelRuntimeStatus::Error
                },
            );
            state.error = Some(error);
            publish(state).await;
            return;
        }
        if was_running {
            attempt = 0;
        }
        attempt = attempt.saturating_add(1);
        let delay = retry_delay(attempt);
        let mut state = empty_state(id, TunnelRuntimeStatus::Reconnecting);
        state.error = Some(error);
        state.retry_attempt = attempt;
        state.next_retry_at = Some(now_ms() + delay.as_millis() as i64);
        publish(state.clone()).await;
        tokio::select! {
            biased;
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(delay) => {}
        }
        state.next_retry_at = None;
        state.updated_at = Some(now_ms());
        publish(state).await;
    }
}

fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs((1u64 << attempt.saturating_sub(1).min(5)).min(30))
}

fn is_retryable(error: &AppError) -> bool {
    match error {
        AppError::Io(error) => matches!(
            error.kind(),
            std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::NotConnected
                | std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::NetworkUnreachable
                | std::io::ErrorKind::HostUnreachable
        ),
        AppError::Ssh(error) => is_retryable_ssh(error),
        _ => false,
    }
}

pub(super) fn is_retryable_ssh(error: &russh::Error) -> bool {
    match error {
        russh::Error::IO(error) => is_retryable_io(error),
        russh::Error::Disconnect
        | russh::Error::SendError
        | russh::Error::HUP
        | russh::Error::ConnectionTimeout
        | russh::Error::KeepaliveTimeout
        | russh::Error::InactivityTimeout
        | russh::Error::Elapsed(_) => true,
        _ => false,
    }
}

fn is_retryable_io(error: &std::io::Error) -> bool {
    // Preserve the IO kind without relying on platform-specific error messages.
    is_retryable(&AppError::Io(std::io::Error::from(error.kind())))
}

fn empty_state(id: &str, status: TunnelRuntimeStatus) -> TunnelRuntimeState {
    TunnelRuntimeState {
        tunnel_id: id.to_string(),
        status,
        error: None,
        updated_at: Some(now_ms()),
        revision: 0,
        retry_attempt: 0,
        next_retry_at: None,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn emit_runtime_state(app: &AppHandle, state: &TunnelRuntimeState) {
    let _ = app.emit("tunnel-runtime-state-changed", state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, future::ready, sync::atomic::AtomicUsize};
    use tokio::time::Instant;

    fn offline() -> AppResult<TunnelTaskExit> {
        Err(AppError::Io(std::io::ErrorKind::ConnectionRefused.into()))
    }

    fn disconnected() -> AppResult<TunnelTaskExit> {
        Ok(TunnelTaskExit::Disconnected(TunnelDisconnect {
            message: "offline".into(),
            retryable: true,
        }))
    }

    #[tokio::test(start_paused = true)]
    async fn tunnel_supervisor_retries_initial_failure_indefinitely_with_capped_backoff() {
        let mut calls = Vec::new();
        let mut states = Vec::new();
        drive_supervisor(
            "t",
            true,
            CancellationToken::new(),
            true,
            |interactive| {
                calls.push((Instant::now(), interactive));
                ready(if calls.len() <= 9 {
                    offline()
                } else {
                    Ok(TunnelTaskExit::Stopped)
                })
            },
            |state| {
                states.push(state);
                ready(())
            },
        )
        .await;
        assert_eq!(calls.len(), 10);
        assert!(calls[0].1);
        assert!(calls.iter().skip(1).all(|call| !call.1));
        let delays: Vec<_> = calls
            .windows(2)
            .map(|pair| (pair[1].0 - pair[0].0).as_secs())
            .collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30, 30, 30]);
        assert_eq!(states.len(), 18);
        for (index, pair) in states.chunks_exact(2).enumerate() {
            assert_eq!(pair[0].status, TunnelRuntimeStatus::Reconnecting);
            assert_eq!(pair[0].retry_attempt, index as u32 + 1);
            assert!(pair[0].next_retry_at.is_some());
            assert!(pair[1].next_retry_at.is_none());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tunnel_supervisor_resets_backoff_after_a_running_connection() {
        let mut results = VecDeque::from([
            offline(),
            offline(),
            disconnected(),
            offline(),
            Ok(TunnelTaskExit::Stopped),
        ]);
        let mut attempts = Vec::new();
        drive_supervisor(
            "t",
            true,
            CancellationToken::new(),
            false,
            |_| ready(results.pop_front().unwrap()),
            |state| {
                if state.next_retry_at.is_some() {
                    attempts.push(state.retry_attempt);
                }
                ready(())
            },
        )
        .await;
        assert_eq!(attempts, [1, 2, 1, 2]);
    }

    #[tokio::test(start_paused = true)]
    async fn tunnel_supervisor_stops_on_permanent_errors_and_when_reconnect_is_disabled() {
        for (enabled, result, expected) in [
            (
                true,
                Err(AppError::Auth("rejected".into())),
                TunnelRuntimeStatus::Error,
            ),
            (
                true,
                Err(AppError::Ssh(russh::Error::UnknownKey)),
                TunnelRuntimeStatus::Error,
            ),
            (
                true,
                Err(AppError::Channel("bind failed".into())),
                TunnelRuntimeStatus::Error,
            ),
            (false, disconnected(), TunnelRuntimeStatus::Disconnected),
            (false, offline(), TunnelRuntimeStatus::Error),
        ] {
            let mut result = Some(result);
            let mut states = Vec::new();
            drive_supervisor(
                "t",
                enabled,
                CancellationToken::new(),
                false,
                |_| ready(result.take().expect("must not retry")),
                |state| {
                    states.push(state);
                    ready(())
                },
            )
            .await;
            assert_eq!(states.len(), 1);
            assert_eq!(states[0].status, expected);
            assert_eq!(states[0].retry_attempt, 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tunnel_supervisor_cancel_during_backoff_prevents_another_attempt() {
        let cancel = CancellationToken::new();
        let mut calls = 0;
        let mut states = Vec::new();
        drive_supervisor(
            "t",
            true,
            cancel.clone(),
            false,
            |_| {
                calls += 1;
                ready(offline())
            },
            |state| {
                states.push(state);
                cancel.cancel();
                ready(())
            },
        )
        .await;
        assert_eq!(calls, 1);
        assert_eq!(states.len(), 1);
    }

    async fn start_fake(manager: &TunnelManager, manual: bool, starts: Arc<AtomicUsize>) {
        manager
            .ensure_task(
                "t",
                manual,
                move |_, cancel, previous| {
                    tokio::spawn(async move {
                        if let Some(previous) = previous {
                            let _ = previous.await;
                        }
                        starts.fetch_add(1, Ordering::SeqCst);
                        cancel.cancelled().await;
                        // Model asynchronous listener/connection cleanup.
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    })
                },
                |_| {},
            )
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn tunnel_concurrent_starts_are_idempotent_and_manual_stop_suppresses_auto_open() {
        let manager = TunnelManager::new();
        let starts = Arc::new(AtomicUsize::new(0));
        tokio::join!(
            start_fake(&manager, true, starts.clone()),
            start_fake(&manager, false, starts.clone()),
            start_fake(&manager, true, starts.clone())
        );
        tokio::task::yield_now().await;
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(manager.active_count().await, 1);
        manager.stop("t", true, |_| {}).await;
        start_fake(&manager, false, starts.clone()).await;
        assert_eq!(manager.active_count().await, 0);
        start_fake(&manager, true, starts.clone()).await;
        tokio::task::yield_now().await;
        assert_eq!(
            starts.load(Ordering::SeqCst),
            1,
            "new connection must wait for old cleanup"
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        manager.stop("t", true, |_| {}).await;
    }

    #[tokio::test(start_paused = true)]
    async fn tunnel_stale_generation_cannot_overwrite_stop_or_recreated_tunnel() {
        let manager = TunnelManager::new();
        let starts = Arc::new(AtomicUsize::new(0));
        start_fake(&manager, true, starts.clone()).await;
        let old_generation = manager.runtime.lock().await.entries["t"].generation;
        manager.stop("t", true, |_| {}).await;
        assert!(
            manager
                .runtime
                .lock()
                .await
                .apply(
                    old_generation,
                    empty_state("t", TunnelRuntimeStatus::Running)
                )
                .is_none()
        );
        start_fake(&manager, true, starts).await;
        let mut runtime = manager.runtime.lock().await;
        assert!(runtime.entries["t"].generation > old_generation);
        assert!(
            runtime
                .apply(old_generation, empty_state("t", TunnelRuntimeStatus::Error))
                .is_none()
        );
        runtime.entries["t"].cancel.cancel();
    }

    #[tokio::test]
    async fn tunnel_auto_open_does_not_restart_error_but_manual_retry_does() {
        let manager = TunnelManager::new();
        manager.stop("t", true, |_| {}).await;
        {
            let mut runtime = manager.runtime.lock().await;
            runtime.suppressed_auto_open.clear();
            runtime.entries.get_mut("t").unwrap().state.status = TunnelRuntimeStatus::Error;
        }
        let starts = Arc::new(AtomicUsize::new(0));
        start_fake(&manager, false, starts.clone()).await;
        assert_eq!(manager.active_count().await, 0);
        start_fake(&manager, true, starts).await;
        assert_eq!(manager.active_count().await, 1);
        manager.stop("t", true, |_| {}).await;
    }

    #[tokio::test]
    async fn tunnel_config_save_does_not_suppress_new_auto_open_or_clear_manual_suppression() {
        let manager = TunnelManager::new();
        let starts = Arc::new(AtomicUsize::new(0));
        manager.stop("t", false, |_| {}).await;
        start_fake(&manager, false, starts.clone()).await;
        assert_eq!(manager.active_count().await, 1);
        manager.stop("t", true, |_| {}).await;
        manager.stop("t", false, |_| {}).await;
        start_fake(&manager, false, starts).await;
        assert_eq!(manager.active_count().await, 0);
    }

    #[tokio::test]
    async fn tunnel_stopping_one_supervisor_does_not_stop_another() {
        let manager = TunnelManager::new();
        start_fake(&manager, true, Arc::new(AtomicUsize::new(0))).await;
        manager
            .ensure_task(
                "other",
                false,
                |_, cancel, _| {
                    tokio::spawn(async move {
                        cancel.cancelled().await;
                    })
                },
                |_| {},
            )
            .await;
        assert_eq!(manager.active_count().await, 2);
        manager.stop("t", true, |_| {}).await;
        assert_eq!(manager.active_count().await, 1);
        manager.stop("other", true, |_| {}).await;
    }

    #[test]
    fn tunnel_error_classification_preserves_security_and_configuration_failures() {
        assert!(is_retryable(&AppError::Ssh(russh::Error::KeepaliveTimeout)));
        assert!(is_retryable(&AppError::Ssh(russh::Error::IO(
            std::io::ErrorKind::ConnectionReset.into()
        ))));
        for error in [
            AppError::Ssh(russh::Error::UnknownKey),
            AppError::Ssh(russh::Error::RequestDenied),
            AppError::Auth("timeout".into()),
            AppError::Config("invalid".into()),
            AppError::Io(std::io::ErrorKind::AddrInUse.into()),
        ] {
            assert!(!is_retryable(&error));
        }
    }
}

#[cfg(test)]
mod forwarding_tests;
