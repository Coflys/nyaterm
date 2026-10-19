import { act, fireEvent, render } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RemoteDesktopFramePatch } from "@/lib/remoteDesktopFrame";
import {
  RemoteDesktopSurface,
  type RemoteDesktopSurfaceHandle,
} from "./RemoteDesktopSurface";

const { createRenderer, draw, drawMany, dispose } = vi.hoisted(() => ({
  createRenderer: vi.fn(),
  draw: vi.fn(),
  drawMany: vi.fn(),
  dispose: vi.fn(),
}));

vi.mock("./renderer", () => ({
  createRemoteDesktopRenderer: createRenderer,
}));
vi.mock("@/lib/logger", () => ({ logger: { debug: vi.fn(), warn: vi.fn() } }));

const patch: RemoteDesktopFramePatch = {
  sequence: 1n,
  desktopWidth: 10,
  desktopHeight: 10,
  x: 0,
  y: 0,
  width: 1,
  height: 1,
  stride: 4,
  pixelFormat: "RGBA8888",
  payload: new Uint8Array([0, 0, 0, 255]),
};

describe("RemoteDesktopSurface", () => {
  afterEach(() => vi.restoreAllMocks());
  beforeEach(() => {
    draw.mockReset();
    drawMany.mockReset();
    dispose.mockReset();
    createRenderer.mockReset();
    createRenderer.mockReturnValue({ draw, drawMany, dispose });
  });

  it("batches imperative frame draws on the next animation frame", () => {
    const ref = createRef<RemoteDesktopSurfaceHandle>();
    let rafCallback: FrameRequestCallback | null = null;
    const requestAnimationFrame = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((callback) => {
        rafCallback = callback;
        return 1;
      });
    const cancelAnimationFrame = vi
      .spyOn(window, "cancelAnimationFrame")
      .mockImplementation(() => {});
    render(<RemoteDesktopSurface ref={ref} scaleMode="fit" active visible />);

    act(() => ref.current?.drawFrame(patch));
    act(() => ref.current?.drawFrame({ ...patch, sequence: 2n }));

    expect(requestAnimationFrame).toHaveBeenCalledOnce();
    expect(createRenderer).not.toHaveBeenCalled();
    expect(drawMany).not.toHaveBeenCalled();

    act(() => rafCallback?.(16));

    expect(createRenderer).toHaveBeenCalledOnce();
    expect(drawMany).toHaveBeenCalledWith([patch, { ...patch, sequence: 2n }]);
    expect(draw).not.toHaveBeenCalled();

    requestAnimationFrame.mockRestore();
    cancelAnimationFrame.mockRestore();
  });

  it("implements fit, actual, and stretch as local canvas presentation modes", () => {
    const view = render(
      <RemoteDesktopSurface scaleMode="fit" active visible />,
    );
    const canvas = view.container.querySelector("canvas");
    expect(canvas?.style.objectFit).toBe("contain");
    expect(canvas?.style.width).toBe("100%");

    view.rerender(<RemoteDesktopSurface scaleMode="actual" active visible />);
    expect(canvas?.style.width).toBe("auto");
    expect(canvas?.style.maxWidth).toBe("none");

    view.rerender(<RemoteDesktopSurface scaleMode="stretch" active visible />);
    expect(canvas?.style.width).toBe("100%");
    expect(canvas?.style.height).toBe("100%");
    expect(canvas?.style.objectFit).toBe("");
  });

  it("marks its focus root as a remote desktop input surface", () => {
    const view = render(
      <RemoteDesktopSurface scaleMode="fit" active visible />,
    );
    expect(
      view.container.querySelector('[data-remote-desktop-input-root="true"]'),
    ).not.toBeNull();
  });

  it("disposes renderer resources on reset and unmount", () => {
    const ref = createRef<RemoteDesktopSurfaceHandle>();
    vi.spyOn(window, "requestAnimationFrame").mockReturnValue(1);
    const cancelAnimationFrame = vi
      .spyOn(window, "cancelAnimationFrame")
      .mockImplementation(() => {});
    const view = render(
      <RemoteDesktopSurface ref={ref} scaleMode="fit" active visible />,
    );
    act(() => ref.current?.drawFrame(patch));
    act(() => ref.current?.reset());
    expect(cancelAnimationFrame).toHaveBeenCalledWith(1);
    expect(dispose).not.toHaveBeenCalled();

    let rafCallback: FrameRequestCallback | null = null;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      rafCallback = callback;
      return 2;
    });
    act(() => ref.current?.drawFrame(patch));
    act(() => rafCallback?.(16));
    view.unmount();
    expect(dispose).toHaveBeenCalledOnce();

    vi.restoreAllMocks();
  });

  it("maps pointer coordinates with stretch presentation", () => {
    const onPointerMove = vi.fn();
    const view = render(
      <RemoteDesktopSurface
        scaleMode="stretch"
        active
        visible
        onPointerMove={onPointerMove}
      />,
    );
    const canvas = view.container.querySelector("canvas");
    if (!(canvas instanceof HTMLCanvasElement))
      throw new Error("expected canvas");
    canvas.width = 100;
    canvas.height = 100;
    canvas.getBoundingClientRect = () =>
      ({
        left: 10,
        top: 20,
        width: 300,
        height: 200,
        right: 310,
        bottom: 220,
        x: 10,
        y: 20,
        toJSON: () => ({}),
      }) as DOMRect;

    fireEvent.pointerMove(canvas, { clientX: 160, clientY: 120 });

    expect(onPointerMove).toHaveBeenCalledWith(
      { x: 50, y: 50 },
      expect.any(PointerEvent),
    );
  });

  it("bounds stalled RAF work and requests recovery once until reset", () => {
    const ref = createRef<RemoteDesktopSurfaceHandle>();
    const recovery = vi.fn();
    vi.spyOn(window, "requestAnimationFrame").mockReturnValue(1);
    const cancel = vi.spyOn(window, "cancelAnimationFrame");
    render(
      <RemoteDesktopSurface
        ref={ref}
        scaleMode="fit"
        active
        visible
        onRecoveryRequired={recovery}
      />,
    );
    act(() => {
      for (let i = 0; i < 1000; i++)
        ref.current?.drawFrame({ ...patch, sequence: BigInt(i) });
    });
    expect(cancel).toHaveBeenCalledWith(1);
    expect(recovery).toHaveBeenCalledExactlyOnceWith("queue-overflow");
    expect(drawMany).not.toHaveBeenCalled();
    act(() => ref.current?.reset());
    act(() => ref.current?.drawFrame(patch));
    expect(window.requestAnimationFrame).toHaveBeenCalledTimes(2);
  });

  it("uses a fresh canvas and stays in 2D after WebGL context loss", () => {
    const ref = createRef<RemoteDesktopSurfaceHandle>();
    const recovery = vi.fn();
    let raf: FrameRequestCallback = () => {};
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      raf = callback;
      return 1;
    });
    const view = render(
      <RemoteDesktopSurface
        ref={ref}
        scaleMode="fit"
        active
        visible
        onRecoveryRequired={recovery}
      />,
    );
    act(() => ref.current?.drawFrame(patch));
    act(() => raf(0));
    const oldCanvas = view.container.querySelector("canvas");
    if (!oldCanvas) throw new Error("canvas missing");
    const lost = new Event("webglcontextlost", { cancelable: true });
    act(() => oldCanvas.dispatchEvent(lost));
    expect(lost.defaultPrevented).toBe(true);
    expect(recovery).toHaveBeenCalledExactlyOnceWith("context-lost");
    expect(dispose).toHaveBeenCalledOnce();
    expect(view.container.querySelector("canvas")).not.toBe(oldCanvas);
    act(() => ref.current?.reset());
    act(() => ref.current?.drawFrame(patch));
    act(() => raf(16));
    expect(createRenderer).toHaveBeenLastCalledWith(
      view.container.querySelector("canvas"),
      true,
    );
  });

  it("replaces a canvas after initialization fails and captures drawing exceptions", () => {
    const ref = createRef<RemoteDesktopSurfaceHandle>();
    const recovery = vi.fn();
    let raf: FrameRequestCallback = () => {};
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      raf = callback;
      return 1;
    });
    createRenderer.mockImplementationOnce(() => {
      throw new Error("shader failed");
    });
    const view = render(
      <RemoteDesktopSurface
        ref={ref}
        scaleMode="fit"
        active
        visible
        onRecoveryRequired={recovery}
      />,
    );
    const original = view.container.querySelector("canvas");
    act(() => ref.current?.drawFrame(patch));
    expect(() => act(() => raf(0))).not.toThrow();
    expect(view.container.querySelector("canvas")).not.toBe(original);
    expect(recovery).toHaveBeenCalledWith("render-failed");
    act(() => ref.current?.reset());
    drawMany.mockImplementationOnce(() => {
      throw new Error("upload failed");
    });
    act(() => ref.current?.drawFrame(patch));
    expect(() => act(() => raf(16))).not.toThrow();
    expect(recovery).toHaveBeenCalledTimes(2);
  });

  it("disposes hidden rendering even when a previously queued RAF fires late", () => {
    const ref = createRef<RemoteDesktopSurfaceHandle>();
    let raf: FrameRequestCallback = () => {};
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      raf = callback;
      return 1;
    });
    const view = render(
      <RemoteDesktopSurface ref={ref} scaleMode="fit" active visible />,
    );
    act(() => ref.current?.drawFrame(patch));
    act(() => raf(0));
    act(() => ref.current?.drawFrame(patch));
    view.rerender(
      <RemoteDesktopSurface ref={ref} scaleMode="fit" active visible={false} />,
    );
    act(() => raf(16));
    act(() => ref.current?.drawFrame(patch));
    expect(drawMany).toHaveBeenCalledOnce();
    expect(dispose).toHaveBeenCalledOnce();
  });
});
