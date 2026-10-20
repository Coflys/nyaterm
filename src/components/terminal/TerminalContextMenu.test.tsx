import {
  act,
  createEvent,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import type { Terminal } from "@xterm/xterm";
import { useMemo, useRef } from "react";
import type React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { TerminalRightClickAction } from "@/lib/interactionSettings";
import { invoke } from "@/lib/invoke";
import {
  clearTerminalScreen,
  sendTerminalClearInput,
} from "@/lib/terminalControlInput";
import type { SessionType } from "@/types/global";
import { toast } from "sonner";
import TerminalContextMenu from "./TerminalContextMenu";

let rightClickAction: TerminalRightClickAction;
const testRuntime = vi.hoisted(() => ({ value: "web" as "desktop" | "web" }));

vi.mock("@/lib/backend/runtime", () => ({
  get runtime() {
    return testRuntime.value;
  },
  supports: (capability: string) =>
    testRuntime.value === "desktop" || capability === "ssh",
}));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("@/context/AppContext", () => ({
  useTerminalAppSettings: () => ({
    interaction: { terminal_right_click_action: rightClickAction },
    translation: {},
    search: { custom_engines: [] },
    ai: { enabled: false, terminal_ai_actions: [] },
    keybindings: {},
  }),
}));
vi.mock("@/hooks/useShortcutMap", () => ({ resolveDisplayKeys: () => "" }));
vi.mock("@/lib/aiEvents", () => ({ openAIAssistant: vi.fn() }));
vi.mock("@/lib/clipboard", () => ({ writeClipboardText: vi.fn() }));
vi.mock("@/lib/invoke", () => ({ invoke: vi.fn() }));
vi.mock("@/lib/terminalControlInput", () => ({
  clearTerminalScreen: vi.fn(),
  sendTerminalClearInput: vi.fn(),
}));
vi.mock("@/lib/windowManager", () => ({ openSettings: vi.fn() }));
vi.mock("../dialog/terminal/TranslationDialog", () => ({
  default: () => null,
}));

beforeEach(() => {
  testRuntime.value = "web";
  vi.mocked(invoke).mockReset();
  vi.mocked(toast.error).mockClear();
  vi.mocked(clearTerminalScreen).mockClear();
  vi.mocked(sendTerminalClearInput).mockClear();
});

describe("TerminalContextMenu right-click behavior", () => {
  beforeEach(() => {
    rightClickAction = "menu";
  });

  it("leaves the right-click event untouched when the action is off", () => {
    rightClickAction = "none";
    const onAncestorContextMenu = vi.fn();
    const onPasteClipboard = vi.fn();
    const { getByTestId } = renderTerminalContextMenu({
      onAncestorContextMenu,
      onPasteClipboard,
    });
    const event = createEvent.contextMenu(getByTestId("terminal-child"));

    fireEvent(getByTestId("terminal-child"), event);

    expect(onAncestorContextMenu).toHaveBeenCalledOnce();
    expect(event.defaultPrevented).toBe(false);
    expect(onPasteClipboard).not.toHaveBeenCalled();
    expect(
      document.querySelector('[data-slot="context-menu-content"]'),
    ).toBeNull();
  });

  it("pastes directly and consumes the context-menu event in paste mode", async () => {
    rightClickAction = "paste";
    const onAncestorContextMenu = vi.fn();
    const onPasteClipboard = vi.fn().mockResolvedValue(undefined);
    const clearSelection = vi.fn();
    const focus = vi.fn();
    const { getByTestId } = renderTerminalContextMenu({
      onAncestorContextMenu,
      onPasteClipboard,
      clearSelection,
      focus,
    });
    const event = createEvent.contextMenu(getByTestId("terminal-child"));

    fireEvent(getByTestId("terminal-child"), event);

    expect(event.defaultPrevented).toBe(true);
    expect(onAncestorContextMenu).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(onPasteClipboard).toHaveBeenCalledOnce();
      expect(clearSelection).toHaveBeenCalledOnce();
      expect(focus).toHaveBeenCalledOnce();
    });
    expect(
      document.querySelector('[data-slot="context-menu-content"]'),
    ).toBeNull();
  });

  it("pastes once for each consecutive context-menu event in paste mode", async () => {
    rightClickAction = "paste";
    const onPasteClipboard = vi.fn().mockResolvedValue(undefined);
    const clearSelection = vi.fn();
    const focus = vi.fn();
    const { getByTestId } = renderTerminalContextMenu({
      onPasteClipboard,
      clearSelection,
      focus,
    });

    fireEvent.contextMenu(getByTestId("terminal-child"));
    fireEvent.contextMenu(getByTestId("terminal-child"));

    await waitFor(() => {
      expect(onPasteClipboard).toHaveBeenCalledTimes(2);
      expect(clearSelection).toHaveBeenCalledTimes(2);
      expect(focus).toHaveBeenCalledTimes(2);
    });
    expect(
      document.querySelector('[data-slot="context-menu-content"]'),
    ).toBeNull();
  });

  it("opens the application context menu without pasting in menu mode", async () => {
    const onPasteClipboard = vi.fn();
    const { getByTestId } = renderTerminalContextMenu({ onPasteClipboard });

    fireEvent.contextMenu(getByTestId("terminal-child"));

    await waitFor(() => {
      expect(
        document.querySelector('[data-slot="context-menu-content"]'),
      ).not.toBeNull();
    });
    expect(onPasteClipboard).not.toHaveBeenCalled();
  });

  it.each(["Serial", "SSH", "Telnet", "Local"] as const)(
    "uses the appropriate clear-screen operation for %s",
    async (sessionType) => {
      const { getByTestId } = renderTerminalContextMenu({ sessionType });
      fireEvent.contextMenu(getByTestId("terminal-child"));
      await waitFor(() => expect(screen.getByText("terminalCtx.clearScreen")).not.toBeNull());
      fireEvent.click(screen.getByText("terminalCtx.clearScreen"));

      if (sessionType === "Serial") {
        expect(clearTerminalScreen).toHaveBeenCalledExactlyOnceWith(
          expect.anything(),
          { focus: true },
        );
        expect(sendTerminalClearInput).not.toHaveBeenCalled();
      } else {
        expect(sendTerminalClearInput).toHaveBeenCalledExactlyOnceWith(
          expect.anything(),
          { focus: true },
        );
        expect(clearTerminalScreen).not.toHaveBeenCalled();
      }
    },
  );

  it("keeps Clear All delegated for Serial without invoking either screen-only helper", async () => {
    const onClearAll = vi.fn();
    const { getByTestId } = renderTerminalContextMenu({
      sessionType: "Serial",
      onClearAll,
    });
    fireEvent.contextMenu(getByTestId("terminal-child"));
    await waitFor(() => expect(screen.getByText("terminalCtx.clearAll")).not.toBeNull());
    fireEvent.click(screen.getByText("terminalCtx.clearAll"));
    expect(onClearAll).toHaveBeenCalledOnce();
    expect(clearTerminalScreen).not.toHaveBeenCalled();
    expect(sendTerminalClearInput).not.toHaveBeenCalled();
  });

  it("blocks local and remote clear-screen operations while locked", async () => {
    const { getByTestId } = renderTerminalContextMenu({
      sessionType: "Serial",
      appLocked: true,
    });
    fireEvent.contextMenu(getByTestId("terminal-child"));
    await waitFor(() => expect(screen.getByText("terminalCtx.clearScreen")).not.toBeNull());
    fireEvent.click(screen.getByText("terminalCtx.clearScreen"));
    expect(clearTerminalScreen).not.toHaveBeenCalled();
    expect(sendTerminalClearInput).not.toHaveBeenCalled();
  });

  it("preserves the terminal DOM node when switching between actions", () => {
    const view = renderTerminalContextMenu();
    const terminalChild = view.getByTestId("terminal-child");

    rightClickAction = "none";
    view.rerenderMenu();
    expect(view.getByTestId("terminal-child")).toBe(terminalChild);

    rightClickAction = "paste";
    view.rerenderMenu();
    expect(view.getByTestId("terminal-child")).toBe(terminalChild);

    rightClickAction = "menu";
    view.rerenderMenu();
    expect(view.getByTestId("terminal-child")).toBe(terminalChild);
  });
});

