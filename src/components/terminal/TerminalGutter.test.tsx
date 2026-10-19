import { act, render } from "@testing-library/react";
import { Terminal } from "@xterm/xterm";
import { Profiler } from "react";
import { afterEach, expect, it, vi } from "vitest";
import TerminalGutter from "./TerminalGutter";

afterEach(() => {
  vi.unstubAllGlobals();
});

function createHarness(initialBuffer: "normal" | "alternate" = "normal") {
  let frameId = 0;
  const frames = new Map<number, FrameRequestCallback>();
  const requestFrame = vi.fn((callback: FrameRequestCallback) => {
    frames.set(++frameId, callback);
    return frameId;
  });
  vi.stubGlobal("requestAnimationFrame", requestFrame);
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));

  const listeners = {
    render: new Set<() => void>(),
    write: new Set<() => void>(),
    resize: new Set<() => void>(),
    scroll: new Set<(viewportY: number) => void>(),
    buffer: new Set<() => void>(),
  };
  const subscribe = <T,>(set: Set<T>, listener: T) => {
    set.add(listener);
    return { dispose: () => set.delete(listener) };
  };
  const element = document.createElement("div");
  element.innerHTML = '<div class="xterm-viewport"></div><div class="xterm-screen"></div>';
  const buffer = {
    baseY: 100,
    cursorY: 0,
    viewportY: 98,
    type: initialBuffer,
    getLine: vi.fn(() => ({ isWrapped: false })),
  };
  const terminal = {
    element,
    buffer: {
      active: buffer,
      onBufferChange: (listener: () => void) => subscribe(listeners.buffer, listener),
    },
    rows: 1,
    options: { fontSize: 12, fontFamily: "monospace" },
    _core: { _renderService: { dimensions: { css: { cell: { height: 18, width: 10 } } } } },
    onRender: (listener: () => void) => subscribe(listeners.render, listener),
    onWriteParsed: (listener: () => void) => subscribe(listeners.write, listener),
    onResize: (listener: () => void) => subscribe(listeners.resize, listener),
    onScroll: (listener: (viewportY: number) => void) => subscribe(listeners.scroll, listener),
  };
  const timestamps = new Map([[98, 1000]]);
  const props = {
    terminalRef: { current: terminal as unknown as Terminal },
    showLineNumbers: true,
    showTimestamps: false,
    timestampFormat: "[HH:mm:ss]",
    lineTimestamps: timestamps,
    getLineOffset: vi.fn(() => 0),
    sessionId: "session-1",
  };
  return {
    props,
    buffer,
    terminal,
    timestamps,
    listeners,
    frames,
    requestFrame,
    flush() {
      act(() => {
        const pending = [...frames.values()];
        frames.clear();
        for (const callback of pending) callback(0);
      });
    },
    repaint() {
      act(() => {
        for (const callback of listeners.write) callback();
        for (const callback of listeners.render) callback();
      });
    },
    scroll(viewportY: number) {
      act(() => {
        buffer.viewportY = viewportY;
        for (const callback of listeners.scroll) callback(viewportY);
      });
    },
    switchBuffer(type: "normal" | "alternate") {
      act(() => {
        buffer.type = type;
        for (const callback of listeners.buffer) callback();
      });
    },
    refresh() {
      act(() => {
        window.dispatchEvent(
          new CustomEvent("nyaterm:refresh-gutter", { detail: { sessionId: props.sessionId } }),
        );
      });
    },
  };
}

it("keeps the line number gutter wide after scrolling back across a digit boundary", () => {
  const harness = createHarness();
  const { container } = render(<TerminalGutter {...harness.props} />);
  harness.flush();
  const gutter = container.firstElementChild as HTMLElement;
  expect(gutter.style.width).toBe("32px");

  harness.scroll(99);
  harness.flush();
  expect(gutter.style.width).toBe("40px");

  harness.scroll(98);
  harness.flush();
  expect(gutter.style.width).toBe("40px");
});

