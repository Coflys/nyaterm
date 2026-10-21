import type { Terminal } from "@xterm/xterm";
import { stripTerminalCommandPrompt } from "@/lib/terminalCommand";
import {
  applyTerminalInputData,
  resyncFromTerminalLine,
  type TerminalInputState,
} from "@/lib/terminalInputTracker";
import { readCurrentInputLine } from "./terminalInputSelection";

/** Observe history echo separately from keystrokes; never recover from an eager screen read. */
export function createTerminalHistoryRecovery(terminal: Terminal, hasPendingOutput: () => boolean) {
  let pending: { state: TerminalInputState; before: string; emptyDown: boolean } | null = null;

  return {
    applyInput(state: TerminalInputState, data: string): TerminalInputState {
      const next = applyTerminalInputData(state, data);
      pending = null;
      if (
        next.desyncReason === "history" &&
        ["\x1b[A", "\x1bOA", "\x1b[B", "\x1bOB"].includes(data) &&
        !state.desynced &&
        !state.multiline &&
        !state.pasteMode &&
        terminal.buffer.active.type === "normal" &&
        !hasPendingOutput()
      ) {
        const before = readCurrentInputLine(terminal);
        const command = stripTerminalCommandPrompt(before);
        pending = {
          state: next,
          before,
          emptyDown:
            (data === "\x1b[B" || data === "\x1bOB") &&
            command === "" &&
            command !== before.trimStart(),
        };
      }
      return next;
    },

    onWriteParsed(state: TerminalInputState): TerminalInputState | null {
      if (!pending || pending.state !== state) {
        pending = null;
        return null;
      }
      if (terminal.buffer.active.type !== "normal" || hasPendingOutput()) return null;
      const line = readCurrentInputLine(terminal);
      // Ignore unrelated output that leaves the old command visible. A Down at
      // an empty prompt can legitimately produce only a bell or a prompt redraw.
      if (line === pending.before && !pending.emptyDown) return null;
      const recovered = resyncFromTerminalLine(state, line, true);
      if (recovered) pending = null;
      return recovered;
    },
  };
}
