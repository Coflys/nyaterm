use crate::keywords::{CompiledKeywordRule, keyword_highlight_spans_compiled};
use crate::types::TerminalHighlightSpan;
use crate::{resolve_cell_bg, resolve_cell_fg};
use nyaterm_terminal::{
    TERMINAL_TAB_STOP_WIDTH, terminal_char_cell_width, terminal_is_zero_width_mark,
};

/// The grid materializes a tab as blank cells up to the next tab stop, but the
/// ANSI fast path only sees the raw escape text. Without expanding tabs here,
/// background ranges computed from ANSI spans land one column short per tab and
/// disagree with the grid-anchored cell path — the first paint then differs from
/// every repaint.
fn expand_ansi_tabs(ansi: &[nyaterm_terminal::StyledSpan]) -> Vec<nyaterm_terminal::StyledSpan> {
    if !ansi.iter().any(|span| span.text.contains('\t')) {
        return ansi.to_vec();
    }
    let mut out = Vec::with_capacity(ansi.len());
    let mut col = 0usize;
    for span in ansi {
        if !span.text.contains('\t') {
            col += span
                .text
                .chars()
                .filter(|ch| !terminal_is_zero_width_mark(*ch))
                .map(terminal_char_cell_width)
                .sum::<usize>();
            out.push(span.clone());
            continue;
        }
        let mut text = String::with_capacity(span.text.len());
        for ch in span.text.chars() {
            if ch == '\t' {
                let next = (col / TERMINAL_TAB_STOP_WIDTH + 1) * TERMINAL_TAB_STOP_WIDTH;
                text.extend(std::iter::repeat_n(' ', next - col));
                col = next;
            } else {
                text.push(ch);
                if !terminal_is_zero_width_mark(ch) {
                    col += terminal_char_cell_width(ch);
                }
            }
        }
        out.push(nyaterm_terminal::StyledSpan {
            text,
            style: span.style,
        });
    }
    out
}

pub(super) fn ansi_to_highlight_spans_compiled(
    ansi: &[nyaterm_terminal::StyledSpan],
    palette: nyaterm_ui::ThemePalette,
    compiled_keyword_rules: &[CompiledKeywordRule],
    bold_default_foreground: bool,
) -> Vec<TerminalHighlightSpan> {
    let ansi = &expand_ansi_tabs(ansi);
    // Build plain line for keyword overlay, then prefer keyword fg over default ANSI fg.
    let line: String = ansi.iter().map(|s| s.text.as_str()).collect();
    let keyword = keyword_highlight_spans_compiled(&line, compiled_keyword_rules);
    if keyword.iter().all(|s| !s.keyword) {
        return ansi
            .iter()
            .filter(|s| !s.text.is_empty())
            .map(|s| TerminalHighlightSpan {
                text: s.text.clone(),
                color: Some(resolve_cell_fg(palette, s.style, bold_default_foreground)),
                bg: resolve_cell_bg(palette, s.style),
                keyword: false,
                underline: s.style.underline,
                strikeout: s.style.strikeout,
                bold: s.style.bold,
                italic: s.style.italic,
            })
            .collect();
    }

    // Flatten keyword color map by byte offset, then re-slice per ANSI span.
    let mut keyword_color_at = vec![None; line.len()];
    let mut offset = 0usize;
    for span in &keyword {
        let end = offset + span.text.len();
        if span.keyword {
            for idx in offset..end.min(keyword_color_at.len()) {
                keyword_color_at[idx] = span.color;
            }
        }
        offset = end;
    }

    let mut out = Vec::new();
    let mut cursor = 0usize;
    for s in ansi {
        if s.text.is_empty() {
            continue;
        }
        let start = cursor;
        let end = cursor + s.text.len();
        cursor = end;
        let bg = resolve_cell_bg(palette, s.style);
        let color = resolve_cell_fg(palette, s.style, bold_default_foreground);
        if s.style.hidden {
            push_ansi_segment(
                &mut out,
                &s.text,
                bg.unwrap_or(palette.terminal_bg),
                None,
                bg,
                s.style,
            );
            continue;
        }
        if s.style.fg.is_some() || s.style.fg_rgb.is_some() {
            push_ansi_segment(&mut out, &s.text, color, None, bg, s.style);
            continue;
        }

        let mut segment_start = 0usize;
        let mut segment_keyword_color = keyword_color_at.get(start).copied().flatten();
        for (offset, _) in s.text.char_indices().skip(1) {
            let keyword_color = keyword_color_at.get(start + offset).copied().flatten();
            if keyword_color == segment_keyword_color {
                continue;
            }
            push_ansi_segment(
                &mut out,
                &s.text[segment_start..offset],
                color,
                segment_keyword_color,
                bg,
                s.style,
            );
            segment_start = offset;
            segment_keyword_color = keyword_color;
        }
        push_ansi_segment(
            &mut out,
            &s.text[segment_start..],
            color,
            segment_keyword_color,
            bg,
            s.style,
        );
    }
    if out.is_empty() {
        out.push(TerminalHighlightSpan {
            text: " ".to_string(),
            color: None,
            bg: None,
            keyword: false,
            underline: false,
            strikeout: false,
            bold: false,
            italic: false,
        });
    }
    out
}

fn push_ansi_segment(
    out: &mut Vec<TerminalHighlightSpan>,
    text: &str,
    default_color: u32,
    keyword_color: Option<u32>,
    bg: Option<u32>,
    style: nyaterm_terminal::CellStyle,
) {
    if text.is_empty() {
        return;
    }
    out.push(TerminalHighlightSpan {
        text: text.to_string(),
        color: Some(keyword_color.unwrap_or(default_color)),
        bg,
        keyword: keyword_color.is_some(),
        underline: style.underline,
        strikeout: style.strikeout,
        bold: style.bold,
        italic: style.italic,
    });
}

