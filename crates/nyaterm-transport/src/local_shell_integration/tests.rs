use crate::local_shell::ResolvedLocalShell;

use super::{LocalShellIntegration, interactive_args, startup_script, wsl_target};

fn shell(program: &str, args: &[&str]) -> ResolvedLocalShell {
    ResolvedLocalShell {
        program: program.into(),
        args: args.iter().map(|arg| (*arg).into()).collect(),
    }
}

#[test]
fn commands_scripts_and_unknown_options_are_never_rewritten() {
    for (program, args) in [
        ("bash", vec!["-c", "cat"]),
        ("bash", vec!["-ic", "cat"]),
        ("bash", vec!["script.sh"]),
        ("bash", vec!["--", "script.sh"]),
        ("zsh", vec!["-f"]),
        ("fish", vec!["--command", "cat"]),
        ("fish", vec!["-C", "read"]),
        ("wsl.exe", vec!["--exec", "sh", "-c", "cat"]),
        ("wsl.exe", vec!["--exec", "bash", "-lc", "cat"]),
        ("wsl.exe", vec!["--list"]),
        ("wsl.exe", vec!["--distribution"]),
        ("wsl.exe", vec!["bash", "-c", "cat"]),
        ("mybash", vec![]),
        ("cmd.exe", vec![]),
    ] {
        assert!(
            LocalShellIntegration::prepare(&shell(program, &args), "test").is_none(),
            "{program}"
        );
    }
}

#[test]
fn wsl_preserves_selectors_and_the_explicit_shell() {
    let args = shell(
        "wsl",
        &[
            "-d",
            "Test distro",
            "-u",
            "user",
            "--cd",
            "/tmp",
            "--exec",
            "/usr/bin/fish",
            "--no-config",
            "-i",
        ],
    )
    .args;
    let (prefix, target, flags) = wsl_target(&args).unwrap();
    assert_eq!(prefix, ["-d", "Test distro", "-u", "user", "--cd", "/tmp"]);
    assert_eq!(target, "/usr/bin/fish");
    assert_eq!(flags, ["--no-config", "-i"]);
    let (launch, _) = LocalShellIntegration::prepare(&shell("wsl.exe", &[]), "test").unwrap();
    assert_eq!(launch.program, "wsl.exe");
    assert_eq!(&launch.args[..3], ["--exec", "/bin/sh", "-c"]);
    assert_eq!(launch.args.last().unwrap(), "");
}

#[test]
fn known_interactive_flags_remain_supported() {
    for (kind, flags) in [
        ("bash", vec!["--noprofile", "--norc", "-i"]),
        ("bash", vec!["--login", "-i"]),
        ("zsh", vec!["-il"]),
        ("fish", vec!["--no-config", "--interactive"]),
    ] {
        assert!(interactive_args(kind, &shell(kind, &flags).args));
    }
}

#[test]
fn startup_prompts_errors_and_terminal_queries_are_visible_without_ready_marker() {
    let (_, mut integration) =
        LocalShellIntegration::prepare(&shell("wsl.exe", &[]), "test").unwrap();
    for data in [
        b"startup error\r\n".as_slice(),
        b"Continue? ",
        b"\x1b[6n",
        b"last output before EOF",
    ] {
        assert_eq!(integration.filter(data).visible, data);
    }
    let output = integration.filter(b"\x1b]7;file://localhost/tmp/My%20Files\x07prompt> ");
    assert_eq!(output.cwd_paths, ["/tmp/My Files"]);
    assert_eq!(output.visible, b"prompt> ");
}

#[test]
fn startup_script_fits_windows_command_line_and_enables_only_cwd() {
    let script = startup_script();
    assert!(script.len() < 12_000);
    assert!(!script.contains("133;"));
    assert!(!script.contains("NyaTermCommand"));
    assert!(!script.contains('\r'));
}
