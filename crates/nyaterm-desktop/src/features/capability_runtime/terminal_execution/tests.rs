use std::time::Duration;

use nyaterm_mcp_protocol::{RpcError, TerminalExecuteResult};
use nyaterm_terminal::agent_capture::AgentOutputCaptureProcessor;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::{TerminalExecutionPolicy, TerminalExecutionRequest, TerminalExecutionState};
use crate::features::terminal::ai_execution::{TerminalExecutionEvent, TerminalExecutionStatus};

fn execution_request(
    id: &str,
    owner: &str,
    session: &str,
    background: bool,
) -> (
    TerminalExecutionRequest,
    oneshot::Receiver<Result<Value, RpcError>>,
) {
    let (reply, receiver) = oneshot::channel();
    (
        TerminalExecutionRequest {
            id: id.into(),
            owner: owner.into(),
            source: owner.into(),
            session_id: session.into(),
            command: "pwd".into(),
            timeout: Duration::from_secs(30),
            cancellation: CancellationToken::new(),
            track_insertion: true,
            policy: TerminalExecutionPolicy {
                background,
                max_lines: 2,
            },
            reply,
        },
        receiver,
    )
}

fn result() -> TerminalExecuteResult {
    TerminalExecuteResult {
        output: "first\nsecond\nthird".into(),
        exit_code: Some(0),
        duration_ms: 0,
        timed_out: false,
        source_truncated: false,
    }
}

