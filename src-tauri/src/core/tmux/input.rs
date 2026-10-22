//! Terminal input classification for `send-keys` injection.
//!
//! In control mode the channel only accepts commands, so pane input is
//! injected through `send-keys`. Writing the *local* terminal's sequences
//! verbatim (`send-keys -H`) leaves the pane to decode them with its own
//! TERM — e.g. xterm's End is `ESC [ F` while `screen`/`tmux` define it as
//! `ESC [ 4 ~`, so vim printed a stray `F` (`H` for Home). A regular tmux
//! client translates keys between the two terminfo tables on its input
//! side; naming the key (`send-keys End`) makes tmux do the same job,
//! including application-cursor and modified xterm forms.

/// One span of classified terminal input.
#[derive(Debug, PartialEq)]
pub(crate) enum InputSegment {
    /// Bytes delivered verbatim via `send-keys -H`.
    Literal(Vec<u8>),
    /// A `send-keys` key name such as `End` or `C-Left`.
    Key(String),
}

/// Split input into literal runs and named keys, preserving order.
///
/// An escape sequence left incomplete at the end of `data` degrades to
/// literal bytes — a sequence split across flushes reaches the pane as the
/// same byte stream a plain `send-keys -H` would have produced, so no extra
/// buffering is needed.
///
/// Between bracketed-paste markers (`ESC[200~`/`ESC[201~`) nothing is a key:
/// payloads copied out of a terminal can themselves contain raw escape
/// sequences, and they must reach the pane as literal text.
pub(crate) fn split_input(data: &[u8]) -> Vec<InputSegment> {
    let mut segments = Vec::new();
    let mut literal_start = 0;
    let mut i = 0;
    let mut in_paste = false;
    while i < data.len() {
        if data[i] == 0x1b {
            if data[i..].starts_with(b"\x1b[200~") {
                in_paste = true;
            } else if data[i..].starts_with(b"\x1b[201~") {
                in_paste = false;
            } else if !in_paste && let Some((name, len)) = key_at(&data[i..]) {
                if literal_start < i {
                    segments.push(InputSegment::Literal(data[literal_start..i].to_vec()));
                }
                segments.push(InputSegment::Key(name));
                i += len;
                literal_start = i;
                continue;
            }
        }
        i += 1;
    }
    if literal_start < data.len() {
        segments.push(InputSegment::Literal(data[literal_start..].to_vec()));
    }
    segments
}

/// If `data` (starting with `ESC`) opens with a complete special-key
/// sequence, return its `send-keys` name and byte length.
fn key_at(data: &[u8]) -> Option<(String, usize)> {
    match *data.get(1)? {
        b'O' => {
            // SS3: F1-F4 and application-mode cursor keys.
            let name = match *data.get(2)? {
                b'A' => "Up",
                b'B' => "Down",
                b'C' => "Right",
                b'D' => "Left",
                b'H' => "Home",
                b'F' => "End",
                b'P' => "F1",
                b'Q' => "F2",
                b'R' => "F3",
                b'S' => "F4",
                _ => return None,
            };
            Some((name.to_string(), 3))
        }
        b'[' => {
            // CSI: parameter/intermediate bytes are 0x20..=0x3f, the first
            // byte in 0x40..=0x7e terminates the sequence.
            let end = data
                .iter()
                .skip(2)
                .position(|b| !(0x20..=0x3f).contains(b))
                .map(|i| i + 2)?;
            let final_byte = data[end];
            if !(0x40..=0x7e).contains(&final_byte) {
                return None;
            }
            let params = &data[2..end];
            // Only plain `N` or `N;M` numeric forms are keys; private
            // markers stay literal (bracketed paste `200`/`201`, SGR mouse
            // `<`, kitty `u` finals are already excluded by their final).
            if !params.iter().all(|b| b.is_ascii_digit() || *b == b';') {
                return None;
            }
            let mut fields = params.split(|b| *b == b';').map(|f| {
                std::str::from_utf8(f)
                    .ok()
                    .and_then(|s| s.parse::<u32>().ok())
            });
            let first = fields.next().flatten();
            let second = fields.next().flatten();
            if fields.next().is_some() {
                return None;
            }
            let name = match final_byte {
                b'Z' if params.is_empty() => Some("BTab".to_string()),
                letter if letter.is_ascii_alphabetic() && first.unwrap_or(1) == 1 => {
                    csi_letter_key(letter)
                        .map(|base| format!("{}{}", modifier_prefix(second), base))
                }
                b'~' => {
                    csi_tilde_key(first?).map(|base| format!("{}{}", modifier_prefix(second), base))
                }
                _ => None,
            }?;
            Some((name, end + 1))
        }
        _ => None,
    }
}

/// `C-M-S-` modifier prefixes for xterm modifier parameters (`2` = Shift,
/// `3` = Meta, `5` = Ctrl; bitfield `param - 1`).
fn modifier_prefix(param: Option<u32>) -> &'static str {
    match param {
        Some(2) => "S-",
        Some(3) => "M-",
        Some(4) => "M-S-",
        Some(5) => "C-",
        Some(6) => "C-S-",
        Some(7) => "C-M-",
        Some(8) => "C-M-S-",
        _ => "",
    }
}

/// `send-keys` name for `ESC [ <first> <letter>` sequences (xterm cursor
/// and SS3-over-CSI function keys; modifiers ride in the second parameter).
fn csi_letter_key(final_byte: u8) -> Option<&'static str> {
    match final_byte {
        b'A' => Some("Up"),
        b'B' => Some("Down"),
        b'C' => Some("Right"),
        b'D' => Some("Left"),
        b'H' => Some("Home"),
        b'F' => Some("End"),
        b'P' => Some("F1"),
        b'Q' => Some("F2"),
        b'R' => Some("F3"),
        b'S' => Some("F4"),
        _ => None,
    }
}

