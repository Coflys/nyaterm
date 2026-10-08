//! Detection of `tmux -CC` startup inside a normal terminal byte stream.
//!
//! Detection is deliberately two-sided so a program that merely *prints*
//! tmux-looking lines can never hijack the channel:
//!
//! 1. **Arm** — the detector only watches output after the input side
//!    submits a command line that invokes tmux with `-C`/`-CC` (typed by
//!    the user, sent as a startup/post-login command, or wrapped in an AI
//!    capture exec). Arming expires after [`ARM_WINDOW`] or when the next
//!    non-matching command line is submitted.
//! 2. **Confirm** — once armed, a whitelisted `%`-notification at a line
//!    start hands the channel to the control session. The control loop
//!    additionally bails back to the shell if it sees plain-text lines
//!    where the protocol guarantees only `%` lines exist.

use super::control::is_control_notification;
use std::time::{Duration, Instant};

/// Upper bound for a held-back candidate line; longer lines flush as normal
/// output so a stray `%` can never stall the terminal.
const MAX_CANDIDATE_LINE_BYTES: usize = 512;

/// How long after a `tmux -CC` command line a control notification is still
/// expected. tmux announces control mode essentially immediately; a longer
/// window would only widen the false-positive surface for `%` lines printed
/// by unrelated programs.
const ARM_WINDOW: Duration = Duration::from_secs(30);

/// Upper bound for the tracked input line; longer lines cannot be matched
/// reliably anyway, so tracking resets instead of growing without bound.
const MAX_INPUT_LINE_BYTES: usize = 4096;

/// Commands whose following tokens are still "command position": prefix
/// wrappers (`sudo nice tmux -CC`) and environment assignments
/// (`FOO=1 tmux -CC` — handled via the `=` token rule).
const COMMAND_WRAPPERS: &[&str] = &[
    "sudo", "doas", "pkexec", "command", "builtin", "nice", "stdbuf", "noglob", "time", "exec",
    "nohup", "env",
];

/// `ssh` flags that consume the next token as their argument, so the
/// destination host is found while skipping them (`ssh -p 22 host tmux -CC`).
const SSH_FLAGS_WITH_ARG: &[&str] = &[
    "-B", "-b", "-c", "-D", "-E", "-F", "-I", "-i", "-J", "-L", "-l", "-m", "-O", "-o", "-p", "-Q",
    "-R", "-S", "-W", "-w",
];

fn strip_quotes(token: &str) -> &str {
    token.trim_matches(|c| c == '"' || c == '\'')
}

fn is_shell_separator(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| matches!(c, ';' | '|' | '&' | '<' | '>' | '(' | ')' | '{' | '}'))
}

/// Split `line` into words and separator tokens. Separator characters count
/// even when glued to a word (`foo; tmux -CC`, `} ; tmux`), but quotes keep
/// them inside their word so `echo 'x; tmux -CC'` cannot arm the detector.
fn shell_tokens(line: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let mut start = 0;
    let mut i = 0;
    let mut quote = 0u8;
    while i < bytes.len() {
        let byte = bytes[i];
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
        } else if byte == b'\'' || byte == b'"' {
            quote = byte;
        } else if byte.is_ascii_whitespace() {
            if start < i {
                tokens.push(&line[start..i]);
            }
            start = i + 1;
        } else if matches!(
            byte,
            b';' | b'|' | b'&' | b'<' | b'>' | b'(' | b')' | b'{' | b'}'
        ) {
            if start < i {
                tokens.push(&line[start..i]);
            }
            tokens.push(&line[i..i + 1]);
            start = i + 1;
        }
        i += 1;
    }
    if start < bytes.len() {
        tokens.push(&line[start..]);
    }
    tokens
}

fn is_tmux_binary(token: &str) -> bool {
    token == "tmux" || token.ends_with("/tmux")
}

fn is_control_flag(token: &str) -> bool {
    // `-C` enters control mode, `-CC` additionally silences echo.
    token.len() >= 2 && token.starts_with('-') && token[1..].chars().all(|c| c == 'C')
}