#[cfg(test)]
mod tests {
    use super::ansi_to_highlight_spans_compiled;
    use crate::keywords::CompiledKeywordRule;

    fn spans(text: &str, style: nyaterm_terminal::CellStyle) -> Vec<nyaterm_terminal::StyledSpan> {
        vec![nyaterm_terminal::StyledSpan {
            text: text.to_string(),
            style,
        }]
    }

    fn compiled(pattern: &str, color: u32) -> Vec<CompiledKeywordRule> {
        vec![CompiledKeywordRule {
            regex: regex::Regex::new(pattern).unwrap(),
            color,
            priority: 0,
        }]
    }

    #[test]
    fn keyword_overlay_splits_default_ansi_span_at_match_boundaries() {
        let palette = nyaterm_ui::theme_palette("github-dark");
        let keyword_color = 0xff2244;
        let compiled = compiled("ERROR", keyword_color);

        let highlighted = ansi_to_highlight_spans_compiled(
            &spans(
                "prefix ERROR suffix",
                nyaterm_terminal::CellStyle::default(),
            ),
            palette,
            &compiled,
            false,
        );

        assert_eq!(highlighted.len(), 3);
        assert_eq!(highlighted[0].text, "prefix ");
        assert!(!highlighted[0].keyword);
        assert_eq!(highlighted[1].text, "ERROR");
        assert_eq!(highlighted[1].color, Some(keyword_color));
        assert!(highlighted[1].keyword);
        assert_eq!(highlighted[2].text, " suffix");
        assert!(!highlighted[2].keyword);
    }

    #[test]
    fn keyword_overlay_respects_explicit_truecolor_foreground() {
        let palette = nyaterm_ui::theme_palette("github-dark");
        let style = nyaterm_terminal::CellStyle {
            fg_rgb: Some(0x112233),
            ..nyaterm_terminal::CellStyle::default()
        };
        let compiled = compiled("ERROR", 0xff2244);

        let highlighted =
            ansi_to_highlight_spans_compiled(&spans("ERROR", style), palette, &compiled, false);

        assert_eq!(highlighted.len(), 1);
        assert_eq!(highlighted[0].color, Some(0x112233));
        assert!(!highlighted[0].keyword);
    }

    #[test]
    fn hidden_ansi_text_stays_concealed_from_keyword_overlay() {
        let palette = nyaterm_ui::theme_palette("github-dark");
        let style = nyaterm_terminal::CellStyle {
            bg_rgb: Some(0x112233),
            hidden: true,
            ..nyaterm_terminal::CellStyle::default()
        };
        let compiled = compiled("secret", 0xff2244);

        let highlighted =
            ansi_to_highlight_spans_compiled(&spans("secret", style), palette, &compiled, false);

        assert_eq!(highlighted.len(), 1);
        assert_eq!(highlighted[0].color, Some(0x112233));
        assert_eq!(highlighted[0].bg, Some(0x112233));
        assert!(!highlighted[0].keyword);
    }

    #[test]
    fn keyword_overlay_handles_multibyte_prefixes() {
        let palette = nyaterm_ui::theme_palette("github-dark");
        let keyword_color = 0xff2244;
        let compiled = compiled("ERROR", keyword_color);

        let highlighted = ansi_to_highlight_spans_compiled(
            &spans("界 ERROR", nyaterm_terminal::CellStyle::default()),
            palette,
            &compiled,
            false,
        );

        assert_eq!(highlighted.len(), 2);
        assert_eq!(highlighted[0].text, "界 ");
        assert_eq!(highlighted[1].text, "ERROR");
        assert_eq!(highlighted[1].color, Some(keyword_color));
    }

    #[test]
    fn tab_expands_to_stop_columns_like_the_grid() {
        // The grid materializes a tab as blank cells up to the next stop and
        // keeps the literal tab in the starting cell. The ANSI fast path only
        // sees raw escape text, so it must expand the tab itself — otherwise
        // background ranges drift one column per tab and disagree with the
        // grid-anchored cell path.
        let palette = nyaterm_ui::theme_palette("github-dark");
        let ansi = vec![
            nyaterm_terminal::StyledSpan {
                text: "System load:  \t".to_string(),
                style: nyaterm_terminal::CellStyle::default(),
            },
            nyaterm_terminal::StyledSpan {
                text: "0.33".to_string(),
                style: nyaterm_terminal::CellStyle {
                    fg: Some(3),
                    bg: Some(0),
                    ..nyaterm_terminal::CellStyle::default()
                },
            },
        ];

        let highlighted = ansi_to_highlight_spans_compiled(&ansi, palette, &[], false);

        assert!(!highlighted.iter().any(|span| span.text.contains('\t')));
        // Tab at column 14 jumps to the stop at 16: both columns must be
        // materialized so the colored span starts exactly at the stop.
        assert_eq!(highlighted[0].text, "System load:    ");
        assert_eq!(highlighted[1].text, "0.33");
        assert!(highlighted[1].bg.is_some());
    }

    #[test]
    fn tab_expansion_tracks_wide_characters() {
        let palette = nyaterm_ui::theme_palette("github-dark");
        let ansi = vec![nyaterm_terminal::StyledSpan {
            // `界` occupies two columns, so the tab starts at column 3 and
            // must stop at column 8.
            text: "a界\tX".to_string(),
            style: nyaterm_terminal::CellStyle::default(),
        }];

        let highlighted = ansi_to_highlight_spans_compiled(&ansi, palette, &[], false);

        assert_eq!(highlighted.len(), 1);
        assert_eq!(highlighted[0].text, format!("a界{}X", " ".repeat(5)));
    }
}
