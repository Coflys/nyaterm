import { Terminal } from "@xterm/xterm";
import type { ILink } from "@xterm/xterm";
import { describe, expect, it, vi } from "vitest";
import {
  createTerminalImagePathLinkProvider,
  findTerminalImagePaths,
  imagePathFromFileUri,
} from "./terminalImagePathLinks";

function paths(text: string, windowsPaths = false) {
  return findTerminalImagePaths(text, { windowsPaths }).map((m) => m.path);
}

describe("findTerminalImagePaths", () => {
  it("matches absolute image paths wrapped by tool output", () => {
    const text = "● Read(/workspace/test/C4514C6B.png)";
    const [match] = findTerminalImagePaths(text);
    expect(match.path).toBe("/workspace/test/C4514C6B.png");
    expect(text.slice(match.start, match.end)).toBe(match.path);
  });

  it("matches home-relative paths, quotes and trailing punctuation", () => {
    expect(paths('saved "~/shots/a.JPG", then /tmp/b.webp.')).toEqual([
      "~/shots/a.JPG",
      "/tmp/b.webp",
    ]);
    expect(paths("--out=/tmp/c.gif and ![x](/tmp/d.bmp)")).toEqual([
      "/tmp/c.gif",
      "/tmp/d.bmp",
    ]);
  });

  it("keeps non-ASCII path segments", () => {
    expect(paths("Read(/workspace/短剧/封面.png)")).toEqual([
      "/workspace/短剧/封面.png",
    ]);
  });

  it("ignores URLs, relative paths and non-image extensions", () => {
    expect(paths("https://example.com/a.png file:///tmp/a.png")).toEqual([]);
    expect(paths("images/a.png ./b.png")).toEqual([]);
    expect(paths("/tmp/a.png.bak /tmp/notes.txt /tmp/a.pngx")).toEqual([]);
  });

  it("matches Windows drive paths only when enabled", () => {
    const text = String.raw`C:\Users\me\Pictures\a.png`;
    expect(paths(text)).toEqual([]);
    expect(paths(text, true)).toEqual([text]);
  });
});

describe("imagePathFromFileUri", () => {
  it("decodes file URIs that point at images", () => {
    expect(imagePathFromFileUri("file://myhost/tmp/a%20b.png")).toBe(
      "/tmp/a b.png",
    );
    expect(imagePathFromFileUri("file:///C:/Users/me/a.jpg")).toBe(
      "C:/Users/me/a.jpg",
    );
  });

  it("rejects other schemes and non-image files", () => {
    expect(imagePathFromFileUri("https://example.com/a.png")).toBeNull();
    expect(imagePathFromFileUri("file:///tmp/a.txt")).toBeNull();
    expect(imagePathFromFileUri("not a uri")).toBeNull();
  });
});

describe("createTerminalImagePathLinkProvider", () => {
  async function createTerminal(cols: number, data: string) {
    const terminal = new Terminal({ cols, rows: 10, allowProposedApi: true });
    await new Promise<void>((resolve) => terminal.write(data, resolve));
    return terminal;
  }

  function provide(
    terminal: Terminal,
    bufferLineNumber: number,
    activate = vi.fn(),
    enabled = true,
  ) {
    const provider = createTerminalImagePathLinkProvider(terminal, {
      isEnabled: () => enabled,
      activate,
    });
    let result: ILink[] | undefined;
    provider.provideLinks(bufferLineNumber, (links) => {
      result = links;
    });
    return result;
  }

  it("returns a 1-based inclusive range and activates with the path", async () => {
    const terminal = await createTerminal(80, "Read(/tmp/a.png)\r\n");
    const activate = vi.fn();
    const links = provide(terminal, 1, activate);

    expect(links).toHaveLength(1);
    expect(links?.[0].range).toEqual({
      start: { x: 6, y: 1 },
      end: { x: 15, y: 1 },
    });
    links?.[0].activate(new MouseEvent("mouseup"), links[0].text);
    expect(activate).toHaveBeenCalledWith(expect.any(MouseEvent), "/tmp/a.png");
  });

  it("follows soft-wrapped rows and wide characters", async () => {
    // "封面" is two double-width cells, so the path spans both rows.
    const terminal = await createTerminal(12, "x /tmp/封面/long.png\r\n");
    const fromFirstRow = provide(terminal, 1);
    const fromSecondRow = provide(terminal, 2);

    expect(fromFirstRow?.[0].text).toBe("/tmp/封面/long.png");
    expect(fromFirstRow?.[0].range).toEqual({
      start: { x: 3, y: 1 },
      end: { x: 8, y: 2 },
    });
    expect(fromSecondRow?.[0].range).toEqual(fromFirstRow?.[0].range);
  });

  it("returns nothing when disabled or when no image path is present", async () => {
    const terminal = await createTerminal(
      80,
      "Read(/tmp/a.png)\r\nplain text\r\n",
    );
    expect(provide(terminal, 1, vi.fn(), false)).toBeUndefined();
    expect(provide(terminal, 2)).toBeUndefined();
  });
});