/// Whether `line` invokes tmux in control mode. Evaluated on the input side:
/// the command that spawned the protocol is visible before its output is.
///
/// Token-level analysis, not a full shell parser: it tracks command position
/// across `; | & >` separators and past prefix wrappers (`sudo`, `env`,
/// `VAR=val`), understands `ssh host tmux -CC`, and matches `tmux`/`*/tmux`
/// followed by a `-C`/`-CC` flag. Misses (aliases, `sh -c 'tmux -CC'`,
/// indirect launches) degrade to plain passthrough — never to a hijack.
pub(crate) fn is_tmux_control_command(line: &str) -> bool {
    let mut expect_command = true;
    let mut expect_ssh_dest = false;
    let mut ssh_flag_arg = false;
    let mut tmux_seen = false;

    for raw in shell_tokens(line) {
        let token = strip_quotes(raw);
        if token.is_empty() {
            continue;
        }
        if is_shell_separator(token) {
            expect_command = true;
            expect_ssh_dest = false;
            ssh_flag_arg = false;
            tmux_seen = false;
            continue;
        }
        if expect_ssh_dest {
            // Skip ssh options and their arguments; the first bare token is
            // the destination, and what follows it is the remote command.
            if ssh_flag_arg {
                ssh_flag_arg = false;
                continue;
            }
            if token.starts_with('-') {
                if SSH_FLAGS_WITH_ARG.contains(&token) {
                    ssh_flag_arg = true;
                }
                continue;
            }
            expect_ssh_dest = false;
            expect_command = true;
            continue;
        }
        if expect_command {
            if token.starts_with('-') {
                // Option of a pending wrapper (`sudo -k tmux -CC`); options
                // that take arguments may still swallow the command name —
                // a documented miss, never a false positive.
                continue;
            }
            if COMMAND_WRAPPERS.contains(&token) || token.contains('=') {
                continue;
            }
            if token == "ssh" {
                expect_ssh_dest = true;
                continue;
            }
            if is_tmux_binary(token) {
                tmux_seen = true;
            }
            expect_command = false;
            continue;
        }
        if tmux_seen && is_control_flag(token) {
            return true;
        }
    }
    false
}

pub(crate) enum TmuxFeed {
    /// Bytes to forward through the regular output pipeline.
    Passthrough(Vec<u8>),
    /// A control notification was found at a line start.
    ///
    /// `passthrough` are bytes before the notification that still belong to
    /// the normal terminal stream; `rest` are the bytes after the detected
    /// line and must be consumed by the control-mode parser.
    Detected { passthrough: Vec<u8>, rest: Vec<u8> },
}

#[derive(Default)]
pub(crate) struct TmuxControlDetector {
    /// The next byte begins a new line.
    line_start: bool,
    /// Bytes of a not-yet-decided line that started with `%`.
    candidate: Option<Vec<u8>>,
    /// Set once control mode was detected; a dead detector passes through.
    done: bool,
    /// Input line being composed on the user side; completed on Enter.
    input_line: Vec<u8>,
    /// Inside a CSI escape (`ESC [ ... <final>`) while tracking input.
    input_in_csi: bool,
    /// Set when a `tmux -CC` command line was just submitted; notifications
    /// only count while this is armed and fresh.
    armed_at: Option<Instant>,
}

impl TmuxControlDetector {
    pub(crate) fn new() -> Self {
        Self {
            line_start: true,
            candidate: None,
            done: false,
            input_line: Vec::new(),
            input_in_csi: false,
            armed_at: None,
        }
    }

    /// Pre-arm detection for a configured startup/post-login command that
    /// already matched `tmux -CC`.
    pub(crate) fn arm(&mut self) {
        self.armed_at = Some(Instant::now());
    }

    fn is_armed(&self) -> bool {
        self.armed_at
            .is_some_and(|armed_at| armed_at.elapsed() <= ARM_WINDOW)
    }

    /// Track bytes written to the channel (user input, AI capture commands)
    /// so a `tmux -CC` command line arms output detection.
    pub(crate) fn feed_input(&mut self, data: &[u8]) {
        if self.done {
            return;
        }
        for &byte in data {
            if self.input_in_csi {
                // CSI ends on a byte in 0x40..=0x7e.
                if (0x40..=0x7e).contains(&byte) {
                    self.input_in_csi = false;
                }
                continue;
            }
            match byte {
                b'\r' | b'\n' => {
                    let line = std::mem::take(&mut self.input_line);
                    self.evaluate_input_line(&line);
                }
                0x7f | 0x08 => {
                    self.input_line.pop();
                }
                0x1b => {
                    self.input_in_csi = true;
                }
                _ => {
                    if self.input_line.len() < MAX_INPUT_LINE_BYTES {
                        self.input_line.push(byte);
                    } else {
                        self.input_line.clear();
                    }
                }
            }
        }
    }

