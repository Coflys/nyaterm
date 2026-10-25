//! Windows ConPTY regressions. Run explicitly on a host with WSL installed.
#![cfg(windows)]

use std::time::{Duration, Instant};

use nyaterm_terminal::TerminalCore;
use nyaterm_transport::{LocalSessionConfig, SessionEvent, SessionManager};

fn observe(args: &[&str], input: Option<&[u8]>) -> (bool, bool) {
    let manager = SessionManager::new();
    let session = manager
        .create_local_session(LocalSessionConfig {
            shell_path: Some("wsl.exe".into()),
            shell_args: args.iter().map(|arg| (*arg).into()).collect(),
            working_dir: Some("/tmp".into()),
            ..Default::default()
        })
        .expect("WSL session");
    let mut terminal = TerminalCore::new(80, 24);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut cwd_seen = false;
    let mut marker_seen = false;
    let mut sent_input = false;
    while Instant::now() < deadline {
        for event in manager.drain_events(64).unwrap().events {
            match event {
                SessionEvent::Output { data, .. } => {
                    terminal.advance(&data);
                    for reply in terminal.take_effects().pty_write {
                        let _ = manager.write_raw(&session.id, &reply);
                    }
                }
                SessionEvent::CwdChanged { cwd, .. } => cwd_seen |= cwd == "/tmp",
                _ => {}
            }
        }
        marker_seen |= terminal
            .lines()
            .iter()
            .any(|line| line.contains("NYATERM_INPUT_OK"));
        // The child's read prompt is visible before any input is supplied.
        if !sent_input
            && terminal
                .lines()
                .iter()
                .any(|line| line.contains("NYATERM_WAITING"))
            && let Some(input) = input
        {
            std::thread::sleep(Duration::from_millis(700));
            manager.write_raw(&session.id, input).unwrap();
            sent_input = true;
        }
        if marker_seen || (input.is_none() && cwd_seen) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    manager.close(&session.id).unwrap();
    (cwd_seen, marker_seen)
}

#[test]
#[ignore = "requires WSL with Bash and Fish; does not read user rc files"]
fn explicit_interactive_shells_report_cwd_without_stdin_injection() {
    for args in [
        vec!["--exec", "bash", "--noprofile", "--norc", "-i"],
        vec!["--exec", "fish", "--no-config", "--interactive"],
    ] {
        assert!(observe(&args, None).0, "interactive shell must report cwd");
    }
}

#[test]
#[ignore = "requires WSL; does not read user rc files"]
fn noninteractive_commands_preserve_output_and_receive_only_user_input() {
    let script = "printf 'NYATERM_WAITING\\n'; IFS= read -r value; if [ \"$value\" = user-input ]; then printf 'NYATERM_INPUT_OK\\n'; fi; sleep 1";
    assert!(observe(&["--exec", "sh", "-c", script], Some(b"user-input\r")).1);
    assert!(
        observe(
            &[
                "--exec",
                "sh",
                "-c",
                "printf 'NYATERM_INPUT_OK\\n'; sleep 1"
            ],
            None
        )
        .1
    );
}

#[test]
#[ignore = "requires WSL with Python, Bash, Zsh and Fish; uses isolated temporary HOME"]
fn native_posix_startup_preserves_rc_prompts_hooks_and_cleanup() {
    let result = std::process::Command::new("wsl.exe")
        .args([
            "--exec",
            "python3",
            "-c",
            include_str!("local_shell_startup.py"),
        ])
        .arg(include_str!("../src/local_shell_integration/startup.sh").replace("\r\n", "\n"))
        .output()
        .expect("WSL Python test runner");
    assert!(
        result.status.success(),
        "isolated startup checks: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
#[ignore = "set NYATERM_TEST_POSIX_SHELL to the Git Bash executable"]
fn git_bash_reports_cwd_through_conpty() {
    let shell = std::env::var("NYATERM_TEST_POSIX_SHELL").expect("Git Bash executable");
    let directory = tempfile::tempdir().unwrap();
    let manager = SessionManager::new();
    let session = manager
        .create_local_session(LocalSessionConfig {
            shell_path: Some(shell),
            shell_args: vec!["--noprofile".into(), "--norc".into(), "-i".into()],
            working_dir: Some(directory.path().into()),
            ..Default::default()
        })
        .unwrap();
    let mut terminal = TerminalCore::new(80, 24);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut matched = false;
    let mut cwd_count = 0;
    let mut mapped_count = 0;
    let mut output_bytes = 0;
    let mut error_seen = false;
    while Instant::now() < deadline && !matched {
        for event in manager.drain_events(64).unwrap().events {
            match event {
                SessionEvent::Output { data, .. } => {
                    output_bytes += data.len();
                    error_seen |= String::from_utf8_lossy(&data).contains("not found");
                    terminal.advance(&data);
                    for reply in terminal.take_effects().pty_write {
                        let _ = manager.write_raw(&session.id, &reply);
                    }
                }
                SessionEvent::CwdChanged { cwd, .. } => {
                    cwd_count += 1;
                    let style = manager.local_shell_path_style(&session.id).unwrap();
                    let mapped = nyaterm_transport::local_shell_cwd_to_browser_path(style, &cwd);
                    mapped_count += usize::from(mapped.is_some());
                    matched = mapped.is_some_and(|mapped| {
                        nyaterm_transport::file_browser_path_eq(
                            nyaterm_transport::FileBrowserBackendKind::Local,
                            &mapped,
                            &directory.path().to_string_lossy(),
                        )
                    });
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    manager.close(&session.id).unwrap();
    assert!(
        matched,
        "Git Bash cwd must resolve to the actual Windows directory: cwd_count={cwd_count}, mapped_count={mapped_count}, output_bytes={output_bytes}, error_seen={error_seen}"
    );
}
