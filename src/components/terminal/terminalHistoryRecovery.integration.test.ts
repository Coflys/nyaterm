import type { Terminal as XTermTerminal } from "@xterm/xterm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  applyTerminalInputData,
  canSuggestFromTracker,
  createTerminalInputState,
  getTrackedSubmissionCommand,
  resyncFromTerminalLine,
} from "@/lib/terminalInputTracker";
import { createTerminalHistoryRecovery } from "./terminalHistoryRecovery";
import { readCurrentInputLine } from "./terminalInputSelection";

describe("history navigation with real xterm parsing", () => {
  const terminals: XTermTerminal[] = [];

  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
      measureText: () => ({ width: 10 }),
    } as unknown as CanvasRenderingContext2D);
  });

  afterEach(() => {
    for (const terminal of terminals.splice(0)) terminal.dispose();
    vi.restoreAllMocks();
  });

  async function createHarness(initial = "old", cols = 80) {
    const { Terminal } = await import("@xterm/xterm");
    const terminal = new Terminal({ cols, rows: 10 });
    terminals.push(terminal);
    const write = (data: string) => new Promise<void>((resolve) => terminal.write(data, resolve));
    await write(`$ ${initial}`);
    let state = applyTerminalInputData(createTerminalInputState(), initial);
    let outputPending = false;
    const recovery = createTerminalHistoryRecovery(terminal, () => outputPending);
    const submissions: string[] = [];
    terminal.onWriteParsed(() => {
      state = recovery.onWriteParsed(state) ?? state;
    });
    terminal.onData((data) => {
      if (data === "\r") {
        // This is also the Enter fallback in XTerminal; it must not bypass
        // the output confirmation required by history recovery.
        const recovered = resyncFromTerminalLine(state, readCurrentInputLine(terminal));
        submissions.push(getTrackedSubmissionCommand(recovered ?? state));
      }
      state = recovery.applyInput(state, data);
    });
    return {
      terminal,
      write,
      input: (data: string) => terminal.input(data, true),
      state: () => state,
      submissions,
      setOutputPending: (pending: boolean) => {
        outputPending = pending;
      },
    };
  }

  it("recovers a long history command across wraps and scrollback", async () => {
    const h = await createHarness("old", 16);
    const command = `echo ${"a  b   ".repeat(30)}end`;
    h.input("\x1b[A");
    await h.write(`\r\x1b[2K$ ${command}`);
    expect(h.terminal.buffer.active.baseY).toBeGreaterThan(0);
    expect(readCurrentInputLine(h.terminal)).toBe(`$ ${command}`);
    expect(h.state().value).toBe(command);
    h.input("s");
    expect(canSuggestFromTracker(h.state())).toBe(true);
    expect(h.state().value).toBe(`${command}s`);
    h.input("\r");
    expect(h.submissions).toEqual([`${command}s`]);
  });

  it("preserves wide characters at wrap boundaries", async () => {
    const h = await createHarness("old", 16);
    const command = `echo ${"x".repeat(8)}界  界`;
    h.input("\x1b[A");
    await h.write(`\r\x1b[2K$ ${command}`);
    expect(readCurrentInputLine(h.terminal)).toBe(`$ ${command}`);
    expect(h.state().value).toBe(command);
  });

  it.each([
    "\x1b[B",
    "\x1bOB",
  ])("recovers an empty Down prompt after parsed output (%j)", async (down) => {
    const h = await createHarness("");
    h.input(down);
    expect(h.state().desynced).toBe(true);
    await h.write("\x07");
    h.input("s");
    expect(h.state().value).toBe("s");
    expect(canSuggestFromTracker(h.state())).toBe(true);
  });

  it("recovers Up then Down once each navigation has echoed", async () => {
    const h = await createHarness("");
    h.input("\x1b[A");
    await h.write("\r\x1b[2K$ history");
    h.input("\x1b[B");
    await h.write("\r\x1b[2K$ ");
    h.input("s");
    expect(h.state().value).toBe("s");
    expect(canSuggestFromTracker(h.state())).toBe(true);
  });

  it("does not register the stale command if Enter beats history echo", async () => {
    const h = await createHarness();
    h.input("\x1b[A");
    h.input("\r");
    expect(h.submissions).toEqual([""]);
    await h.write("\r\x1b[2K$ history\r\n$ ");
    expect(h.state()).toEqual(createTerminalInputState());
  });

  it("keeps rapid typing suppressed even when history and character echoes arrive separately", async () => {
    const h = await createHarness();
    h.input("\x1b[A");
    h.input("s");
    expect(h.state().desynced).toBe(true);
    expect(canSuggestFromTracker(h.state())).toBe(false);
    await h.write("\r\x1b[2K$ history");
    await h.write("s");
    expect(canSuggestFromTracker(h.state())).toBe(false);
    h.input("\r");
    expect(h.submissions).toEqual([""]);
    h.input("fresh");
    expect(canSuggestFromTracker(h.state())).toBe(true);
  });

  it("does not mistake the first of multiple delayed navigation echoes for the last", async () => {
    const h = await createHarness();
    h.input("\x1b[A");
    h.input("\x1b[B");
    await h.write("\r\x1b[2K$ history");
    expect(h.state().desynced).toBe(true);
    h.input("\r");
    expect(h.submissions).toEqual([""]);
  });

  it("ignores unrelated output and output already queued before navigation", async () => {
    const h = await createHarness();
    h.input("\x1b[A");
    await h.write("\x1b[31m\x1b[0m");
    expect(h.state().desynced).toBe(true);
    h.input("\u0003");
    h.input("old");
    h.setOutputPending(true);
    h.input("\x1b[A");
    h.setOutputPending(false);
    await h.write("\r\x1b[2K$ queued before Up");
    h.input("\r");
    expect(h.submissions).toEqual([""]);
  });

  it.each([
    "\x1b[D",
    "\t",
    "\x1b[200~pasted\x1b[201~",
  ])("does not recover a screen snapshot after another edit (%j)", async (edit) => {
    const h = await createHarness();
    h.input("\x1b[A");
    h.input(edit);
    await h.write("\r\x1b[2K$ history");
    expect(h.state().desynced).toBe(true);
    h.input("\r");
    expect(h.submissions).toEqual([""]);
  });

  it("requires a known prompt and normal buffer", async () => {
    const h = await createHarness();
    h.input("\x1b[A");
    await h.write("\r\x1b[2Kcustom>> history");
    expect(h.state().desynced).toBe(true);
    await h.write("\x1b[?1049h$ alternate");
    expect(h.state().desynced).toBe(true);
    h.input("\r");
    expect(h.submissions).toEqual([""]);
  });
});