    fn evaluate_input_line(&mut self, line: &[u8]) {
        match std::str::from_utf8(line) {
            Ok(text) if is_tmux_control_command(text) => {
                self.armed_at = Some(Instant::now());
            }
            // A different submitted command resolves the `tmux -CC` attempt
            // either way — close the window rather than staying primed.
            _ if line.iter().any(|b| !b.is_ascii_whitespace()) => {
                self.armed_at = None;
            }
            _ => {}
        }
    }

    pub(crate) fn feed(&mut self, data: &[u8]) -> TmuxFeed {
        if self.done {
            return TmuxFeed::Passthrough(data.to_vec());
        }

        let mut out: Vec<u8> = Vec::with_capacity(data.len());
        let mut i = 0;

        while i < data.len() {
            if let Some(candidate) = self.candidate.as_mut() {
                let byte = data[i];
                i += 1;
                if byte == b'\n' {
                    let mut line = std::mem::take(candidate);
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    self.candidate = None;
                    match std::str::from_utf8(&line) {
                        Ok(text) if is_control_notification(text) && self.is_armed() => {
                            self.done = true;
                            return TmuxFeed::Detected {
                                passthrough: out,
                                rest: data[i..].to_vec(),
                            };
                        }
                        _ => {
                            // Not a notification: the line was ordinary output.
                            out.extend_from_slice(&line);
                            out.push(b'\n');
                            self.line_start = true;
                        }
                    }
                } else {
                    // Grammar: `%<keyword:[a-z-]+> [' ' <args>] '\n'.
                    // Once a space ends the keyword, args may be arbitrary.
                    let in_args = candidate[1..].contains(&b' ');
                    let allowed = if in_args {
                        true
                    } else {
                        byte.is_ascii_lowercase() || byte == b'-' || byte == b' '
                    };
                    if !allowed || candidate.len() >= MAX_CANDIDATE_LINE_BYTES {
                        // Can never become a notification: flush as output.
                        out.extend_from_slice(candidate);
                        out.push(byte);
                        self.candidate = None;
                        self.line_start = false;
                    } else {
                        candidate.push(byte);
                    }
                }
                continue;
            }

            let byte = data[i];
            i += 1;
            if self.line_start && byte == b'%' {
                self.candidate = Some(vec![b'%']);
                self.line_start = false;
            } else {
                out.push(byte);
                self.line_start = byte == b'\n';
            }
        }

        TmuxFeed::Passthrough(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{TmuxControlDetector, TmuxFeed, is_tmux_control_command};

    #[test]
    fn detects_control_line_and_splits_stream() {
        let mut detector = TmuxControlDetector::new();
        detector.feed_input(b"tmux -CC new -As0\r");
        let chunk =
            b"user@host:~$ tmux -CC new -As0\r\n%session-changed $0 0\r\n%window-add @1\r\n";
        match detector.feed(chunk) {
            TmuxFeed::Detected { passthrough, rest } => {
                assert_eq!(passthrough, b"user@host:~$ tmux -CC new -As0\r\n");
                assert_eq!(rest, b"%window-add @1\r\n");
            }
            TmuxFeed::Passthrough(_) => panic!("expected detection"),
        }
    }

    #[test]
    fn unarmed_output_never_triggers_takeover() {
        // A normal shell that prints tmux-looking lines must not be
        // hijacked — the review's `printf '%window-add @1\n'` case.
        let mut detector = TmuxControlDetector::new();
        match detector.feed(b"%window-add @1\n%session-changed $0 x\n") {
            TmuxFeed::Passthrough(bytes) => {
                assert_eq!(bytes, b"%window-add @1\n%session-changed $0 x\n");
            }
            TmuxFeed::Detected { .. } => panic!("unarmed detector must pass through"),
        }
    }

    #[test]
    fn non_tmux_command_disarms() {
        // `tmux -CC` failed (`command not found`); the next command closes
        // the arming window so later `%` output is harmless.
        let mut detector = TmuxControlDetector::new();
        detector.feed_input(b"tmux -CC\r");
        detector.feed_input(b"ls\r");
        match detector.feed(b"%window-add @1\n") {
            TmuxFeed::Passthrough(bytes) => assert_eq!(bytes, b"%window-add @1\n"),
            TmuxFeed::Detected { .. } => panic!("disarmed detector must pass through"),
        }
    }

    #[test]
    fn startup_command_can_pre_arm() {
        let mut detector = TmuxControlDetector::new();
        detector.arm();
        match detector.feed(b"%session-changed $0 0\nrest") {
            TmuxFeed::Detected { rest, .. } => assert_eq!(rest, b"rest"),
            TmuxFeed::Passthrough(_) => panic!("pre-armed detector must detect"),
        }
    }

    #[test]
    fn ai_wrapped_command_arms() {
        // Marker-wrapped capture commands still contain the tmux invocation
        // (`{ cmd; }` braces and glued `;` must not hide command position).
        let mut detector = TmuxControlDetector::new();
        detector.feed_input(
            b" printf '\\n__DF_CMD_''START_x__\\n'; { tmux -CC attach -t 0; }; _dfec=$?; printf '\\n__DF_CMD_''END_x__\\n'\r",
        );
        assert!(matches!(
            detector.feed(b"%session-changed $0 0\n"),
            TmuxFeed::Detected { .. }
        ));
    }

    #[test]
    fn quoted_separator_never_arms() {
        // `;` inside quotes stays inside the word — echoing a tmux-looking
        // string must not arm, or `echo 'x; tmux -CC'` would hijack.
        let mut detector = TmuxControlDetector::new();
        detector.feed_input(b"echo 'x; tmux -CC'\r");
        match detector.feed(b"%session-changed $0 0\n") {
            TmuxFeed::Passthrough(bytes) => {
                assert_eq!(bytes, b"%session-changed $0 0\n");
            }
            TmuxFeed::Detected { .. } => panic!("quoted tmux text must not arm"),
        }
    }

    #[test]
    fn ignores_percent_text_mid_line() {
        let mut detector = TmuxControlDetector::new();
        detector.feed_input(b"tmux -CC\r");
        match detector.feed(b"value is %window-add @1\n") {
            TmuxFeed::Passthrough(bytes) => {
                assert_eq!(bytes, b"value is %window-add @1\n");
            }
            TmuxFeed::Detected { .. } => panic!("must not detect mid-line"),
        }
    }

    #[test]
    fn ignores_unknown_percent_lines_at_start() {
        let mut detector = TmuxControlDetector::new();
        detector.feed_input(b"tmux -CC\r");
        match detector.feed(b"%percent-sign example\nnext") {
            TmuxFeed::Passthrough(bytes) => {
                assert_eq!(bytes, b"%percent-sign example\nnext");
            }
            TmuxFeed::Detected { .. } => panic!("must not detect unknown keyword"),
        }
    }

    #[test]
    fn candidate_line_flushed_when_keyword_breaks() {
        let mut detector = TmuxControlDetector::new();
        match detector.feed(b"\n%foo_bar more\n") {
            TmuxFeed::Passthrough(bytes) => assert_eq!(bytes, b"\n%foo_bar more\n"),
            TmuxFeed::Detected { .. } => panic!("unexpected detection"),
        }
    }

    #[test]
    fn detection_spans_chunk_boundaries() {
        let mut detector = TmuxControlDetector::new();
        detector.feed_input(b"tmux -CC attach\r");
        assert!(matches!(
            detector.feed(b"prompt%window-ad"),
            TmuxFeed::Passthrough(_)
        ));
        // `%window-ad` was not at a line start, second chunk continues normal.
        match detector.feed(b"\n%wind") {
            TmuxFeed::Passthrough(bytes) => assert_eq!(bytes, b"\n"),
            TmuxFeed::Detected { .. } => panic!("line not complete yet"),
        }
        match detector.feed(b"ow-add @1\n%exit\n") {
            TmuxFeed::Detected { rest, .. } => assert_eq!(rest, b"%exit\n"),
            TmuxFeed::Passthrough(_) => panic!("expected detection"),
        }
    }

    #[test]
    fn tmux_control_command_matching() {
        // Positive: direct, wrapped, chained, nested-ssh invocations.
        for line in [
            "tmux -CC",
            "tmux -C",
            "tmux -CC new -As0",
            "tmux -CC attach -t 0",
            "  tmux   -CC  ",
            "sudo tmux -CC",
            "sudo -k nice tmux -CC",
            "env FOO=1 tmux -CC",
            "FOO=1 tmux -C",
            "cd ~ && tmux -CC attach",
            "ssh host tmux -CC",
            "ssh -p 2222 user@host tmux -CC",
            "/usr/bin/tmux -CC",
            "tmux \t-CC",
        ] {
            assert!(is_tmux_control_command(line), "should match: {line}");
        }
        // Negative: tmux as an argument, missing flag, lookalike names.
        for line in [
            "echo tmux -CC",
            "printf 'tmux -CC\\n'",
            "cat tmux -CC",
            "tmux new",
            "tmux attach -t 0",
            "tmux -CCx",
            "tmuxer -CC",
            "mytmux -CC",
            "ssh host",
            "",
            "ls -la",
        ] {
            assert!(!is_tmux_control_command(line), "should not match: {line}");
        }
    }
}
