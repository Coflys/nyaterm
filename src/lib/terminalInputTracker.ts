import type { SessionInputPreview } from "@/lib/sessionInput";
import { sanitizeTerminalCommand, stripTerminalCommandPrompt } from "@/lib/terminalCommand";

export interface TerminalInputState {
  value: string;
  cursor: number;
  desynced: boolean;
  desyncReason: "tab" | "history" | "terminal" | null;
  lineRewriteRequired: boolean;
  multiline: boolean;
  pasteMode: boolean;
}

export function createTerminalInputState(): TerminalInputState {
  return {
    value: "",
    cursor: 0,
    desynced: false,
    desyncReason: null,
    lineRewriteRequired: false,
    multiline: false,
    pasteMode: false,
  };
}

function resetState(multiline = false): TerminalInputState {
  return {
    value: "",
    cursor: 0,
    desynced: false,
    desyncReason: null,
    lineRewriteRequired: false,
    multiline,
    pasteMode: false,
  };
}

function insertText(state: TerminalInputState, text: string): TerminalInputState {
  if (!text) {
    return state;
  }

  const value = `${state.value.slice(0, state.cursor)}${text}${state.value.slice(state.cursor)}`;
  return {
    ...state,
    value,
    cursor: state.cursor + text.length,
    multiline: /[\r\n]/u.test(value),
  };
}

function deleteLeft(state: TerminalInputState): TerminalInputState {
  if (state.cursor === 0) {
    return state;
  }

  const value = `${state.value.slice(0, state.cursor - 1)}${state.value.slice(state.cursor)}`;
  return {
    ...state,
    value,
    cursor: state.cursor - 1,
    multiline: value.includes("\n"),
  };
}

function deleteRight(state: TerminalInputState): TerminalInputState {
  if (state.cursor >= state.value.length) {
    return state;
  }

  const value = `${state.value.slice(0, state.cursor)}${state.value.slice(state.cursor + 1)}`;
  return {
    ...state,
    value,
    multiline: value.includes("\n"),
  };
}

function deletePreviousWord(state: TerminalInputState): TerminalInputState {
  if (state.cursor === 0) {
    return state;
  }

  let start = state.cursor;
  while (start > 0 && /\s/u.test(state.value[start - 1] ?? "")) {
    start -= 1;
  }
  while (start > 0 && !/\s/u.test(state.value[start - 1] ?? "")) {
    start -= 1;
  }

  const value = `${state.value.slice(0, start)}${state.value.slice(state.cursor)}`;
  return {
    ...state,
    value,
    cursor: start,
    multiline: value.includes("\n"),
  };
}

export function deleteTerminalInputRange(
  state: TerminalInputState,
  start: number,
  end: number,
): TerminalInputState {
  const length = state.value.length;
  const from = Math.max(0, Math.min(length, Math.trunc(start)));
  const to = Math.max(from, Math.min(length, Math.trunc(end)));

  if (to <= from) {
    return state;
  }

  const value = `${state.value.slice(0, from)}${state.value.slice(to)}`;
  return {
    ...state,
    value,
    cursor: from,
    multiline: value.includes("\n"),
  };
}

function markDesynced(
  state: TerminalInputState,
  reason: "tab" | "history" | "terminal",
  multiline = false,
): TerminalInputState {
  return {
    ...state,
    desynced: true,
    desyncReason: reason,
    lineRewriteRequired: state.lineRewriteRequired || reason === "tab",
    multiline,
  };
}

function replaceValue(value: string): TerminalInputState {
  return {
    value,
    cursor: value.length,
    desynced: false,
    desyncReason: null,
    lineRewriteRequired: false,
    multiline: value.includes("\n"),
    pasteMode: false,
  };
}