it("coalesces repaint events and skips React commits when visible gutter data is unchanged", () => {
  const harness = createHarness();
  const onCommit = vi.fn();
  render(
    <Profiler id="gutter" onRender={onCommit}>
      <TerminalGutter {...harness.props} showTimestamps />
    </Profiler>,
  );
  harness.flush();
  onCommit.mockClear();
  harness.requestFrame.mockClear();

  for (let i = 0; i < 20; i += 1) harness.repaint();
  harness.refresh();
  expect(harness.requestFrame).toHaveBeenCalledTimes(1);
  harness.flush();
  expect(onCommit).not.toHaveBeenCalled();

  harness.repaint();
  harness.flush();
  expect(onCommit).not.toHaveBeenCalled();
});

it("updates timestamps, wrapped rows, and dimensions even when the viewport does not move", () => {
  const harness = createHarness();
  const { container } = render(<TerminalGutter {...harness.props} showTimestamps />);
  harness.flush();
  const gutter = container.firstElementChild as HTMLElement;
  const originalTimestamp = gutter.firstElementChild?.firstElementChild?.textContent;

  harness.timestamps.set(98, 5000);
  harness.refresh();
  harness.flush();
  expect(gutter.firstElementChild?.firstElementChild?.textContent).not.toBe(originalTimestamp);

  harness.buffer.getLine.mockReturnValue({ isWrapped: true });
  harness.repaint();
  harness.flush();
  expect(gutter.textContent).toBe("");

  harness.buffer.getLine.mockReturnValue({ isWrapped: false });
  harness.terminal._core._renderService.dimensions.css.cell = { height: 20, width: 12 };
  harness.terminal.options.fontFamily = "serif";
  harness.terminal.options.fontSize = 14;
  act(() => {
    for (const callback of harness.listeners.resize) callback();
  });
  harness.flush();
  expect(gutter.style.fontFamily).toBe("serif");
  expect(gutter.style.fontSize).toBe("14px");
  expect((gutter.firstElementChild as HTMLElement).style.height).toBe("20px");
  expect(gutter.textContent).toContain("99");
});

it("cancels pending work and detaches repaint listeners in alternate screen, then resumes", () => {
  const harness = createHarness();
  const { container } = render(<TerminalGutter {...harness.props} showTimestamps />);
  harness.flush();
  const gutter = container.firstElementChild as HTMLElement;
  const width = gutter.style.width;
  const originalTimestamp = gutter.firstElementChild?.firstElementChild?.textContent;

  harness.repaint();
  expect(harness.frames.size).toBe(1);
  harness.switchBuffer("alternate");
  expect(harness.frames.size).toBe(0);
  expect(harness.listeners.render.size).toBe(0);
  expect(harness.listeners.write.size).toBe(0);
  expect(harness.listeners.scroll.size).toBe(0);
  expect(harness.listeners.resize.size).toBe(0);
  expect(harness.listeners.buffer.size).toBe(1);
  expect(gutter.style.visibility).toBe("hidden");
  expect(gutter.style.width).toBe(width);
  harness.buffer.getLine.mockClear();
  harness.props.getLineOffset.mockClear();
  harness.requestFrame.mockClear();

  harness.timestamps.set(98, 5000);
  harness.repaint();
  harness.refresh();
  harness.flush();
  expect(harness.requestFrame).not.toHaveBeenCalled();
  expect(harness.buffer.getLine).not.toHaveBeenCalled();
  expect(harness.props.getLineOffset).not.toHaveBeenCalled();

  harness.switchBuffer("normal");
  harness.flush();
  expect(harness.listeners.render.size).toBe(1);
  expect(harness.listeners.write.size).toBe(1);
  expect(harness.listeners.scroll.size).toBe(1);
  expect(harness.listeners.resize.size).toBe(1);
  expect(gutter.style.visibility).toBe("");
  expect(gutter.style.width).toBe(width);
  expect(gutter.firstElementChild?.firstElementChild?.textContent).not.toBe(originalTimestamp);
});