describe("TerminalContextMenu close autofocus", () => {
  it.each([
    ["without a selection", ""],
    ["with a selection", "selected text"],
  ])("keeps focus in search after Find %s", async (_label, selection) => {
    renderFocusHarness(selection);
    const terminalInput = screen.getByTestId("terminal-input");
    terminalInput.focus();

    fireEvent.contextMenu(terminalInput);
    await waitFor(() =>
      expect(screen.getByText("terminalCtx.find")).not.toBeNull(),
    );
    fireEvent.click(screen.getByText("terminalCtx.find"));

    await waitFor(() => {
      expect(screen.queryByText("terminalCtx.find")).toBeNull();
      expect(document.activeElement).toBe(screen.getByTestId("search-input"));
    });
  });

  it("preserves the default close autofocus for ordinary menu actions", async () => {
    renderFocusHarness("");
    const terminalInput = screen.getByTestId("terminal-input");
    terminalInput.focus();

    fireEvent.contextMenu(terminalInput);
    await waitFor(() =>
      expect(screen.getByText("terminalCtx.clearAll")).not.toBeNull(),
    );
    fireEvent.click(screen.getByText("terminalCtx.clearAll"));

    await waitFor(() => {
      expect(screen.queryByText("terminalCtx.clearAll")).toBeNull();
      expect(document.activeElement).toBe(terminalInput);
    });
  });
});

