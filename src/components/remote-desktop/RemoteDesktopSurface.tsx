import {
  type CSSProperties,
  forwardRef,
  type ReactNode,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
import { RemoteDesktopFrameQueue } from "@/lib/remoteDesktopFrameQueue";
import { logger } from "@/lib/logger";
import type { RemoteDesktopFramePatch } from "@/lib/remoteDesktopFrame";
import { mapClientEventToRemoteDesktopPixel } from "@/lib/remoteDesktopViewport";
import {
  createRemoteDesktopRenderer,
  type RemoteDesktopRenderer,
} from "./renderer";

export type DesktopRecoveryReason =
  | "queue-overflow"
  | "context-lost"
  | "render-failed";

export type RemoteDesktopScaleMode = "fit" | "actual" | "stretch";

export interface RemoteDesktopSurfaceHandle {
  drawFrame(patch: RemoteDesktopFramePatch): void;
  reset(): void;
  focus(): void;
}

interface RemoteDesktopSurfaceProps {
  scaleMode: RemoteDesktopScaleMode;
  active: boolean;
  visible: boolean;
  inputEnabled?: boolean;
  className?: string;
  children?: ReactNode;
  onPointerMove?: (
    point: { x: number; y: number },
    event: PointerEvent,
  ) => void;
  onPointerButton?: (
    point: { x: number; y: number },
    button: number,
    pressed: boolean,
    event: PointerEvent,
  ) => void;
  onWheel?: (point: { x: number; y: number }, event: WheelEvent) => void;
  onFocus?: () => void;
  onBlur?: () => void;
  onRecoveryRequired?: (reason: DesktopRecoveryReason) => void;
  onFrameRendered?: () => void;
}

function canvasStyle(scaleMode: RemoteDesktopScaleMode): CSSProperties {
  if (scaleMode === "stretch") {
    return { width: "100%", height: "100%" };
  }
  if (scaleMode === "actual") {
    return {
      width: "auto",
      height: "auto",
      maxWidth: "none",
      maxHeight: "none",
    };
  }
  return { width: "100%", height: "100%", objectFit: "contain" };
}

export const RemoteDesktopSurface = forwardRef<
  RemoteDesktopSurfaceHandle,
  RemoteDesktopSurfaceProps
>(function RemoteDesktopSurface(
  {
    scaleMode,
    active,
    visible,
    inputEnabled = true,
    className,
    children,
    onPointerMove,
    onPointerButton,
    onWheel,
    onFocus,
    onBlur,
    onRecoveryRequired,
    onFrameRendered,
  },
  forwardedRef,
) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rendererRef = useRef<RemoteDesktopRenderer | null>(null);
  const pendingFramesRef = useRef(new RemoteDesktopFrameQueue());
  const blockedRef = useRef(false);
  const preferCanvas2dRef = useRef(false);
  const visibleRef = useRef(visible);
  visibleRef.current = visible;
  const recoveryCallbackRef = useRef(onRecoveryRequired);
  recoveryCallbackRef.current = onRecoveryRequired;
  const renderedCallbackRef = useRef(onFrameRendered);
  renderedCallbackRef.current = onFrameRendered;
  const lastLogRef = useRef(-Infinity);
  const [canvasVersion, setCanvasVersion] = useState(0);
  const frameRequestRef = useRef<number | null>(null);
  const style = useMemo(() => canvasStyle(scaleMode), [scaleMode]);

  const cancelPendingDraw = useCallback(() => {
    if (frameRequestRef.current !== null) {
      window.cancelAnimationFrame(frameRequestRef.current);
      frameRequestRef.current = null;
    }
    pendingFramesRef.current.clear();
  }, []);

  const disposeRenderer = useCallback(() => {
    rendererRef.current?.dispose();
    rendererRef.current = null;
    const canvas = canvasRef.current;
    if (canvas) {
      canvas.width = 0;
      canvas.height = 0;
    }
  }, []);

  const requireRecovery = useCallback(
    (reason: DesktopRecoveryReason) => {
      if (blockedRef.current) return;
      blockedRef.current = true;
      cancelPendingDraw();
      if (reason !== "queue-overflow") {
        preferCanvas2dRef.current = true;
        disposeRenderer();
        setCanvasVersion((version) => version + 1);
      }
      if (performance.now() - lastLogRef.current >= 5000) {
        lastLogRef.current = performance.now();
        logger.warn({
          domain: "session.lifecycle",
          event: "vnc.render_recovery",
          message: "Remote desktop rendering requires a complete frame",
          data: {
            reason,
            peakBytes: pendingFramesRef.current.peakBytes,
            peakCount: pendingFramesRef.current.peakCount,
            canvas2d: preferCanvas2dRef.current,
          },
        });
      }
      recoveryCallbackRef.current?.(reason);
    },
    [cancelPendingDraw, disposeRenderer],
  );

  const flushPendingFrames = useCallback(() => {
    frameRequestRef.current = null;
    const frames = pendingFramesRef.current.take();
    const canvas = canvasRef.current;
    if (
      !canvas ||
      !visibleRef.current ||
      blockedRef.current ||
      frames.length === 0
    )
      return;
    try {
      if (!rendererRef.current) {
        rendererRef.current = createRemoteDesktopRenderer(
          canvas,
          preferCanvas2dRef.current,
        );
        if (!rendererRef.current)
          throw new Error("Remote desktop rendering context unavailable");
        logger.debug({
          domain: "session.lifecycle",
          event: "vnc.renderer_created",
          message: "Remote desktop renderer initialized",
          data: { mode: rendererRef.current.mode },
        });
      }
      rendererRef.current.drawMany(frames);
      renderedCallbackRef.current?.();
    } catch {
      requireRecovery("render-failed");
    }
  }, [requireRecovery]);

  const scheduleFrameDraw = useCallback(
    (patch: RemoteDesktopFramePatch) => {
      if (!visibleRef.current || blockedRef.current) return;
      if (!pendingFramesRef.current.push(patch)) {
        requireRecovery("queue-overflow");
        return;
      }
      if (frameRequestRef.current !== null) return;
      frameRequestRef.current =
        window.requestAnimationFrame(flushPendingFrames);
    },
    [flushPendingFrames, requireRecovery],
  );

  useImperativeHandle(
    forwardedRef,
    () => ({
      drawFrame(patch) {
        scheduleFrameDraw(patch);
      },
      reset() {
        cancelPendingDraw();
        disposeRenderer();
        blockedRef.current = false;
        setCanvasVersion((version) => version + 1);
      },
      focus() {
        rootRef.current?.focus({ preventScroll: true });
      },
    }),
    [cancelPendingDraw, disposeRenderer, scheduleFrameDraw],
  );

  useEffect(() => {
    if (!visible) {
      cancelPendingDraw();
      disposeRenderer();
    }
  }, [visible, cancelPendingDraw, disposeRenderer]);

  const onContextLost = useCallback(
    (event: Event) => {
      event.preventDefault();
      requireRecovery("context-lost");
    },
    [requireRecovery],
  );
  const bindCanvas = useCallback(
    (canvas: HTMLCanvasElement | null) => {
      canvasRef.current?.removeEventListener("webglcontextlost", onContextLost);
      canvasRef.current = canvas;
      canvas?.addEventListener("webglcontextlost", onContextLost);
    },
    [onContextLost],
  );

  useEffect(
    () => () => {
      cancelPendingDraw();
      disposeRenderer();
    },
    [cancelPendingDraw, disposeRenderer],
  );

  const getPoint = (event: PointerEvent | WheelEvent) => {
    const canvas = canvasRef.current;
    return canvas
      ? mapClientEventToRemoteDesktopPixel(canvas, event, scaleMode)
      : { x: 0, y: 0 };
  };

  return (
    <div
      ref={rootRef}
      className={
        className ??
        "relative flex h-full w-full min-h-0 min-w-0 items-center justify-center overflow-hidden bg-black outline-none"
      }
      data-remote-desktop-input-root="true"
      tabIndex={active && visible ? 0 : -1}
      onFocus={onFocus}
      onBlur={(event) => {
        if (event.currentTarget.contains(event.relatedTarget as Node | null))
          return;
        onBlur?.();
      }}
    >
      <canvas
        key={canvasVersion}
        ref={bindCanvas}
        className="block"
        style={style}
        onPointerMove={
          inputEnabled && onPointerMove
            ? (event: ReactPointerEvent<HTMLCanvasElement>) =>
                onPointerMove(getPoint(event.nativeEvent), event.nativeEvent)
            : undefined
        }
        onPointerDown={
          inputEnabled && onPointerButton
            ? (event: ReactPointerEvent<HTMLCanvasElement>) => {
                event.currentTarget.setPointerCapture(event.pointerId);
                onPointerButton(
                  getPoint(event.nativeEvent),
                  event.button,
                  true,
                  event.nativeEvent,
                );
              }
            : undefined
        }
        onPointerUp={
          inputEnabled && onPointerButton
            ? (event: ReactPointerEvent<HTMLCanvasElement>) =>
                onPointerButton(
                  getPoint(event.nativeEvent),
                  event.button,
                  false,
                  event.nativeEvent,
                )
            : undefined
        }
        onWheel={
          inputEnabled && onWheel
            ? (event) => {
                event.preventDefault();
                onWheel(getPoint(event.nativeEvent), event.nativeEvent);
              }
            : undefined
        }
      />
      {children}
    </div>
  );
});
