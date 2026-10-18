import type {
  IBufferRange,
  ILink,
  ILinkProvider,
  Terminal,
} from "@xterm/xterm";
import {
  getFileExtension,
  IMAGE_PREVIEW_EXTENSIONS,
} from "@/components/panel/file-explorer/model";

export interface TerminalImagePathMatch {
  path: string;
  start: number;
  end: number;
}

export interface TerminalImagePathMatchOptions {
  /** Also match `C:\...` style paths; only meaningful for Windows local shells. */
  windowsPaths?: boolean;
}

export interface TerminalImagePathLinkProviderOptions {
  isEnabled: () => boolean;
  getMatchOptions?: () => TerminalImagePathMatchOptions;
  activate: (event: MouseEvent, path: string) => void;
  hover?: (event: MouseEvent, path: string) => void;
  leave?: () => void;
}

const MAX_WRAPPED_ROWS = 64;
const MAX_PATH_LENGTH = 4096;

const IMAGE_EXTENSION_PATTERN = [...IMAGE_PREVIEW_EXTENSIONS].join("|");
// Brackets and quotes are excluded so wrappers such as `Read(/a.png)` or
// `"/a.png"` are not swallowed into the path. Paths containing spaces are not
// matched; OSC 8 file links cover those.
const PATH_CHAR = String.raw`[^\s"'\`<>|()\[\]{}]`;
// A path must start at a token boundary. ":" is deliberately not a boundary so
// URLs such as https://host/a.png never produce a path match.
const IMAGE_PATH_PATTERN = new RegExp(
  String.raw`(?<=^|[\s"'\`(\[{<=,])(~?\/|[A-Za-z]:[\\/])${PATH_CHAR}*?\.(?:${IMAGE_EXTENSION_PATTERN})(?=$|[\s"'\`)\]}>,;:!?]|\.(?:$|\s))`,
  "gi",
);

export function isImagePreviewPath(path: string) {
  return IMAGE_PREVIEW_EXTENSIONS.has(getFileExtension(path));
}

export function findTerminalImagePaths(
  text: string,
  options: TerminalImagePathMatchOptions = {},
): TerminalImagePathMatch[] {
  if (!text.includes(".")) return [];

  const matches: TerminalImagePathMatch[] = [];
  const regex = new RegExp(IMAGE_PATH_PATTERN.source, IMAGE_PATH_PATTERN.flags);
  for (const match of text.matchAll(regex)) {
    const path = match[0];
    if (path.length > MAX_PATH_LENGTH) continue;
    if (!options.windowsPaths && /^[A-Za-z]:/.test(path)) continue;
    matches.push({ path, start: match.index, end: match.index + path.length });
  }
  return matches;
}

/** Returns the image path of a `file:` URI (e.g. from an OSC 8 hyperlink), or null. */
export function imagePathFromFileUri(uri: string): string | null {
  let url: URL;
  try {
    url = new URL(uri);
  } catch {
    return null;
  }
  if (url.protocol !== "file:") return null;

  let path: string;
  try {
    path = decodeURIComponent(url.pathname);
  } catch {
    return null;
  }
  // file:///C:/foo.png -> C:/foo.png. The host part names the machine that
  // emitted the link, which for a remote shell is the session's own host.
  if (/^\/[A-Za-z]:\//.test(path)) path = path.slice(1);
  return path && isImagePreviewPath(path) ? path : null;
}

interface WrappedLine {
  text: string;
  /** Buffer cell (0-based) of every UTF-16 code unit in `text`. */
  cells: { x: number; y: number }[];
}

function readWrappedLine(terminal: Terminal, y: number): WrappedLine | null {
  const buffer = terminal.buffer.active;
  if (!buffer.getLine(y)) return null;

  let startY = y;
  while (
    startY > 0 &&
    y - startY < MAX_WRAPPED_ROWS &&
    buffer.getLine(startY)?.isWrapped
  ) {
    startY -= 1;
  }
  let endY = y;
  while (
    endY - startY < MAX_WRAPPED_ROWS &&
    buffer.getLine(endY + 1)?.isWrapped
  ) {
    endY += 1;
  }

  let text = "";
  const cells: WrappedLine["cells"] = [];
  const cell = buffer.getNullCell();
  for (let row = startY; row <= endY; row++) {
    const line = buffer.getLine(row);
    if (!line) break;
    for (let x = 0; x < line.length; x++) {
      if (!line.getCell(x, cell)) break;
      // The trailing half of a wide character has width 0 and no content.
      if (cell.getWidth() === 0) continue;
      const chars = cell.getChars() || " ";
      text += chars;
      for (let i = 0; i < chars.length; i++) cells.push({ x, y: row });
    }
  }
  return { text, cells };
}

function toBufferRange(
  line: WrappedLine,
  match: TerminalImagePathMatch,
): IBufferRange | null {
  const first = line.cells[match.start];
  const last = line.cells[match.end - 1];
  if (!first || !last) return null;
  // ILink ranges are 1-based and inclusive; image extensions are ASCII, so the
  // last character always occupies exactly one cell.
  return {
    start: { x: first.x + 1, y: first.y + 1 },
    end: { x: last.x + 1, y: last.y + 1 },
  };
}

export function createTerminalImagePathLinkProvider(
  terminal: Terminal,
  options: TerminalImagePathLinkProviderOptions,
): ILinkProvider {
  return {
    provideLinks(bufferLineNumber, callback) {
      if (!options.isEnabled()) {
        callback(undefined);
        return;
      }
      const line = readWrappedLine(terminal, bufferLineNumber - 1);
      if (!line) {
        callback(undefined);
        return;
      }

      const links: ILink[] = [];
      for (const match of findTerminalImagePaths(
        line.text,
        options.getMatchOptions?.(),
      )) {
        const range = toBufferRange(line, match);
        if (
          !range ||
          range.start.y > bufferLineNumber ||
          range.end.y < bufferLineNumber
        ) {
          continue;
        }
        links.push({
          range,
          text: match.path,
          activate: (event) => options.activate(event, match.path),
          hover: options.hover
            ? (event) => options.hover?.(event, match.path)
            : undefined,
          leave: options.leave ? () => options.leave?.() : undefined,
        });
      }
      callback(links.length > 0 ? links : undefined);
    },
  };
}
