# Terminal IME caret fix — 2026-10-29

Chinese IME preedit previously retained only its text. The terminal discarded
the IME selection, reported `0..0` to native text services, anchored every range
to the remote cursor, and painted no composition caret. The remote block cursor
therefore remained at the beginning of uncommitted text.

The terminal now owns one session-bound runtime composition state with UTF-16
selection and replacement semantics. A single shaped-line layout drives preedit
painting, caret bounds, native range queries, and hit testing. Preedit retains
its background and underline, uses a steady thin caret, and stays inside the
text grid with a 360px maximum width and horizontal clipping around the caret.
The layout uses terminal hit-test geometry, including gutters and cleared rows,
and leaves physical-pixel conversion to GPUI.

Composition changes explicitly refresh the independent terminal surface's
cursor policy. Commit, cancellation, blur, session activation, and session
removal clear composition and restore the normal cursor. Preedit is never sent
to the transport; committed text continues through the existing terminal edit
and encoding path. No persistence format or public API changes.

## Fork revisions

Both dependency changes were committed and pushed to their `nyaterm` branches,
with patch and validation records in each fork's `NYATERM.md`.

| Fork | Previous revision | New revision | Change |
| --- | --- | --- | --- |
| [nyakang/zed](https://github.com/nyakang/zed) | `c054331114f9abdc44e5d587cd23314db6ac5aff` | `8e5a53c0aa6ff991bdf2bcbb4e6049e2f4715658` | Forward cursor-only Windows IME composition notifications once, preserving commit ordering and the existing compatibility policy. |
| [nyakang/gpui-kit](https://github.com/nyakang/gpui-kit) | `5aca244fdcfbeeb3243e41cdbfa954e8be4a39f5` | `c0ca15118df36f96675b9ce355daa5ca2444f285` | Align all six Zed dependency pins and their lockfile sources to avoid two incompatible GPUI type sets. |

## Automated validation on Windows

- GPUI Windows: 4 composition dispatch regression tests passed.
- GPUI Kit: pin consistency script and locked offline Cargo metadata passed.
- NyaTerm: `cargo check -p nyaterm-app` passed; dependency inspection found one GPUI revision.
- NyaTerm: `cargo check --workspace` passed.
- NyaTerm: `cargo fmt --all -- --check` passed.
- `cargo test --workspace -j 1` passed, including all 2,335 non-ignored desktop
  tests. The relevant coverage includes 3 IME state tests, 2 edge-clipping
  geometry tests, 3 GPUI integration tests, and the existing commit-once
  transport regression. The integration tests cover shared caret geometry,
  cursor restoration, lifecycle cleanup, and independent surface invalidation
  for text-only and selection-only updates.
- `cargo clippy --workspace --all-targets -j 1` passed without warnings.

The initial default-parallel workspace test build exceeded Windows virtual
memory (`os error 1455`) while mapping/linking large test artifacts. Retrying
with one Cargo build job completed successfully; no system settings changed.

## Interactive validation still required

Real Windows input-method interaction was not performed. Headless state and
painting tests do not establish candidate-window behavior in a native desktop
session. The computer-use skill prohibits terminal application automation.

Manually check both a local shell and SSH with a Chinese input method:

1. Type a long pinyin sequence and move left/right within it; the thin caret and
   candidate anchor should follow the actual insertion point without a block
   cursor at the composition start.
2. Delete text, choose a candidate, cancel with Escape, and switch input modes;
   committed text should arrive once and the original terminal cursor should
   return when composition ends.
3. Repeat at the right edge, with line numbers/timestamps enabled and with
   different terminal fonts and display scaling; keep the caret visible and
   the composition outside the gutter.
4. Blur the terminal, switch sessions, and close a composing session; no stale
   preedit should appear in the next session.
