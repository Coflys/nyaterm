import { renderHook, waitFor } from "@testing-library/react";
import type { Terminal } from "@xterm/xterm";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionType } from "@/types/global";
import type { TerminalFitScheduler } from "./terminalFitScheduler";
import { useTerminalRefreshEffects } from "./useTerminalRefreshEffects";

const windowMocks = vi.hoisted(() => ({
  focusChanged: undefined as ((event: { payload: boolean }) => void) | undefined,
  scaleChanged: undefined as
    | ((event: { payload: { scaleFactor: number } }) => void)
    | undefined,
}));

function createTerminal(baseY = 0, viewportY = baseY) {
  const scrollToBottom = vi.fn();
  return {
    terminal: {
      buffer: { active: { baseY, viewportY } },
      scrollToBottom,
    } as unknown as Terminal,
    scrollToBottom,
  };
}

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onResized: async () => vi.fn(),
    onMoved: async () => vi.fn(),
    onFocusChanged: async (callback: (event: { payload: boolean }) => void) => {
      windowMocks.focusChanged = callback;
      return vi.fn();
    },
    onScaleChanged: async (
      callback: (event: { payload: { scaleFactor: number } }) => void,
    ) => {
      windowMocks.scaleChanged = callback;
      return vi.fn();
    },
  }),
}));