describe("TerminalContextMenu temporary session encoding", () => {
  beforeEach(() => {
    testRuntime.value = "desktop";
    rightClickAction = "menu";
  });

  it.each(["", "selected text"])(
    "reads the active encoding and switches only this session with selection %s",
    async (selection) => {
      vi.mocked(invoke)
        .mockResolvedValueOnce("UTF-8")
        .mockResolvedValueOnce("GBK");
      const { getByTestId } = renderTerminalContextMenu({ selection });
      fireEvent.contextMenu(getByTestId("terminal-child"));

      await waitFor(() => {
        expect(invoke).toHaveBeenCalledWith("get_session_encoding", {
          sessionId: "session-1",
        });
        expect(screen.getByText("terminalCtx.encoding")).not.toBeNull();
      });
      fireEvent.pointerMove(screen.getByText("terminalCtx.encoding"), {
        pointerType: "mouse",
      });
      await waitFor(() => expect(screen.getByText("GBK")).not.toBeNull());
      fireEvent.click(screen.getByText("GBK"));
      await waitFor(() => {
        expect(invoke).toHaveBeenCalledWith("set_session_encoding", {
          sessionId: "session-1",
          encoding: "GBK",
        });
      });
    },
  );

  it("hides the encoding submenu in Web mode", async () => {
    testRuntime.value = "web";
    const { getByTestId } = renderTerminalContextMenu();
    fireEvent.contextMenu(getByTestId("terminal-child"));
    await waitFor(() =>
      expect(screen.getByText("terminalCtx.clearAll")).not.toBeNull(),
    );
    expect(screen.queryByText("terminalCtx.encoding")).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("disables the encoding submenu when the application is locked", async () => {
    const { getByTestId } = renderTerminalContextMenu({ appLocked: true });
    fireEvent.contextMenu(getByTestId("terminal-child"));
    await waitFor(() =>
      expect(screen.getByText("terminalCtx.encoding")).not.toBeNull(),
    );
    expect(
      screen
        .getByText("terminalCtx.encoding")
        .closest('[data-slot="context-menu-sub-trigger"]')
        ?.hasAttribute("data-disabled"),
    ).toBe(true);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("reports a failed encoding query instead of showing an incorrect active value", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("session ended"));
    const { getByTestId } = renderTerminalContextMenu();
    fireEvent.contextMenu(getByTestId("terminal-child"));
    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith(
        "terminalCtx.encodingLoadFailed",
      ),
    );
  });

  it("ignores a stale encoding query after reopening the context menu", async () => {
    let resolveFirst!: (value: string) => void;
    const firstQuery = new Promise<string>((resolve) => {
      resolveFirst = resolve;
    });
    vi.mocked(invoke)
      .mockReturnValueOnce(firstQuery)
      .mockResolvedValueOnce("GB18030");
    const { getByTestId } = renderTerminalContextMenu();
    fireEvent.contextMenu(getByTestId("terminal-child"));
    fireEvent.contextMenu(getByTestId("terminal-child"));
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.getByText("GB18030")).not.toBeNull());

    await act(async () => {
      resolveFirst("UTF-8");
    });
    expect(screen.getByText("GB18030")).not.toBeNull();
    expect(screen.queryByText("UTF-8")).toBeNull();
  });

  it("reports a rejected encoding change and retains the active encoding", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce("UTF-8")
      .mockRejectedValueOnce(new Error("session closed"));
    const { getByTestId } = renderTerminalContextMenu();
    fireEvent.contextMenu(getByTestId("terminal-child"));
    await waitFor(() => expect(screen.getByText("UTF-8")).not.toBeNull());
    fireEvent.pointerMove(screen.getByText("terminalCtx.encoding"), {
      pointerType: "mouse",
    });
    await waitFor(() => expect(screen.getByText("GBK")).not.toBeNull());
    fireEvent.click(screen.getByText("GBK"));
    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith(
        "terminalCtx.encodingChangeFailed",
      ),
    );
    expect(invoke).toHaveBeenCalledWith("set_session_encoding", {
      sessionId: "session-1",
      encoding: "GBK",
    });
  });
});