/// `send-keys` name for `ESC [ <number> ~` sequences, or `None` for non-key
/// payloads (bracketed paste markers `200`/`201`, ...).
fn csi_tilde_key(number: u32) -> Option<&'static str> {
    match number {
        1 | 7 => Some("Home"),
        2 => Some("IC"),
        3 => Some("DC"),
        4 | 8 => Some("End"),
        5 => Some("PPage"),
        6 => Some("NPage"),
        11 => Some("F1"),
        12 => Some("F2"),
        13 => Some("F3"),
        14 => Some("F4"),
        15 => Some("F5"),
        17 => Some("F6"),
        18 => Some("F7"),
        19 => Some("F8"),
        20 => Some("F9"),
        21 => Some("F10"),
        23 => Some("F11"),
        24 => Some("F12"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{InputSegment, split_input};

    fn lit(text: &str) -> InputSegment {
        InputSegment::Literal(text.as_bytes().to_vec())
    }

    fn key(name: &str) -> InputSegment {
        InputSegment::Key(name.to_string())
    }

    #[test]
    fn plain_text_stays_literal() {
        assert_eq!(split_input(b"ls -la\r"), vec![lit("ls -la\r")]);
    }

    #[test]
    fn home_and_end_map_to_names() {
        assert_eq!(split_input(b"\x1b[F"), vec![key("End")]);
        assert_eq!(split_input(b"\x1b[H"), vec![key("Home")]);
        assert_eq!(split_input(b"\x1bOF"), vec![key("End")]);
        assert_eq!(split_input(b"\x1bOH"), vec![key("Home")]);
        assert_eq!(split_input(b"\x1b[1~"), vec![key("Home")]);
        assert_eq!(split_input(b"\x1b[4~"), vec![key("End")]);
        assert_eq!(split_input(b"\x1b[7~"), vec![key("Home")]);
        assert_eq!(split_input(b"\x1b[8~"), vec![key("End")]);
    }

    #[test]
    fn arrows_and_function_keys_map() {
        assert_eq!(split_input(b"\x1b[A"), vec![key("Up")]);
        assert_eq!(split_input(b"\x1bOD"), vec![key("Left")]);
        assert_eq!(split_input(b"\x1b[1;5C"), vec![key("C-Right")]);
        assert_eq!(split_input(b"\x1b[1;3D"), vec![key("M-Left")]);
        assert_eq!(split_input(b"\x1b[1;8B"), vec![key("C-M-S-Down")]);
        assert_eq!(split_input(b"\x1bOP"), vec![key("F1")]);
        assert_eq!(split_input(b"\x1b[1;2Q"), vec![key("S-F2")]);
        assert_eq!(split_input(b"\x1b[15~"), vec![key("F5")]);
        assert_eq!(split_input(b"\x1b[24~"), vec![key("F12")]);
        assert_eq!(split_input(b"\x1b[Z"), vec![key("BTab")]);
    }

    #[test]
    fn editing_keys_map() {
        assert_eq!(split_input(b"\x1b[2~"), vec![key("IC")]);
        assert_eq!(split_input(b"\x1b[3~"), vec![key("DC")]);
        assert_eq!(split_input(b"\x1b[3;5~"), vec![key("C-DC")]);
        assert_eq!(split_input(b"\x1b[5~"), vec![key("PPage")]);
        assert_eq!(split_input(b"\x1b[6~"), vec![key("NPage")]);
    }

    #[test]
    fn non_key_sequences_stay_literal() {
        // Bracketed paste markers, focus reports, SGR mouse input, kitty
        // CSI-u keys, Alt+char — none of these are tmux key names.
        for data in [
            "\x1b[200~pasted\x1b[201~",
            "\x1b[I",
            "\x1b[O",
            "\x1b[<64;12;30M",
            "\x1b[97;5u",
            "\x1bb",
            "\x1b",
            "\x1b[1;",
        ] {
            assert_eq!(
                split_input(data.as_bytes()),
                vec![lit(data)],
                "{data:?} must pass through verbatim"
            );
        }
    }

    #[test]
    fn bracketed_paste_payload_stays_literal() {
        // Payload copied from a terminal may itself contain key-shaped
        // sequences; inside the markers none of them classify as keys.
        let data = b"\x1b[200~end \x1b[F home \x1b[H\x1b[201~";
        assert_eq!(
            split_input(data),
            vec![lit("\x1b[200~end \x1b[F home \x1b[H\x1b[201~")]
        );
        // Keys after the closing marker classify again.
        assert_eq!(
            split_input(b"\x1b[200~x\x1b[201~\x1b[F"),
            vec![lit("\x1b[200~x\x1b[201~"), key("End")]
        );
        // An unclosed marker keeps the remainder literal, matching the
        // pre-patch `send-keys -H` behavior for truncated input.
        assert_eq!(
            split_input(b"\x1b[200~x\x1b[F"),
            vec![lit("\x1b[200~x\x1b[F")]
        );
    }

    #[test]
    fn order_is_preserved() {
        assert_eq!(
            split_input(b"a\x1b[Fb"),
            vec![lit("a"), key("End"), lit("b")]
        );
        assert_eq!(
            split_input(b"\x1b[Ax\x1bOA"),
            vec![key("Up"), lit("x"), key("Up")]
        );
    }
}
