import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { StrictMode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { VncSessionPane } from "@/types/global";
import VncPaneHost from "./VncPaneHost";
import { createRemoteDesktopFramePatchBuffer } from "@/lib/remoteDesktopFrame";

const {
  invokeMock,
  listenMock,
  listeners,
  readClipboardTextMock,
  writeClipboardTextMock,
} = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  readClipboardTextMock: vi.fn(),
  writeClipboardTextMock: vi.fn(),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
}));

vi.mock("@/lib/clipboard", () => ({
  readClipboardText: readClipboardTextMock,
  writeClipboardText: writeClipboardTextMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class Channel<T> {
    onmessage: (message: T) => void;

    constructor(onmessage: (message: T) => void) {
      this.onmessage = onmessage;
    }
  },
}));

describe("VncPaneHost", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    readClipboardTextMock.mockReset();
    readClipboardTextMock.mockResolvedValue("");
    writeClipboardTextMock.mockReset();
    writeClipboardTextMock.mockResolvedValue(undefined);
    listenMock.mockReset();
    listeners.clear();
    listenMock.mockImplementation(
      (eventName: string, handler: (event: { payload: unknown }) => void) => {
        listeners.set(eventName, handler);
        return Promise.resolve(vi.fn());
      },
    );
  });

  it("attaches the frame channel and subscribes to VNC state", async () => {
    render(<VncPaneHost pane={vncPane()} active visible />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("vnc_attach_frame_channel", {
        sessionId: "vnc-session",
        frameChannel: expect.any(Object),
      });
    });
    expect(listenMock).toHaveBeenCalledWith(
      "vnc-state-vnc-session",
      expect.any(Function),
    );
  });

  it("survives StrictMode mounting, pane moves, and late frames from old owners", async () => {
    const view = render(
      <StrictMode>
        <VncPaneHost pane={vncPane()} active visible />
      </StrictMode>,
    );
    await waitFor(() =>
      expect(
        invokeMock.mock.calls.some(
          ([name]) => name === "vnc_attach_frame_channel",
        ),
      ).toBe(true),
    );
    const oldChannel = invokeMock.mock.calls.find(
      ([name]) => name === "vnc_attach_frame_channel",
    )?.[1].frameChannel;
    view.unmount();
    const moved = render(
      <VncPaneHost pane={vncPane()} active={false} visible />,
    );
    await waitFor(() =>
      expect(
        invokeMock.mock.calls.filter(
          ([name]) => name === "vnc_attach_frame_channel",
        ),
      ).toHaveLength(2),
    );
    // An obsolete owner must not decode or update the newly mounted pane.
    expect(() => oldChannel.onmessage(new ArrayBuffer(1))).not.toThrow();
    expect(invokeMock).not.toHaveBeenCalledWith(
      "close_vnc_session",
      expect.anything(),
    );
    moved.unmount();
  });

  it("detaches hidden Desktop panes without closing their VNC sessions", async () => {
    const view = render(<VncPaneHost pane={vncPane()} active visible />);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "vnc_attach_frame_channel",
        expect.anything(),
      ),
    );
    view.rerender(<VncPaneHost pane={vncPane()} active visible={false} />);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("vnc_detach_frame_channel", {
        sessionId: "vnc-session",
      }),
    );
    view.rerender(<VncPaneHost pane={vncPane()} active={false} visible />);
    await waitFor(() =>
      expect(
        invokeMock.mock.calls.filter(
          ([name]) => name === "vnc_attach_frame_channel",
        ),
      ).toHaveLength(2),
    );
    expect(invokeMock).not.toHaveBeenCalledWith(
      "close_vnc_session",
      expect.anything(),
    );
  });

  it("ignores hidden data before decoding and clears desktop dimensions when the session changes", async () => {
    const view = render(<VncPaneHost pane={vncPane()} active visible />);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "vnc_attach_frame_channel",
        expect.anything(),
      ),
    );
    const channel = invokeMock.mock.calls.find(
      ([name]) => name === "vnc_attach_frame_channel",
    )?.[1].frameChannel;
    view.rerender(<VncPaneHost pane={vncPane()} active visible={false} />);
    act(() =>
      channel.onmessage(
        createRemoteDesktopFramePatchBuffer({
          sequence: 1n,
          desktopWidth: 2,
          desktopHeight: 1,
          x: 0,
          y: 0,
          width: 2,
          height: 1,
          stride: 8,
          pixelFormat: "RGBA8888",
          payload: new Uint8Array(8),
        }),
      ),
    );
    expect(screen.queryByText("2x1")).toBeNull();
    view.rerender(
      <VncPaneHost
        pane={vncPane({ sessionId: "replacement" })}
        active
        visible
      />,
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("vnc_attach_frame_channel", {
        sessionId: "replacement",
        frameChannel: expect.anything(),
      }),
    );
    expect(screen.queryByText("2x1")).toBeNull();
  });

  it("forwards state failures", async () => {
    const onConnectionError = vi.fn();
    render(
      <VncPaneHost
        pane={vncPane()}
        active
        visible
        onConnectionError={onConnectionError}
      />,
    );

    await waitFor(() =>
      expect(listeners.has("vnc-state-vnc-session")).toBe(true),
    );
    act(() => {
      listeners.get("vnc-state-vnc-session")?.({
        payload: {
          sessionId: "vnc-session",
          state: "failed",
          message: "unsupported security",
        },
      });
    });

    expect(screen.getByText("unsupported security")).not.toBeNull();
    expect(onConnectionError).toHaveBeenCalledWith(
      "vnc-session",
      "unsupported security",
    );
  });

  it("sends keysym keyboard events only when active", async () => {
    render(<VncPaneHost pane={vncPane()} active visible />);
    await activate();
    invokeMock.mockClear();

    const root = document.querySelector(
      '[data-remote-desktop-input-root="true"]',
    );
    if (!(root instanceof HTMLElement))
      throw new Error("expected VNC input root");
    fireEvent.keyDown(root, { key: "ArrowLeft", code: "ArrowLeft" });
    fireEvent.keyUp(root, { key: "ArrowLeft", code: "ArrowLeft" });

    expect(invokeMock).toHaveBeenCalledWith("vnc_input_batch", {
      sessionId: "vnc-session",
      events: [{ type: "key", keysym: 0xff51, pressed: true }],
    });
    expect(invokeMock).toHaveBeenCalledWith("vnc_input_batch", {
      sessionId: "vnc-session",
      events: [{ type: "key", keysym: 0xff51, pressed: false }],
    });
  });

  it("does not send local input for view-only panes", async () => {
    render(
      <VncPaneHost
        pane={vncPane({ display: { scaleMode: "fit", viewOnly: true } })}
        active
        visible
      />,
    );
    await activate();
    invokeMock.mockClear();

    const root = document.querySelector(
      '[data-remote-desktop-input-root="true"]',
    );
    if (!(root instanceof HTMLElement))
      throw new Error("expected VNC input root");
    fireEvent.keyDown(root, { key: "ArrowLeft", code: "ArrowLeft" });

    expect(invokeMock).not.toHaveBeenCalledWith(
      "vnc_input_batch",
      expect.anything(),
    );
  });

  it("keeps reconnect and close controls wired", async () => {
    const onDisconnectedCloseRequested = vi.fn();
    render(
      <VncPaneHost
        pane={vncPane()}
        active
        visible
        onDisconnectedCloseRequested={onDisconnectedCloseRequested}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Reconnect VNC session" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Close VNC session" }));

    expect(invokeMock).toHaveBeenCalledWith("vnc_reconnect", {
      sessionId: "vnc-session",
    });
    expect(onDisconnectedCloseRequested).toHaveBeenCalledOnce();
  });

  it("writes remote clipboard events to the local clipboard when enabled and focused", async () => {
    render(
      <VncPaneHost
        pane={vncPane({
          display: { scaleMode: "fit", clipboardEnabled: true },
        })}
        active
        visible
      />,
    );
    await activate();
    await waitFor(() =>
      expect(listeners.has("vnc-clipboard-vnc-session")).toBe(true),
    );

    act(() => {
      listeners.get("vnc-clipboard-vnc-session")?.({
        payload: { sessionId: "vnc-session", text: "remote" },
      });
    });

    expect(writeClipboardTextMock).toHaveBeenCalledWith("remote");
  });

  it("does not subscribe to remote clipboard events when clipboard is disabled", async () => {
    render(
      <VncPaneHost
        pane={vncPane({
          display: { scaleMode: "fit", clipboardEnabled: false },
        })}
        active
        visible
      />,
    );
    await activate();

    expect(listeners.has("vnc-clipboard-vnc-session")).toBe(false);
  });

  it("polls local clipboard and sends changed Latin-1 text to the VNC server", async () => {
    readClipboardTextMock.mockResolvedValue("local");
    render(
      <VncPaneHost
        pane={vncPane({
          display: { scaleMode: "fit", clipboardEnabled: true },
        })}
        active
        visible
      />,
    );
    await activate();

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("vnc_set_clipboard_text", {
        sessionId: "vnc-session",
        text: "local",
      });
    });
  });

  it("does not send local clipboard text for view-only panes", async () => {
    readClipboardTextMock.mockResolvedValue("local");
    render(
      <VncPaneHost
        pane={vncPane({
          display: { scaleMode: "fit", clipboardEnabled: true, viewOnly: true },
        })}
        active
        visible
      />,
    );
    await activate();
    invokeMock.mockClear();

    await new Promise((resolve) => window.setTimeout(resolve, 0));

    expect(invokeMock).not.toHaveBeenCalledWith(
      "vnc_set_clipboard_text",
      expect.anything(),
    );
  });
});

async function activate() {
  await waitFor(() =>
    expect(listeners.has("vnc-state-vnc-session")).toBe(true),
  );
  act(() => {
    listeners.get("vnc-state-vnc-session")?.({
      payload: { sessionId: "vnc-session", state: "active" },
    });
  });
}

function vncPane(overrides: Partial<VncSessionPane> = {}): VncSessionPane {
  return {
    id: "vnc-pane",
    kind: "leaf",
    paneKind: "remote-desktop",
    sessionId: "vnc-session",
    name: "VNC Desktop",
    type: "VNC",
    connectionId: "vnc-connection",
    display: { scaleMode: "fit" },
    ...overrides,
  };
}