it("does not schedule gutter work when mounted in alternate screen", () => {
  const harness = createHarness("alternate");
  const { container } = render(<TerminalGutter {...harness.props} />);
  harness.refresh();
  expect(harness.requestFrame).not.toHaveBeenCalled();
  expect(harness.buffer.getLine).not.toHaveBeenCalled();
  expect((container.firstElementChild as HTMLElement).style.visibility).toBe("hidden");

  harness.switchBuffer("normal");
  harness.flush();
  expect(container.textContent).toBe("99");
});

it("follows real xterm alternate-screen escape sequences without losing normal-buffer timestamps", async () => {
  const harness = createHarness();
  const terminal = new Terminal({ cols: 80, rows: 2 });
  // Supply only the measurement surface; xterm's parser and buffers remain real.
  Object.defineProperty(terminal, "element", { value: harness.terminal.element });
  const write = (data: string) => new Promise<void>((resolve) => terminal.write(data, resolve));
  const timestamps = new Map([[0, 1000]]);
  const terminalRef = { current: terminal };
  const gutterView = (suspended = false) => (
    <TerminalGutter
      {...harness.props}
      terminalRef={terminalRef}
      lineTimestamps={timestamps}
      showTimestamps
      suspended={suspended}
    />
  );
  const { container, rerender, unmount } = render(gutterView());

  try {
    await act(async () => {
      await write("shell prompt");
    });
    harness.flush();
    const gutter = container.firstElementChild as HTMLElement;
    const normalContent = gutter.textContent;
    const width = gutter.style.width;

    rerender(gutterView(true));
    expect(container.firstElementChild).toBe(gutter);
    expect(gutter.style.visibility).toBe("hidden");
    expect(gutter.style.width).toBe(width);
    await act(async () => {
      await write("\x1b[?1049h\x1b[2J\x1b[HTUI");
    });
    rerender(gutterView());
    expect(terminal.buffer.active.type).toBe("alternate");
    expect(harness.frames.size).toBe(0);
    expect(gutter.style.visibility).toBe("hidden");
    expect(gutter.style.width).toBe(width);
    harness.requestFrame.mockClear();
    await act(async () => {
      await write("\x1b[Hredraw\x1b[2;1Hcursor movement");
    });
    harness.refresh();
    expect(harness.requestFrame).not.toHaveBeenCalled();

    rerender(gutterView(true));
    await act(async () => {
      await write("\x1b[?1049l");
    });
    expect(container.firstElementChild).toBe(gutter);
    expect(gutter.style.visibility).toBe("hidden");
    expect(gutter.style.width).toBe(width);
    expect(harness.frames.size).toBe(0);
    rerender(gutterView());
    harness.flush();
    expect(terminal.buffer.active.type).toBe("normal");
    expect(gutter.style.visibility).toBe("");
    expect(gutter.style.width).toBe(width);
    expect(gutter.textContent).toBe(normalContent);
    expect([...timestamps]).toEqual([[0, 1000]]);
  } finally {
    unmount();
    terminal.dispose();
  }
});

