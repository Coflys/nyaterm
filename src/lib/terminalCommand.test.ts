import { describe, expect, it } from "vitest";
import { sanitizeTerminalCommand, stripTerminalCommandPrompt } from "./terminalCommand";

describe("terminal prompt boundaries", () => {
  it("strips a standalone POSIX dollar prompt but not command variables", () => {
    expect(stripTerminalCommandPrompt("$ s")).toBe("s");
    expect(stripTerminalCommandPrompt("$ s  s   s")).toBe("s  s   s");
    expect(stripTerminalCommandPrompt("$ ")).toBe("");
    expect(stripTerminalCommandPrompt("$HOME")).toBe("$HOME");
    expect(stripTerminalCommandPrompt("$s")).toBe("$s");
    expect(stripTerminalCommandPrompt("echo $DISPLAY")).toBe("echo $DISPLAY");
    expect(stripTerminalCommandPrompt('echo "$HOME"')).toBe('echo "$HOME"');
    expect(stripTerminalCommandPrompt("'$HOME'")).toBe("'$HOME'");
  });

  it("preserves existing POSIX, bracketed, PowerShell and Windows prefixes", () => {
    expect(stripTerminalCommandPrompt("alice@server:~/work$ echo $DISPLAY")).toBe(
      "echo $DISPLAY",
    );
    expect(stripTerminalCommandPrompt("[alice@server] # ls -la")).toBe("ls -la");
    expect(stripTerminalCommandPrompt("PS C:\\work> echo $DISPLAY")).toBe("echo $DISPLAY");
    expect(stripTerminalCommandPrompt("C:\\work> dir")).toBe("dir");
  });

  it("keeps command-internal whitespace when normalizing suggestions", () => {
    expect(sanitizeTerminalCommand("$ s  s   s  ")).toBe("s  s   s");
    expect(sanitizeTerminalCommand("$HOME  &&  echo $DISPLAY")).toBe(
      "$HOME  &&  echo $DISPLAY",
    );
  });
});