function extractPastedInputData(
  state: TerminalInputState,
  data: string,
): { text: string; pasteMode: boolean } {
  let text = data;
  let pasteMode = state.pasteMode;

  if (text.includes("\x1b[200~")) {
    pasteMode = true;
    text = text.replace(/\x1b\[200~/gu, "");
  }

  if (text.includes("\x1b[201~")) {
    pasteMode = false;
    text = text.replace(/\x1b\[201~/gu, "");
  }

  return {
    text: text.replace(/\r\n|\r/gu, "\n"),
    pasteMode,
  };
}

export function applyTerminalPastedInputData(
  state: TerminalInputState,
  data: string,
): TerminalInputState {
  const pasted = extractPastedInputData(state, data);
  const pasteState = { ...state, pasteMode: pasted.pasteMode };
  if (!pasted.text) return pasteState;

  const nextState =
    pasteState.desynced && pasteState.desyncReason === "tab"
      ? {
          ...pasteState,
          desynced: false,
          desyncReason: null,
          lineRewriteRequired: true,
        }
      : pasteState;

  return insertText(nextState, pasted.text);
}

function normalizeLineContent(value: string): string {
  return value.replace(/\r?\n/gu, "");
}

function addCandidate(candidates: Set<string>, value: string): void {
  const normalized = stripTerminalCommandPrompt(normalizeLineContent(value));
  if (sanitizeTerminalCommand(normalized)) {
    candidates.add(normalized);
  }
}

function addSuffixCandidate(candidates: Set<string>, source: string, prefix: string): void {
  const normalizedSource = normalizeLineContent(source);
  const normalizedPrefix = normalizeLineContent(prefix);
  const comparableSource = sanitizeTerminalCommand(normalizedSource);
  const comparablePrefix = sanitizeTerminalCommand(normalizedPrefix);
  if (!comparableSource || !comparablePrefix) {
    return;
  }

  const index = comparableSource.lastIndexOf(comparablePrefix);
  if (index >= 0) {
    addCandidate(candidates, stripTerminalCommandPrompt(normalizedSource).slice(index));
  }
}

function chooseTerminalLineCommand(
  previousValue: string,
  lineContent: string,
  historyNavigation = false,
): string | null {
  const normalizedLine = normalizeLineContent(lineContent);
  const visibleCommand = stripTerminalCommandPrompt(normalizedLine);
  const hasKnownPrompt = visibleCommand !== normalizedLine.trimStart();
  if (hasKnownPrompt && !visibleCommand.trim()) {
    return "";
  }
  // History navigation can replace the whole command, unlike Tab completion.
  // Only trust a replacement when the visible prompt boundary is recognizable.
  if (historyNavigation) {
    return hasKnownPrompt ? visibleCommand : null;
  }

  const previousCommand = sanitizeTerminalCommand(previousValue);
  const sanitizedLine = sanitizeTerminalCommand(normalizedLine);
  const candidates = new Set<string>();

  addCandidate(candidates, sanitizedLine);
  addCandidate(candidates, lineContent);
  addSuffixCandidate(candidates, lineContent, previousValue);
  addSuffixCandidate(candidates, lineContent, previousCommand);
  addSuffixCandidate(candidates, sanitizedLine, previousValue);
  addSuffixCandidate(candidates, sanitizedLine, previousCommand);

  let best: { value: string; score: number } | null = null;
  for (const candidate of candidates) {
    const command = sanitizeTerminalCommand(candidate);
    if (!command) {
      continue;
    }

    const score = previousCommand && command.startsWith(previousCommand) ? command.length : 0;
    if (previousCommand && score === 0) {
      continue;
    }

    if (!best || score > best.score) {
      best = { value: candidate, score };
    }
  }

  return best?.value ?? null;
}

export function applyTerminalInputData(
  state: TerminalInputState,
  data: string,
): TerminalInputState {
  if (!data) {
    return state;
  }

  if (
    state.desynced &&
    state.desyncReason === "history" &&
    !state.pasteMode &&
    !data.includes("\x1b[200~") &&
    !data.includes("\x1b[201~") &&
    !data.includes("\n") &&
    !["\r", "\u0003", "\t", "\x1b[Z", "\x1b[A", "\x1bOA", "\x1b[B", "\x1bOB"].includes(
      data,
    ) &&
    (data.startsWith("\x1b") || /[\x00-\x1f\x7f]/u.test(data))
  ) {
    // A cursor edit or an unknown control sequence makes the buffer's
    // cursor-prefix insufficient to reconstruct the entire history line.
    return markDesynced(state, "terminal");
  }

  switch (data) {
    case "\r":
      return resetState();
    case "\u0003":
      return resetState();
    case "\u0001":
      return { ...state, cursor: 0 };
    case "\u0005":
      return { ...state, cursor: state.value.length };
    case "\u0015": {
      const value = state.value.slice(state.cursor);
      return { ...state, value, cursor: 0, multiline: value.includes("\n") };
    }
    case "\u0017":
      return deletePreviousWord(state);
    case "\u000b": {
      const value = state.value.slice(0, state.cursor);
      return { ...state, value, multiline: value.includes("\n") };
    }
    case "\u000c":
      return state;
    case "\u007f":
    case "\b":
      return deleteLeft(state);
    case "\x1b[D":
    case "\x1bOD":
      return { ...state, cursor: Math.max(0, state.cursor - 1) };
    case "\x1b[C":
    case "\x1bOC":
      return { ...state, cursor: Math.min(state.value.length, state.cursor + 1) };
    case "\x1b[H":
    case "\x1bOH":
      return { ...state, cursor: 0 };
    case "\x1b[F":
    case "\x1bOF":
      return { ...state, cursor: state.value.length };
    case "\x1b[3~":
      return deleteRight(state);
    case "\x1b[A":
    case "\x1bOA":
    case "\x1b[B":
    case "\x1bOB":
      return markDesynced(state, "history");
    case "\t":
    case "\x1b[Z":
      return state.desynced && state.desyncReason === "history"
        ? { ...markDesynced(state, "history"), lineRewriteRequired: true }
        : markDesynced(state, "tab");
  }

  if (state.pasteMode || data.includes("\x1b[200~") || data.includes("\x1b[201~")) {
    return applyTerminalPastedInputData(state, data);
  }

  if ((data.includes("\n") || data.includes("\r")) && data !== "\r") {
    return applyTerminalPastedInputData(state, data);
  }

  if (data.startsWith("\x1b")) {
    return markDesynced(state, "terminal");
  }

  if (/[\x00-\x1f\x7f]/u.test(data)) {
    return markDesynced(state, "terminal");
  }

  if (state.desynced && state.desyncReason === "history") {
    // Text sent before history echo is confirmed cannot be replayed against
    // a later screen snapshot: it may already be present in that snapshot.
    return insertText(markDesynced(state, "terminal"), data);
  }

  if (state.desynced && state.desyncReason === "tab") {
    if (state.value.length > 0) {
      // An unverified completion must not append to stale keystrokes and
      // re-enable fuzzy matching for a command the shell may have replaced.
      return insertText(state, data);
    }
    return insertText(
      {
        ...state,
        desynced: false,
        desyncReason: null,
        lineRewriteRequired: true,
      },
      data,
    );
  }

  return insertText(state, data);
}

export function applyTerminalInputPreview(
  state: TerminalInputState,
  preview: SessionInputPreview,
): TerminalInputState {
  switch (preview.kind) {
    case "data":
      return applyTerminalInputData(state, preview.data);
    case "replace":
      return replaceValue(preview.value);
    case "replace-and-execute":
      return resetState();
    case "reset":
      return resetState();
  }
}

export function getTrackedCommand(state: TerminalInputState): string {
  if (state.desynced || state.multiline) {
    return "";
  }
  return sanitizeTerminalCommand(state.value);
}

export function canRegisterCommandFromTracker(state: TerminalInputState): boolean {
  return !state.desynced && !state.multiline && !state.lineRewriteRequired;
}

export function getTrackedSubmissionCommand(state: TerminalInputState): string {
  if (!canRegisterCommandFromTracker(state)) {
    return "";
  }

  return sanitizeTerminalCommand(state.value);
}

/**
 * Replace the tracker value with command text read from the terminal buffer.
 * Used after Tab completion or shell history navigation: the terminal buffer
 * may contain a replaced command rather than the tracker's stale keystrokes.
 * History requires confirmation from parsed output for the same navigation;
 * keyboard/Enter callers must leave historyOutputConfirmed at its default.
 */
export function resyncFromTerminalLine(
  current: TerminalInputState,
  lineContent: string,
  historyOutputConfirmed = false,
): TerminalInputState | null {
  if (
    current.desyncReason === "terminal" ||
    (current.desyncReason === "history" && !historyOutputConfirmed)
  ) {
    return null;
  }
  const value = chooseTerminalLineCommand(
    current.value,
    lineContent,
    current.desyncReason === "history",
  );
  // A stale, prompt-only buffer must not erase a valid keystroke-tracked
  // command during an ordinary Enter; only history/Tab may have cleared it.
  if (value === null || (value === "" && !current.desynced)) {
    return null;
  }

  return {
    value,
    cursor: value.length,
    desynced: false,
    desyncReason: null,
    lineRewriteRequired: false,
    multiline: false,
    pasteMode: false,
  };
}

export function canSuggestFromTracker(state: TerminalInputState): boolean {
  return (
    !state.desynced &&
    !state.multiline &&
    state.cursor === state.value.length &&
    getTrackedCommand(state).length > 0
  );
}