it.each([
  { showLineNumbers: true, showTimestamps: false },
  { showLineNumbers: false, showTimestamps: true },
  { showLineNumbers: true, showTimestamps: true },
])("preserves gutter geometry through suspension and buffer switches (%o)", (settings) => {
  const harness = createHarness();
  const props = { ...harness.props, ...settings };
  const { container, rerender, unmount } = render(<TerminalGutter {...props} />);
  harness.flush();
  const gutter = container.firstElementChild as HTMLElement;
  const width = gutter.style.width;
  const content = gutter.textContent;
  const border = gutter.style.borderColor;
  harness.repaint();
  expect(harness.frames.size).toBe(1);
  rerender(<TerminalGutter {...props} suspended />);
  expect(container.firstElementChild).toBe(gutter);
  expect(gutter.style.width).toBe(width);
  expect(gutter.style.borderColor).toBe(border);
  expect(gutter.style.visibility).toBe("hidden");
  expect(harness.frames.size).toBe(0);
  for (const listeners of Object.values(harness.listeners)) expect(listeners.size).toBe(0);
  harness.requestFrame.mockClear();
  harness.buffer.getLine.mockClear();
  harness.props.getLineOffset.mockClear();

  harness.timestamps.set(98, 5000);
  harness.repaint();
  harness.refresh();
  harness.flush();
  expect(harness.requestFrame).not.toHaveBeenCalled();
  expect(harness.buffer.getLine).not.toHaveBeenCalled();
  expect(harness.props.getLineOffset).not.toHaveBeenCalled();
  expect(gutter.textContent).toBe(content);

  harness.switchBuffer("alternate");
  rerender(<TerminalGutter {...props} />);
  expect(harness.frames.size).toBe(0);
  expect(container.firstElementChild).toBe(gutter);
  expect(gutter.style.width).toBe(width);
  expect(gutter.style.visibility).toBe("hidden");
  harness.switchBuffer("normal");
  harness.flush();
  expect(gutter.style.width).toBe(width);
  expect(gutter.style.visibility).toBe("");
  if (settings.showLineNumbers) expect(gutter.textContent).toContain("99");
  if (settings.showTimestamps) expect(gutter.textContent).not.toBe(content);

  harness.repaint();
  unmount();
  expect(harness.frames.size).toBe(0);
  for (const listeners of Object.values(harness.listeners)) expect(listeners.size).toBe(0);
  harness.requestFrame.mockClear();
  harness.refresh();
  expect(harness.requestFrame).not.toHaveBeenCalled();
});

it("refreshes the current normal-buffer data after suspension", () => {
  const harness = createHarness();
  const { container, rerender } = render(<TerminalGutter {...harness.props} showTimestamps />);
  harness.flush();
  const gutter = container.firstElementChild as HTMLElement;
  const width = gutter.style.width;
  const content = gutter.textContent;

  rerender(<TerminalGutter {...harness.props} showTimestamps suspended />);
  harness.timestamps.set(98, 5000);
  rerender(<TerminalGutter {...harness.props} showTimestamps />);
  harness.flush();

  expect(container.firstElementChild).toBe(gutter);
  expect(gutter.style.visibility).toBe("");
  expect(gutter.style.width).toBe(width);
  expect(gutter.textContent).not.toBe(content);
  for (const listeners of Object.values(harness.listeners)) expect(listeners.size).toBe(1);
});

it.each([
  { showLineNumbers: true, showTimestamps: false },
  { showLineNumbers: false, showTimestamps: true },
  { showLineNumbers: true, showTimestamps: true },
])("reserves initial gutter space without scheduling work while suspended (%o)", (settings) => {
  const harness = createHarness();
  const props = { ...harness.props, ...settings };
  const { container, rerender } = render(<TerminalGutter {...props} suspended />);
  const gutter = container.firstElementChild as HTMLElement;
  expect(Number.parseFloat(gutter.style.width)).toBeGreaterThan(0);
  expect(gutter.style.visibility).toBe("hidden");
  harness.refresh();
  expect(harness.requestFrame).not.toHaveBeenCalled();
  expect(harness.buffer.getLine).not.toHaveBeenCalled();
  for (const listeners of Object.values(harness.listeners)) expect(listeners.size).toBe(0);

  rerender(<TerminalGutter {...props} />);
  harness.flush();
  expect(container.firstElementChild).toBe(gutter);
  expect(gutter.style.visibility).toBe("");
  expect(gutter.textContent).not.toBe("");
});

it.each([false, true])("omits gutter space when both columns are disabled (suspended=%s)", (suspended) => {
  const harness = createHarness();
  const { container } = render(
    <TerminalGutter {...harness.props} showLineNumbers={false} showTimestamps={false} suspended={suspended} />,
  );
  expect(container.firstElementChild).toBeNull();
});