describe("useTerminalRefreshEffects", () => {
  beforeEach(() => {
    windowMocks.focusChanged = undefined;
    windowMocks.scaleChanged = undefined;
    vi.restoreAllMocks();
  });

  it("restores the bottom viewport after an active terminal fit", () => {
    const schedule = vi.fn();
    const { terminal, scrollToBottom } = createTerminal(12, 12);
    renderHook(() =>
      useTerminalRefreshEffects({
        terminalRef: { current: terminal },
        fitSchedulerRef: {
          current: { schedule } as unknown as TerminalFitScheduler,
        },
        active: true,
        visible: true,
        appLocked: false,
        terminalReady: true,
        performanceMode: "normal",
        sessionId: "session-1",
        sessionType: "SSH",
        showGutter: false,
        showContentPadding: false,
      }),
    );

    const activeRefresh = schedule.mock.calls
      .map(([request]) => request)
      .find((request) => request.reason === "active");
    expect(activeRefresh).toEqual(
      expect.objectContaining({ force: true, refresh: true, focus: true }),
    );
    expect(activeRefresh).not.toHaveProperty("clearTextureAtlas");
    activeRefresh.onComplete({ applied: true });
    expect(scrollToBottom).toHaveBeenCalledTimes(1);
  });

  it("preserves manual scrollback after an active terminal fit", () => {
    const schedule = vi.fn();
    const { terminal, scrollToBottom } = createTerminal(12, 4);
    renderHook(() =>
      useTerminalRefreshEffects({
        terminalRef: { current: terminal },
        fitSchedulerRef: {
          current: { schedule } as unknown as TerminalFitScheduler,
        },
        active: true,
        visible: true,
        appLocked: false,
        terminalReady: true,
        performanceMode: "normal",
        sessionId: "session-1",
        sessionType: "SSH",
        showGutter: false,
        showContentPadding: false,
      }),
    );

    const activeRefresh = schedule.mock.calls
      .map(([request]) => request)
      .find((request) => request.reason === "active");
    activeRefresh.onComplete({ applied: true });
    expect(scrollToBottom).not.toHaveBeenCalled();
  });

  it("does not change the viewport when an active terminal fit is skipped", () => {
    const schedule = vi.fn();
    const { terminal, scrollToBottom } = createTerminal(12, 12);
    renderHook(() =>
      useTerminalRefreshEffects({
        terminalRef: { current: terminal },
        fitSchedulerRef: {
          current: { schedule } as unknown as TerminalFitScheduler,
        },
        active: true,
        visible: true,
        appLocked: false,
        terminalReady: true,
        performanceMode: "normal",
        sessionId: "session-1",
        sessionType: "SSH",
        showGutter: false,
        showContentPadding: false,
      }),
    );

    const activeRefresh = schedule.mock.calls
      .map(([request]) => request)
      .find((request) => request.reason === "active");
    activeRefresh.onComplete({ applied: false });
    expect(scrollToBottom).not.toHaveBeenCalled();
  });

  it("restores the terminal that owned input focus when the native window regains focus", async () => {
    const schedule = vi.fn();
    const focus = vi.fn();
    const textarea = document.createElement("textarea");
    document.body.append(textarea);
    textarea.focus();
    const { terminal } = createTerminal();
    Object.assign(terminal, { textarea, focus });
    renderHook(() =>
      useTerminalRefreshEffects({
        terminalRef: { current: terminal },
        fitSchedulerRef: {
          current: { schedule } as unknown as TerminalFitScheduler,
        },
        active: true,
        visible: true,
        appLocked: false,
        terminalReady: true,
        performanceMode: "normal",
        sessionId: "session-1",
        sessionType: "SSH",
        showGutter: false,
        showContentPadding: false,
      }),
    );
    await waitFor(() => expect(windowMocks.focusChanged).toBeTypeOf("function"));
    schedule.mockClear();
    const hasFocus = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    textarea.dispatchEvent(new FocusEvent("blur", { relatedTarget: null }));

    windowMocks.focusChanged?.({ payload: false });
    expect(schedule).not.toHaveBeenCalled();
    expect(focus).not.toHaveBeenCalled();

    hasFocus.mockReturnValue(true);
    windowMocks.focusChanged?.({ payload: true });
    expect(schedule).toHaveBeenCalledTimes(1);
    expect(focus).toHaveBeenCalledOnce();
    expect(schedule).toHaveBeenCalledWith(
      expect.objectContaining({
        reason: "window-focus",
        force: true,
        refresh: true,
        clearTextureAtlas: false,
        focus: false,
      }),
    );
  });

  it("does not reclaim terminal focus when the app locks while the native window is blurred", async () => {
    const schedule = vi.fn();
    const focus = vi.fn();
    const textarea = document.createElement("textarea");
    document.body.append(textarea);
    textarea.focus();
    const { terminal } = createTerminal();
    Object.assign(terminal, { textarea, focus });
    const terminalRef = { current: terminal };
    const fitSchedulerRef = {
      current: { schedule } as unknown as TerminalFitScheduler,
    };
    const { rerender } = renderHook(
      ({ appLocked }) =>
        useTerminalRefreshEffects({
          terminalRef,
          fitSchedulerRef,
          active: true,
          visible: true,
          appLocked,
          terminalReady: true,
          performanceMode: "normal",
          sessionId: "session-1",
          sessionType: "SSH",
          showGutter: false,
          showContentPadding: false,
        }),
      { initialProps: { appLocked: false } },
    );
    await waitFor(() => expect(windowMocks.focusChanged).toBeTypeOf("function"));
    schedule.mockClear();
    const hasFocus = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    textarea.dispatchEvent(new FocusEvent("blur", { relatedTarget: null }));

    rerender({ appLocked: true });
    hasFocus.mockReturnValue(true);
    windowMocks.focusChanged?.({ payload: true });

    expect(schedule).toHaveBeenCalledWith(
      expect.objectContaining({ reason: "window-focus", focus: false }),
    );
    expect(focus).not.toHaveBeenCalled();
  });

  it("does not reclaim focus after another in-app control owns DOM focus", async () => {
    const schedule = vi.fn();
    const focus = vi.fn();
    const textarea = document.createElement("textarea");
    const searchInput = document.createElement("input");
    document.body.append(textarea, searchInput);
    textarea.focus();
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const { terminal } = createTerminal();
    Object.assign(terminal, { textarea, focus });

    renderHook(() =>
      useTerminalRefreshEffects({
        terminalRef: { current: terminal },
        fitSchedulerRef: {
          current: { schedule } as unknown as TerminalFitScheduler,
        },
        active: true,
        visible: true,
        appLocked: false,
        terminalReady: true,
        performanceMode: "normal",
        sessionId: "session-1",
        sessionType: "SSH",
        showGutter: false,
        showContentPadding: false,
      }),
    );
    await waitFor(() => expect(windowMocks.focusChanged).toBeTypeOf("function"));
    schedule.mockClear();

    textarea.dispatchEvent(
      new FocusEvent("blur", {
        relatedTarget: searchInput,
      }),
    );
    windowMocks.focusChanged?.({ payload: true });

    expect(focus).not.toHaveBeenCalled();
    expect(schedule).toHaveBeenCalledWith(
      expect.objectContaining({
        reason: "window-focus",
        force: true,
        refresh: true,
        focus: false,
      }),
    );
  });

  it("keeps the active refresh focus path when a hidden terminal becomes active and visible", () => {
    const schedule = vi.fn();
    const { terminal } = createTerminal();
    const terminalRef = { current: terminal };
    const fitSchedulerRef = {
      current: { schedule } as unknown as TerminalFitScheduler,
    };
    const { rerender } = renderHook(
      ({ active, visible }) =>
        useTerminalRefreshEffects({
          terminalRef,
          fitSchedulerRef,
          active,
          visible,
          appLocked: false,
          terminalReady: true,
          performanceMode: "normal",
          sessionId: "session-1",
          sessionType: "SSH",
          showGutter: false,
          showContentPadding: false,
        }),
      { initialProps: { active: false, visible: false } },
    );
    schedule.mockClear();

    rerender({ active: true, visible: true });

    expect(schedule).toHaveBeenCalledWith(
      expect.objectContaining({
        reason: "active",
        force: true,
        refresh: true,
        focus: true,
      }),
    );
  });

  it("still invalidates textures after a DPI scale change", async () => {
    const schedule = vi.fn();
    const { terminal } = createTerminal();
    renderHook(() =>
      useTerminalRefreshEffects({
        terminalRef: { current: terminal },
        fitSchedulerRef: {
          current: { schedule } as unknown as TerminalFitScheduler,
        },
        active: true,
        visible: true,
        appLocked: false,
        terminalReady: true,
        performanceMode: "normal",
        sessionId: "session-1",
        sessionType: "SSH",
        showGutter: false,
        showContentPadding: false,
      }),
    );
    await waitFor(() =>
      expect(windowMocks.scaleChanged).toBeTypeOf("function"),
    );
    windowMocks.scaleChanged?.({ payload: { scaleFactor: 2 } });

    expect(schedule).toHaveBeenCalledWith(
      expect.objectContaining({
        reason: "scale-factor",
        force: true,
        refresh: true,
        clearTextureAtlas: true,
      }),
    );
  });

  it("suppresses incidental refreshes while a snapshot restore is finalizing", async () => {
    const schedule = vi.fn();
    const snapshotRestoringRef = { current: true };
    const { terminal } = createTerminal();
    renderHook(() =>
      useTerminalRefreshEffects({
        terminalRef: { current: terminal },
        fitSchedulerRef: {
          current: { schedule } as unknown as TerminalFitScheduler,
        },
        active: true,
        visible: true,
        appLocked: false,
        terminalReady: true,
        performanceMode: "normal",
        sessionId: "session-1",
        sessionType: "SSH",
        showGutter: false,
        showContentPadding: false,
        snapshotRestoringRef,
      }),
    );
    await waitFor(() =>
      expect(windowMocks.scaleChanged).toBeTypeOf("function"),
    );
    schedule.mockClear();

    window.dispatchEvent(new Event("nyaterm:refresh-terminals"));
    windowMocks.scaleChanged?.({ payload: { scaleFactor: 2 } });

    expect(schedule).not.toHaveBeenCalled();
  });

  it.each(["Serial", "SSH", "Telnet", "Local"] as const)(
    "routes the global clear event safely for %s",
    (sessionType: SessionType) => {
      const terminal = {
        clearSelection: vi.fn(),
        write: vi.fn(),
        input: vi.fn(),
        focus: vi.fn(),
        buffer: { active: { baseY: 0, viewportY: 0 } },
      } as unknown as Terminal;
      renderHook(() =>
        useTerminalRefreshEffects({
          terminalRef: { current: terminal },
          fitSchedulerRef: { current: null },
          active: true,
          visible: true,
          appLocked: false,
          terminalReady: true,
          performanceMode: "normal",
          sessionId: "session-1",
          sessionType,
          showGutter: false,
          showContentPadding: false,
        }),
      );

      window.dispatchEvent(new Event("nyaterm:clear-terminal"));
      expect(terminal.clearSelection).toHaveBeenCalledOnce();
      expect(terminal.focus).toHaveBeenCalledOnce();
      if (sessionType === "Serial") {
        expect(terminal.write).toHaveBeenCalledExactlyOnceWith("\x1b[2J");
        expect(terminal.input).not.toHaveBeenCalled();
      } else {
        expect(terminal.input).toHaveBeenCalledExactlyOnceWith("\x0c", true);
        expect(terminal.write).not.toHaveBeenCalled();
      }
    },
  );

  it.each([
    { active: false, appLocked: false },
    { active: true, appLocked: true },
  ])("does not clear an inactive or locked Serial tab (%o)", ({ active, appLocked }) => {
    const terminal = {
      clearSelection: vi.fn(),
      write: vi.fn(),
      input: vi.fn(),
      focus: vi.fn(),
    } as unknown as Terminal;
    renderHook(() =>
      useTerminalRefreshEffects({
        terminalRef: { current: terminal },
        fitSchedulerRef: { current: null },
        active,
        visible: true,
        appLocked,
        terminalReady: true,
        performanceMode: "normal",
        sessionId: "session-1",
        sessionType: "Serial",
        showGutter: false,
        showContentPadding: false,
      }),
    );

    window.dispatchEvent(new Event("nyaterm:clear-terminal"));
    expect(terminal.clearSelection).not.toHaveBeenCalled();
    expect(terminal.write).not.toHaveBeenCalled();
    expect(terminal.input).not.toHaveBeenCalled();
    expect(terminal.focus).not.toHaveBeenCalled();
  });
});
