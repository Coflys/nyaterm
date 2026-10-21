import type { Terminal } from "@xterm/xterm";
import { describe, expect, it, vi } from "vitest";
import { createTerminalInputState } from "@/lib/terminalInputTracker";
import { getInputIndexAtBufferPosition, readCurrentInputLine } from "./terminalInputSelection";

describe("terminal input line recovery", () => {
  it("preserves the prompt separator and stops at the cursor", () => {
    const translate = vi.fn((trim: boolean, start: number, end: number) =>
      trim ? "$" : "$ s  s   s".slice(start, end),
    );
    const terminal = {
      buffer: {
        active: {
          baseY: 4,
          cursorY: 2,
          cursorX: 2,
          getLine: (y: number) => (y === 6 ? { translateToString: translate } : undefined),
        },
      },
    } as unknown as Terminal;
    expect(readCurrentInputLine(terminal)).toBe("$ ");
    expect(translate).toHaveBeenCalledWith(false, 0, 2);
  });

  it("joins wrapped rows without trimming spaces or reading beyond the cursor", () => {
    const rows = ["unrelated", "$ echo  ", " a  b   ", " c tail"];
    const terminal = {
      cols: 8,
      buffer: {
        active: {
          baseY: 1,
          cursorY: 2,
          cursorX: 2,
          getLine: (y: number) => ({
            isWrapped: y > 1,
            getCell: () => undefined,
            translateToString: (_trim: boolean, start: number, end: number) =>
              rows[y]?.slice(start, end),
          }),
        },
      },
    } as unknown as Terminal;
    expect(readCurrentInputLine(terminal)).toBe("$ echo   a  b    c");
  });

  it.each([
    "rows",
    "characters",
    "missing prompt",
    "missing row",
  ])("rejects incomplete input when the %s budget or boundary is invalid", (reason) => {
    const cursorY = reason === "rows" ? 512 : 1;
    const translate = vi.fn(() => "x".repeat(reason === "characters" ? 65_537 : 80));
    const terminal = {
      cols: reason === "characters" ? 65_537 : 80,
      buffer: {
        active: {
          baseY: cursorY,
          cursorY: 0,
          cursorX: 1,
          getLine: (y: number) =>
            reason === "missing row" && y === 0
              ? undefined
              : {
                  isWrapped: y > 0 || reason === "missing prompt",
                  getCell: () => undefined,
                  translateToString: translate,
                },
        },
      },
    } as unknown as Terminal;
    expect(readCurrentInputLine(terminal)).toBe("");
    if (reason !== "characters") expect(translate).not.toHaveBeenCalled();
  });
});

describe("Smart Cursor logical line budget", () => {
  it("falls back before translating a line wrapped across thousands of rows", () => {
    const translate = vi.fn(() => "x".repeat(80));
    const terminal = {
      cols: 80,
      buffer: {
        active: {
          type: "normal",
          length: 10_000,
          baseY: 9_999,
          cursorY: 0,
          cursorX: 1,
          getLine: (y: number) =>
            y >= 0 && y < 10_000
              ? { isWrapped: y > 0, length: 80, translateToString: translate }
              : undefined,
        },
      },
    } as unknown as Terminal;
    const state = { ...createTerminalInputState(), value: "x", cursor: 1 };

    expect(getInputIndexAtBufferPosition(terminal, state, { x: 1, y: 9_999 })).toBeNull();
    expect(translate).not.toHaveBeenCalled();
  });
});
