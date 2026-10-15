import type { Terminal } from "@xterm/xterm";

const CTRL_L_INPUT = "\x0c";
const terminalUserInputMarkers = new WeakMap<Terminal, () => void>();

export function registerTerminalUserInputMarker(
  terminal: Terminal,
  marker: () => void,
): () => void {
  terminalUserInputMarkers.set(terminal, marker);
  return () => {
    terminalUserInputMarkers.delete(terminal);
  };
}

export function markTerminalUserInput(terminal: Terminal): void {
  terminalUserInputMarkers.get(terminal)?.();
}

export function sendTerminalClearInput(
  terminal: Terminal,
  options: { focus?: boolean } = {},
) {
  terminal.clearSelection();
  markTerminalUserInput(terminal);
  terminal.input(CTRL_L_INPUT, true);
  if (options.focus) {
    terminal.focus();
  }
}

export function clearTerminalScreen(
  terminal: Terminal,
  options: { focus?: boolean } = {},
): void {
  terminal.clearSelection();
  // Erase the visible screen without clearing scrollback or sending device input.
  terminal.write("\x1b[2J");
  if (options.focus) {
    terminal.focus();
  }
}

export function clearTerminalAll(
  terminal: Terminal,
  options: { shellRedraw?: boolean } = {},
): void {
  const { cursorX, cursorY } = terminal.buffer.active;
  terminal.clearSelection();
  terminal.clear();
  if (options.shellRedraw) {
    sendTerminalClearInput(terminal);
  } else if (cursorY > 0) {
    // Keep ConPTY's viewport cursor coordinates while discarding old output.
    terminal.write(`\x1b[1;1H\x1b[${cursorY}L\x1b[${cursorY + 1};${cursorX + 1}H`);
  }
}