function renderFocusHarness(selection: string) {
  function FocusHarness() {
    const terminalInputRef = useRef<HTMLInputElement>(null);
    const searchInputRef = useRef<HTMLInputElement>(null);
    const terminal = useMemo(
      () =>
        ({
          clearSelection: vi.fn(),
          focus: () => terminalInputRef.current?.focus(),
          getSelection: () => selection,
        }) as unknown as Terminal,
      [],
    );

    return (
      <>
        <TerminalContextMenu
          sessionId="session-1"
          sessionType="SSH"
          appLocked={false}
          terminalRef={{ current: terminal }}
          onFind={() => {
            // Match TerminalSearchBar's existing post-show focus effect.
            window.setTimeout(() => searchInputRef.current?.focus(), 0);
          }}
          onPasteText={vi.fn()}
          onPasteClipboard={vi.fn()}
          onClearAll={vi.fn()}
        >
          <input ref={terminalInputRef} data-testid="terminal-input" />
        </TerminalContextMenu>
        <input ref={searchInputRef} data-testid="search-input" />
      </>
    );
  }

  return render(<FocusHarness />);
}

function renderTerminalContextMenu({
  onAncestorContextMenu = vi.fn(),
  onPasteClipboard = vi.fn(),
  clearSelection = vi.fn(),
  focus = vi.fn(),
  selection = "",
  appLocked = false,
  sessionType = "SSH",
  onClearAll = vi.fn(),
}: {
  onAncestorContextMenu?: () => void;
  onPasteClipboard?: () => Promise<void> | void;
  clearSelection?: () => void;
  focus?: () => void;
  selection?: string;
  appLocked?: boolean;
  sessionType?: SessionType;
  onClearAll?: () => void;
} = {}) {
  const terminal = {
    clearSelection,
    focus,
    getSelection: () => selection,
  } as unknown as Terminal;
  const terminalRef = { current: terminal } as React.RefObject<Terminal | null>;

  const element = () => (
    <div onContextMenu={onAncestorContextMenu}>
      <TerminalContextMenu
        sessionId="session-1"
        sessionType={sessionType}
        appLocked={appLocked}
        terminalRef={terminalRef}
        onFind={vi.fn()}
        onPasteText={vi.fn()}
        onPasteClipboard={onPasteClipboard}
        onClearAll={onClearAll}
      >
        <div data-testid="terminal-child" />
      </TerminalContextMenu>
    </div>
  );
  const view = render(element());

  return {
    ...view,
    rerenderMenu: () => view.rerender(element()),
  };
}