#[cfg(windows)]
#[gpui::test]
fn shared_entry_routes_native_codex_claude_and_external_calls_on_a_hidden_terminal(
    cx: &mut gpui::TestAppContext,
) {
    use crate::features::capability_runtime::{CapabilityCaller, CapabilityRequest};
    use crate::features::test_support::app_with_visible_local_session;
    use crate::test_support::TestConfigDir;
    use gpui::AppContext as _;
    use nyaterm_core::{AiExecutionProfile, AiPermissionMode, CapabilityScope};
    use nyaterm_transport::LocalSessionConfig;
    use serde_json::json;
    cx.executor().allow_parking();
    let directory = TestConfigDir::new("nyaterm-shared-terminal-execution");
    let app = app_with_visible_local_session(cx, directory.path(), "visible");
    let manager = cx.update_entity(&app, |app, _| app.session.manager_handle());
    let config = LocalSessionConfig {
        shell_path: Some("cmd.exe".into()),
        shell_args: vec!["/D".into(), "/Q".into()],
        ..LocalSessionConfig::default()
    };
    let session = manager.create_local_session(config).expect("Windows PTY");
    manager
        .write(&session.id, b"set NYATERM_CAPTURE_FIXTURE=current-pty\r")
        .unwrap();
    cx.update_entity(&app, |app, _| {
        let mut metadata = app.session.metadata("visible").unwrap().clone();
        metadata.ai_execution_profile = AiExecutionProfile::Cmd;
        app.session.register_session_metadata(&session.id, metadata);
    });
    let mut credentials = Vec::new();
    for (source, background) in [
        ("NyaTerm Agent", false),
        ("Codex", false),
        ("Claude Code", true),
        ("external MCP", false),
    ] {
        let mut receiver = cx.update_entity(&app, |app, cx| {
            let policy = TerminalExecutionPolicy { background, max_lines: 2 };
            let native = source == "NyaTerm Agent";
            let integrated = matches!(source, "Codex" | "Claude Code");
            let mut settings = app.ai.settings_config_cloned();
            settings.agent_background_execution_enabled = background;
            settings.terminal_output_lines = 2;
            app.ai.replace_settings_config(settings, true);
            let caller = if native {
                app.ai.begin_native_run(serde_json::from_value(json!({"mode":"agent","action":"generate_command","userInput":"fixture"})).unwrap());
                CapabilityCaller::Native { run_id: "fixture".into(), ai_session_id: app.ai.chat_session_id().into(), model_risk: None }
            } else { CapabilityCaller::Mcp };
            let generation = if integrated {
                let credential = app.create_ephemeral_mcp_credential(vec![session.id.clone()], Some(session.id.clone()), AiPermissionMode::FullAccess, policy).unwrap();
                let generation = credential.environment().remove("NYATERM_MCP_GENERATION").unwrap();
                credentials.push(credential);
                generation
            } else { "external-generation".into() };
            // The visible settings and scope must not identify an integrated caller.
            let mut settings = app.ai.settings_config_cloned();
            settings.agent_background_execution_enabled = !background;
            app.ai.replace_settings_config(settings, true);
            let (reply, receiver) = oneshot::channel();
            app.dispatch_shared_terminal_execute(CapabilityRequest {
                connection_id: source.into(), request_id: source.into(), generation,
                client: source.into(), permission_mode: AiPermissionMode::FullAccess,
                scope: CapabilityScope::AllSessions,
                tool: nyaterm_mcp_protocol::tool::TERMINAL_EXECUTE.into(),
                arguments: json!({"sessionId":session.id,"command":if background { "echo background-value" } else { "echo %NYATERM_CAPTURE_FIXTURE%" },"timeoutMs":10000}),
                cancellation: CancellationToken::new(), approved: true, approval_decision: None, caller, reply,
            }, session.id.clone(), cx);
            assert_eq!(app.mcp.capabilities.terminal_executions.tasks[&session.id].request.policy.background, background);
            if !background {
                assert!(app.write_session_input_recorded(&session.id, b"interference\r").is_err());
                assert!(app.write_session_raw_input_recorded(&session.id, b"interference\r").is_err());
            }
            app.ai.switch_scope("terminal:unrelated");
            receiver
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let result = loop {
            cx.run_until_parked();
            cx.update_entity(&app, |app, cx| {
                app.drain_session_events(cx);
                app.drain_terminal_frame_events(cx);
                app.poll_terminal_executions(cx);
            });
            if let Ok(result) = receiver.try_recv() {
                break result.unwrap();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "execution did not finish"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(result["exitCode"], 0, "{source}: {result}");
        assert!(
            !result["output"].as_str().unwrap().contains("__DF_CMD_"),
            "{source}: captured an internal marker"
        );
        assert!(
            result["output"].as_str().unwrap().contains(if background {
                "background-value"
            } else {
                "current-pty"
            }),
            "{source}"
        );
        cx.update_entity(&app, |app, _| {
            assert!(!app.mcp.capabilities.terminal_executions.busy(&session.id));
            assert_eq!(
                app.mcp
                    .capabilities
                    .terminal_executions
                    .take_inserted(source),
                !background
            );
        });
    }
    let text = cx.update_entity(&app, |app, _| {
        app.terminal.request_frame_text_for_test(&session.id)
    });
    let text = futures::executor::block_on(text).unwrap().unwrap();
    assert_eq!(text.matches("┌ AI #").count(), 3);
    assert_eq!(text.matches("└").count(), 3);
    assert!(
        !text.contains("__DF_CMD_"),
        "internal marker leaked: start={}, end={}",
        text.matches("__DF_CMD_START_").count(),
        text.matches("__DF_CMD_END_").count()
    );
    assert!(!text.contains("background-value"));
    let mut send_only = cx.update_entity(&app, |app, cx| {
        app.session
            .metadata_mut(&session.id)
            .unwrap()
            .ai_execution_profile = AiExecutionProfile::SendOnly;
        let (mut request, receiver) = execution_request("send-only", "external", &session.id, true);
        request.command = "echo send-only-fixture".into();
        app.start_terminal_execution(request, cx);
        assert!(!app.mcp.capabilities.terminal_executions.busy(&session.id));
        assert!(
            app.mcp
                .capabilities
                .terminal_executions
                .take_inserted("send-only")
        );
        receiver
    });
    let sent = send_only.try_recv().unwrap().unwrap();
    assert!(sent["exitCode"].is_null());
    assert!(sent["output"].as_str().unwrap().contains("Command sent"));
    let powershell = manager
        .create_local_session(LocalSessionConfig {
            shell_path: Some("powershell.exe".into()),
            shell_args: vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NoExit".into(),
                "-Command".into(),
                // The prompt is emitted after startup, unlike process creation or
                // output from the initialization command itself.
                "$nyatermCaptureFixture = 'powershell-pty'; function global:prompt { 'nyaterm-powershell-ready> ' }".into(),
            ],
            ..LocalSessionConfig::default()
        })
        .unwrap();
    cx.update_entity(&app, |app, _| {
        let mut metadata = app.session.metadata("visible").unwrap().clone();
        metadata.ai_execution_profile = AiExecutionProfile::Powershell;
        app.session
            .register_session_metadata(&powershell.id, metadata);
    });
    let startup_deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut frame = cx.update_entity(&app, |app, _| {
        app.terminal.request_frame_text_for_test(&powershell.id)
    });
    let mut saw_output = false;
    loop {
        cx.run_until_parked();
        cx.update_entity(&app, |app, cx| {
            app.drain_session_events(cx);
            app.drain_terminal_frame_events(cx);
        });
        if let Some(text) = frame.try_recv().expect("PowerShell frame worker stopped") {
            let text = text.unwrap_or_default();
            saw_output |= !text.is_empty();
            if text.contains("nyaterm-powershell-ready>") {
                break;
            }
            frame = cx.update_entity(&app, |app, _| {
                app.terminal.request_frame_text_for_test(&powershell.id)
            });
        }
        assert!(
            std::time::Instant::now() < startup_deadline,
            "PowerShell startup timed out before its prompt (terminal output received: {saw_output})"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let execution_timeout = Duration::from_secs(30);
    let mut powershell_reply = cx.update_entity(&app, |app, cx| {
        let (mut request, receiver) =
            execution_request("powershell", "external", &powershell.id, false);
        request.command = "Write-Output $nyatermCaptureFixture".into();
        request.timeout = execution_timeout;
        request.policy.max_lines = 0;
        app.start_terminal_execution(request, cx);
        receiver
    });
    // Let the execution layer report its own timeout before the test watchdog.
    let deadline = std::time::Instant::now() + execution_timeout + Duration::from_secs(5);
    let powershell_result = loop {
        cx.run_until_parked();
        cx.update_entity(&app, |app, cx| {
            app.drain_session_events(cx);
            app.drain_terminal_frame_events(cx);
            app.poll_terminal_executions(cx);
        });
        match powershell_reply.try_recv() {
            Ok(result) => break result.expect("PowerShell execution failed after readiness"),
            Err(oneshot::error::TryRecvError::Closed) => {
                panic!("PowerShell execution reply closed after readiness");
            }
            Err(oneshot::error::TryRecvError::Empty) => {}
        }
        assert!(
            std::time::Instant::now() < deadline,
            "PowerShell execution did not reply after its request timeout (startup completed)"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        powershell_result["timedOut"], false,
        "PowerShell execution timed out after its prompt was ready"
    );
    assert_eq!(powershell_result["exitCode"], 0);
    assert!(
        powershell_result["output"]
            .as_str()
            .unwrap()
            .contains("powershell-pty")
    );
    let text = cx.update_entity(&app, |app, _| {
        app.terminal.request_frame_text_for_test(&powershell.id)
    });
    let text = futures::executor::block_on(text)
        .unwrap()
        .unwrap_or_default();
    assert!(!text.contains("AI #") && !text.contains("__DF_CMD_"));
    manager.close(&powershell.id).unwrap();
    manager.close(&session.id).unwrap();
    drop(credentials);
    let cleanup = cx.update_entity(&app, |app, cx| app.shutdown_blocking_jobs(cx));
    cx.foreground_executor().block_test(cleanup);
    cx.run_until_parked();
}

#[test]
fn reusing_an_rpc_id_does_not_let_a_late_worker_finish_a_new_execution() {
    let mut state = TerminalExecutionState::default();
    let (request, _receiver) = execution_request("same-rpc-id", "external", "a", true);
    assert!(state.insert(request, None).is_ok());
    state.tasks.get_mut("a").unwrap().capture_marker_id = "first-internal-id".into();
    state
        .finish(
            "a",
            "first-internal-id",
            Ok(result()),
            TerminalExecutionStatus::TimedOut,
        )
        .unwrap();
    let (request, _receiver) = execution_request("same-rpc-id", "external", "a", true);
    assert!(state.insert(request, None).is_ok());
    state.tasks.get_mut("a").unwrap().capture_marker_id = "second-internal-id".into();
    assert!(
        state
            .finish(
                "a",
                "first-internal-id",
                Ok(result()),
                TerminalExecutionStatus::Completed
            )
            .is_none()
    );
    assert!(state.busy("a"));
    assert!(
        state
            .finish(
                "a",
                "second-internal-id",
                Ok(result()),
                TerminalExecutionStatus::Completed
            )
            .is_some()
    );
}

#[test]
fn output_before_the_boundary_is_preserved_and_stale_boundaries_do_nothing() {
    let mut state = TerminalExecutionState::default();
    let (request, _receiver) = execution_request("new", "owner", "a", false);
    let mut capture = AgentOutputCaptureProcessor::new();
    capture.register("new".into());
    assert!(state.insert(request, Some(capture)).is_ok());
    assert_eq!(
        state.process_output("a", "previous output\n").visible_text,
        "previous output\n"
    );
    assert!(state.begin_capture("a", "old").is_none());
    assert!(state.begin_capture("a", "new").is_some());
    assert!(state.begin_capture("a", "new").is_none());
    assert!(
        state
            .process_output("a", "wrapper echo\n__DF_CMD_START_new__\nhello")
            .visible_text
            .is_empty()
    );
    let cancelled = state
        .finish("a", "new", Ok(result()), TerminalExecutionStatus::TimedOut)
        .unwrap();
    assert!(
        cancelled.result.unwrap()["output"]
            .as_str()
            .unwrap()
            .contains("hello")
    );
    assert!(state.begin_capture("a", "new").is_none());
}

#[test]
fn a_prompt_from_the_previous_task_cannot_overtake_the_next_start_boundary() {
    let mut state = TerminalExecutionState::default();
    let (request, _receiver) = execution_request("old", "owner", "a", false);
    let mut capture = AgentOutputCaptureProcessor::new();
    capture.register("old".into());
    assert!(state.insert(request, Some(capture)).is_ok());
    state.begin_capture("a", "old");
    state.process_output("a", "__DF_CMD_START_old__\nok\n__DF_CMD_END_old_0__\n");
    state
        .finish("a", "old", Ok(result()), TerminalExecutionStatus::Completed)
        .unwrap();
    let (request, _receiver) = execution_request("new", "owner", "a", false);
    let mut capture = AgentOutputCaptureProcessor::new();
    capture.register("new".into());
    assert!(state.insert(request, Some(capture)).is_ok());
    assert!(state.process_output("a", "PS>").visible_text.is_empty());
    assert!(state.begin_capture("a", "new").is_some());
    assert!(
        state
            .process_output("a", "wrapper echo")
            .visible_text
            .is_empty()
    );
}

#[test]
fn retired_markers_remain_hidden_after_cancellation_and_a_new_task() {
    let mut state = TerminalExecutionState::default();
    let (request, _receiver) = execution_request("old", "owner", "a", false);
    let mut capture = AgentOutputCaptureProcessor::new();
    capture.register("old".into());
    assert!(state.insert(request, Some(capture)).is_ok());
    state.begin_capture("a", "old");
    state.process_output("a", "__DF_CMD_START_old__\npartial\n__DF_CMD_EN");
    state
        .finish("a", "old", Ok(result()), TerminalExecutionStatus::Cancelled)
        .unwrap();
    assert!(!state.capturing("a"));
    assert_eq!(
        state.process_output("a", "D_old_0__\nnormal").visible_text,
        "\nnormal"
    );
    assert!(!state.intercepting("a"));
    let (request, _receiver) = execution_request("new", "owner", "a", false);
    let mut capture = AgentOutputCaptureProcessor::new();
    capture.register("new".into());
    assert!(state.insert(request, Some(capture)).is_ok());
    state.begin_capture("a", "new");
    assert!(
        state
            .finish("a", "old", Ok(result()), TerminalExecutionStatus::Completed)
            .is_none()
    );
    assert!(state.busy("a"));
}

#[test]
fn all_callers_share_target_occupancy_and_other_targets_can_run_in_parallel() {
    let mut state = TerminalExecutionState::default();
    let (native, _native_reply) = execution_request("native", "NyaTerm", "a", false);
    let (codex, _codex_reply) = execution_request("codex", "Codex", "a", true);
    let (claude, _claude_reply) = execution_request("claude", "Claude Code", "b", false);
    assert!(state.insert(native, None).is_ok());
    assert!(state.insert(codex, None).is_err());
    assert!(state.insert(claude, None).is_ok());
    assert!(state.busy("a") && state.busy("b"));
}

#[test]
fn generations_keep_separate_counters_and_execution_policies() {
    let mut state = TerminalExecutionState::default();
    for (id, owner, session, background, expected_step) in [
        ("a1", "codex-run", "a", false, 0),
        ("b1", "claude-run", "b", true, 0),
        ("a2", "codex-run", "c", false, 1),
    ] {
        let (request, _receiver) = execution_request(id, owner, session, background);
        let event = state
            .insert(request, None)
            .unwrap_or_else(|_| panic!("free target"));
        let TerminalExecutionEvent::Started { step_index, .. } = event else {
            panic!("start event");
        };
        assert_eq!(step_index, expected_step);
        assert_eq!(state.tasks[session].request.policy.background, background);
    }
    state.forget_owner("codex-run");
    assert!(state.tasks["a"].request.cancellation.is_cancelled());
    assert!(!state.tasks["b"].request.cancellation.is_cancelled());
}

#[test]
fn completion_replies_once_keeps_full_output_and_ignores_late_results() {
    let mut state = TerminalExecutionState::default();
    let (request, mut receiver) = execution_request("one", "external", "a", false);
    let mut capture = AgentOutputCaptureProcessor::new();
    capture.register("one".into());
    assert!(state.insert(request, Some(capture)).is_ok());
    assert!(state.begin_capture("a", "one").is_some());
    let captured = state.process_output(
        "a",
        "__DF_CMD_START_one__\nfirst\nsecond\nthird\n__DF_CMD_END_one_0__\nPS>",
    );
    assert_eq!(captured.completed.len(), 1);
    let completion = state
        .finish("a", "one", Ok(result()), TerminalExecutionStatus::Completed)
        .unwrap();
    assert!(matches!(
        completion.event,
        Some(TerminalExecutionEvent::Finished { max_lines: 2, .. })
    ));
    completion.reply.send(completion.result).unwrap();
    assert_eq!(
        receiver.try_recv().unwrap().unwrap()["output"],
        "first\nsecond\nthird"
    );
    assert!(
        state
            .finish("a", "one", Ok(result()), TerminalExecutionStatus::Completed)
            .is_none()
    );
    let (next, _receiver) = execution_request("two", "external", "a", false);
    assert!(state.insert(next, None).is_ok());
    assert!(
        state
            .finish("a", "one", Ok(result()), TerminalExecutionStatus::Completed)
            .is_none()
    );
    assert!(state.busy("a"));
}

#[test]
fn every_terminal_outcome_releases_capture_and_background_never_draws_a_block() {
    for status in [
        TerminalExecutionStatus::Failed,
        TerminalExecutionStatus::TimedOut,
        TerminalExecutionStatus::Cancelled,
        TerminalExecutionStatus::Disconnected,
    ] {
        let mut state = TerminalExecutionState::default();
        let (request, _receiver) = execution_request("task", "owner", "a", false);
        let mut capture = AgentOutputCaptureProcessor::new();
        capture.register("task".into());
        assert!(state.insert(request, Some(capture)).is_ok());
        assert!(state.begin_capture("a", "task").is_some());
        state.process_output("a", "__DF_CMD_START_task__\npartial");
        assert!(state.capturing("a"));
        let completed = state.finish("a", "task", Ok(result()), status).unwrap();
        assert!(
            matches!(completed.event, Some(TerminalExecutionEvent::Finished { status: ended, .. }) if ended == status)
        );
        assert!(!state.busy("a") && !state.capturing("a"));
        assert_eq!(
            state.process_output("a", "normal output").visible_text,
            "normal output"
        );
    }
    let mut state = TerminalExecutionState::default();
    let (request, _receiver) = execution_request("background", "owner", "b", true);
    assert!(state.insert(request, None).is_ok());
    assert!(
        state
            .finish(
                "b",
                "background",
                Ok(result()),
                TerminalExecutionStatus::Completed
            )
            .unwrap()
            .event
            .is_none()
    );
}

#[test]
fn cancellation_wins_a_racing_success_and_insertion_audit_is_consumed_once() {
    let mut state = TerminalExecutionState::default();
    let (request, _receiver) = execution_request("race", "owner", "a", true);
    let cancellation = request.cancellation.clone();
    assert!(state.insert(request, None).is_ok());
    cancellation.cancel();
    let completion = state
        .finish(
            "a",
            "race",
            Ok(result()),
            TerminalExecutionStatus::Completed,
        )
        .unwrap();
    assert_eq!(completion.result.unwrap_err().code, "cancelled");
    assert!(!state.take_inserted("race"));
    state.inserted_requests.insert("foreground".into());
    assert!(state.take_inserted("foreground"));
    assert!(!state.take_inserted("foreground"));
}
