//! Local integration is installed through shell startup hooks, never PTY input.
//! Unrecognized launch modes retain their original command and byte stream.

use std::path::{Path, PathBuf};

use crate::file_browser::LocalShellPathStyle;
use crate::local_shell::ResolvedLocalShell;
use crate::ssh_shell_integration::{
    SshIntegrationOutput, SshShellIntegrationState, build_ssh_ready_marker,
};

#[cfg(all(test, windows))]
mod lifecycle_tests;
#[cfg(test)]
mod tests;

pub(super) struct LocalShellIntegration {
    state: SshShellIntegrationState,
    pub(super) path_style: LocalShellPathStyle,
    pub(super) startup_files: Option<tempfile::TempDir>,
}

impl LocalShellIntegration {
    pub(super) fn prepare(
        shell: &ResolvedLocalShell,
        session_id: &str,
    ) -> Option<(ResolvedLocalShell, Self)> {
        let (launch, startup_files) = integrated_launch(shell, session_id)?;
        let path_style = if program_name(&shell.program) == "wsl" {
            LocalShellPathStyle::Wsl
        } else {
            LocalShellPathStyle::from_program(&launch.program)
        };
        Some((
            launch,
            Self {
                path_style,
                startup_files: Some(startup_files),
                // No ready marker or suppression is needed: the script is a startup
                // argument, not echoed PTY input. Startup errors and prompts stay visible.
                state: SshShellIntegrationState::new(
                    None,
                    build_ssh_ready_marker(session_id),
                    None,
                ),
            },
        ))
    }

    pub(super) fn filter(&mut self, bytes: &[u8]) -> SshIntegrationOutput {
        self.state.filter_output(bytes)
    }
}

fn program_name(program: &str) -> String {
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

fn interactive_args(kind: &str, args: &[String]) -> bool {
    // This deliberately accepts only known startup switches. Commands, scripts,
    // option values, '--', and unfamiliar switches must not be rewritten.
    args.iter().all(|arg| {
        matches!(arg.as_str(), "--login" | "--interactive")
            || (kind == "bash" && matches!(arg.as_str(), "--noprofile" | "--norc"))
            || (kind == "fish" && arg == "--no-config")
            || (arg.starts_with('-')
                && arg.len() > 1
                && arg[1..].chars().all(|c| matches!(c, 'i' | 'l')))
    })
}

fn wsl_target(args: &[String]) -> Option<(Vec<String>, String, Vec<String>)> {
    let mut prefix = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if matches!(arg.as_str(), "--exec" | "-e" | "--") {
            let program = args.get(index + 1)?;
            let kind = program_name(program);
            let rest = &args[index + 2..];
            return (matches!(kind.as_str(), "bash" | "zsh" | "fish")
                && interactive_args(&kind, rest))
            .then(|| (prefix, program.clone(), rest.to_vec()));
        }
        if matches!(
            arg.as_str(),
            "--distribution" | "-d" | "--user" | "-u" | "--cd"
        ) {
            prefix.push(arg.clone());
            prefix.push(args.get(index + 1)?.clone());
            index += 2;
        } else if ["--distribution=", "--user=", "--cd="]
            .iter()
            .any(|name| arg.starts_with(name) && arg.len() > name.len())
        {
            prefix.push(arg.clone());
            index += 1;
        } else {
            return None;
        }
    }
    // The guest bootstrap discovers the selected user's login shell. Never
    // infer Bash from the Windows launcher name or the host environment.
    Some((prefix, String::new(), Vec::new()))
}

fn integrated_launch(
    shell: &ResolvedLocalShell,
    session_id: &str,
) -> Option<(ResolvedLocalShell, tempfile::TempDir)> {
    let kind = program_name(&shell.program);
    if kind == "wsl" {
        let (mut args, target, target_args) = wsl_target(&shell.args)?;
        let startup_files = startup_directory(session_id)?;
        args.extend([
            "--exec".into(),
            "/bin/sh".into(),
            "-c".into(),
            startup_script(),
            "nyaterm-local".into(),
            startup_files.path().to_str()?.into(),
            "wsl".into(),
            target,
        ]);
        args.extend(target_args);
        return Some((
            ResolvedLocalShell {
                program: shell.program.clone(),
                args,
            },
            startup_files,
        ));
    }
    if !matches!(kind.as_str(), "bash" | "zsh" | "fish") || !interactive_args(&kind, &shell.args) {
        return None;
    }
    let (bootstrap, target) = native_bootstrap(&shell.program)?;
    let startup_files = startup_directory(session_id)?;
    let mut args = vec![
        "-c".into(),
        startup_script(),
        "nyaterm-local".into(),
        startup_files.path().to_str()?.replace('\\', "/"),
        "native".into(),
        target,
    ];
    args.extend(shell.args.clone());
    Some((
        ResolvedLocalShell {
            program: bootstrap,
            args,
        },
        startup_files,
    ))
}

fn startup_directory(session_id: &str) -> Option<tempfile::TempDir> {
    // Created before spawning, so every failure and session teardown has a host
    // owner even if TerminateProcess prevents the shell from running traps.
    tempfile::Builder::new()
        .prefix(&format!("nyaterm-shell-{session_id}-"))
        .tempdir()
        .ok()
}

fn native_bootstrap(program: &str) -> Option<(String, String)> {
    if !cfg!(windows) {
        return Some(("/bin/sh".into(), program.into()));
    }
    // Use the same POSIX installation, never a different sh found on PATH.
    // In particular WindowsApps' legacy bash.exe launcher has no sibling sh.
    let path = Path::new(program);
    let executable = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
            let candidate = directory.join(program);
            let candidate = if candidate.extension().is_none() {
                candidate.with_extension("exe")
            } else {
                candidate
            };
            candidate.is_file().then_some(candidate)
        })?
    };
    let bootstrap: PathBuf = executable.parent()?.join("sh.exe");
    bootstrap.is_file().then(|| {
        (
            bootstrap.to_string_lossy().into_owned(),
            executable.to_string_lossy().replace('\\', "/"),
        )
    })
}

fn startup_script() -> String {
    include_str!("local_shell_integration/startup.sh").replace("\r\n", "\n")
}
