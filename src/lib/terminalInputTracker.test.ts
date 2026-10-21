import { describe, expect, it } from "vitest";
import {
  applyTerminalInputData,
  applyTerminalInputPreview,
  canRegisterCommandFromTracker,
  canSuggestFromTracker,
  createTerminalInputState,
  getTrackedCommand,
  getTrackedSubmissionCommand,
  resyncFromTerminalLine,
} from "./terminalInputTracker";

function typeInput(input: string) {
  return [...input].reduce(applyTerminalInputData, createTerminalInputState());
}

describe("terminal command tracker recovery", () => {
  it.each([
    ["Down", ["\x1b[B"]],
    ["Up then Down", ["\x1b[A", "\x1b[B"]],
    ["application-mode Down", ["\x1bOB"]],
  ])("recovers a fresh prompt after %s history navigation", (_label, navigation) => {
    let state = typeInput("stale");
    for (const arrow of navigation) {
      state = applyTerminalInputData(state, arrow);
    }
    expect(state.desynced).toBe(true);
    expect(state.desyncReason).toBe("history");
    expect(canSuggestFromTracker(state)).toBe(false);

    expect(resyncFromTerminalLine(state, "$ ")).toBeNull();
    const recovered = resyncFromTerminalLine(state, "$ ", true);
    expect(recovered).not.toBeNull();
    state = applyTerminalInputData(recovered!, "s");
    expect(state.value).toBe("s");
    expect(state.cursor).toBe(1);
    expect(getTrackedCommand(state)).toBe("s");
    expect(canSuggestFromTracker(state)).toBe(true);
    expect(getTrackedSubmissionCommand(state)).toBe("s");
  });

  it("takes a completely different history command instead of appending to the old one", () => {
    const historyState = applyTerminalInputData(typeInput("old command"), "\x1b[A");
    expect(resyncFromTerminalLine(historyState, "user@host:~$ git status")).toBeNull();
    const recovered = resyncFromTerminalLine(historyState, "user@host:~$ git status", true);
    expect(recovered?.value).toBe("git status");
    expect(recovered?.cursor).toBe("git status".length);
    expect(recovered?.lineRewriteRequired).toBe(false);
    const next = applyTerminalInputData(recovered!, "s");
    expect(getTrackedCommand(next)).toBe("git statuss");
  });

  it("does not trust an unrecognized history prompt or cursor edits", () => {
    const historyState = applyTerminalInputData(typeInput("old"), "\x1b[A");
    expect(resyncFromTerminalLine(historyState, "custom>> other command", true)).toBeNull();
    const unresolved = applyTerminalInputData(historyState, "s");
    expect(unresolved.desynced).toBe(true);
    expect(canSuggestFromTracker(unresolved)).toBe(false);
    expect(getTrackedSubmissionCommand(unresolved)).toBe("");

    const movedCursor = applyTerminalInputData(historyState, "\x1b[D");
    expect(movedCursor.desyncReason).toBe("terminal");
    expect(resyncFromTerminalLine(movedCursor, "$ new command")).toBeNull();
    expect(canSuggestFromTracker(applyTerminalInputData(movedCursor, "s"))).toBe(false);
  });

  it("does not erase a normally typed command from a stale prompt-only buffer", () => {
    const typed = typeInput("ls -la");
    expect(resyncFromTerminalLine(typed, "$ ")).toBeNull();
    expect(getTrackedSubmissionCommand(typed)).toBe("ls -la");

    const unknownControl = applyTerminalInputData(typed, "\x1b[1;5A");
    expect(resyncFromTerminalLine(unknownControl, "$ ls -la")).toBeNull();
    expect(getTrackedSubmissionCommand(unknownControl)).toBe("");
  });

  it("resynchronizes Tab completion without treating a bare $ prompt as a command", () => {
    const tabState = applyTerminalInputData(typeInput("stale"), "\t");
    expect(tabState.desyncReason).toBe("tab");
    const recovered = resyncFromTerminalLine(tabState, "$ ");
    expect(recovered).not.toBeNull();
    const next = applyTerminalInputData(recovered!, "s");
    expect(getTrackedCommand(next)).toBe("s");
    expect(canSuggestFromTracker(next)).toBe(true);
  });

  it("preserves completed text and its spaces after Tab and Shift-Tab", () => {
    for (const completionKey of ["\t", "\x1b[Z"]) {
      const desynced = applyTerminalInputData(typeInput("s"), completionKey);
      const recovered = resyncFromTerminalLine(desynced, "$ s  s   s");
      expect(recovered?.value).toBe("s  s   s");
      expect(getTrackedCommand(recovered!)).toBe("s  s   s");
      expect(canSuggestFromTracker(recovered!)).toBe(true);
    }
  });

  it("keeps an unverified nonempty Tab completion suppressed", () => {
    const desynced = applyTerminalInputData(typeInput("old"), "\t");
    expect(resyncFromTerminalLine(desynced, "unknown> changed")).toBeNull();
    const next = applyTerminalInputData(desynced, "s");
    expect(next.desynced).toBe(true);
    expect(canSuggestFromTracker(next)).toBe(false);
  });

  it("does not mistake unknown escapes for recoverable history", () => {
    let state = applyTerminalInputData(typeInput("ls"), "\x1b[1;5A");
    expect(state.desyncReason).toBe("terminal");
    state = applyTerminalInputData(state, "s");
    expect(canSuggestFromTracker(state)).toBe(false);
  });

  it("preserves Enter, Ctrl+C, paste and explicit command replacement", () => {
    const navigated = applyTerminalInputData(typeInput("previous"), "\x1b[B");
    for (const end of ["\r", "\u0003"]) {
      expect(applyTerminalInputData(navigated, end)).toEqual(
        createTerminalInputState(),
      );
    }
    const pasted = applyTerminalInputData(navigated, "\x1b[200~multi\nline\x1b[201~");
    expect(pasted.desynced).toBe(true);
    expect(pasted.multiline).toBe(true);
    expect(canRegisterCommandFromTracker(pasted)).toBe(false);
    const replaced = applyTerminalInputPreview(navigated, {
      kind: "replace",
      value: "echo $DISPLAY",
    });
    expect(getTrackedCommand(replaced)).toBe("echo $DISPLAY");
    expect(getTrackedSubmissionCommand(replaced)).toBe("echo $DISPLAY");
  });

  it("does not change selection suggestions while the cursor is before the end", () => {
    const moved = applyTerminalInputData(typeInput("echo $DISPLAY"), "\x1b[D");
    expect(moved.cursor).toBe(moved.value.length - 1);
    expect(canSuggestFromTracker(moved)).toBe(false);
  });
});
