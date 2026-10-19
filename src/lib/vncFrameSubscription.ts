import { runtime } from "./backend/runtime";
import { logger } from "./logger";
import {
  decodeRemoteDesktopFramePatch,
  type RemoteDesktopFramePatch,
} from "./remoteDesktopFrame";
import { isFullDesktopFrame } from "./remoteDesktopFrameQueue";
import { subscribeVncFrames } from "./vncFrames";

interface Options {
  sessionId: string;
  onPatch: (patch: RemoteDesktopFramePatch) => void;
  onReset: () => void;
  onFailure: (error: Error) => void;
}

/** Owns one pane's display subscription, independently of keyboard focus. */
export class VncFrameSubscription {
  private disposed = false;
  private visible = false;
  private controller?: AbortController;
  private unsubscribe?: () => void;
  private retryTimer?: ReturnType<typeof setTimeout>;
  private deadline?: ReturnType<typeof setTimeout>;
  private awaitingFull = false;
  private awaitingPresentation = false;
  private recovering = false;
  private receivedFrame = false;
  private needsFullFrame = false;
  private failures = 0;
  private lastAttempt = -Infinity;
  private lastSequence: bigint | null = null;
  private lastRecoveryLog = -Infinity;
  private lastResultLog = -Infinity;

  constructor(private readonly options: Options) {}

  start(visible: boolean): void {
    this.visible = visible;
    if (visible || runtime === "web") this.attach(false);
  }

  setVisible(visible: boolean): void {
    if (this.disposed || this.visible === visible) return;
    this.visible = visible;
    this.clearTimers();
    this.recovering = false;
    this.failures = 0;
    this.options.onReset();
    if (!visible) {
      if (runtime === "desktop") this.detach();
      // Keep the Web socket attached, or the server expires the session after 30s.
      else if (!this.controller) this.attach(false);
    } else {
      this.detach();
      this.attach(this.receivedFrame || this.needsFullFrame);
    }
  }

  recover(reason: string): void {
    if (this.disposed || !this.visible) return;
    this.needsFullFrame = true;
    if (this.recovering) {
      if (this.awaitingPresentation)
        this.failAttempt(new Error(`VNC frame presentation failed: ${reason}`));
      return;
    }
    this.recovering = true;
    this.failures = 0;
    if (performance.now() - this.lastRecoveryLog >= 5000) {
      this.lastRecoveryLog = performance.now();
      logger.warn({
        domain: "session.lifecycle",
        event: "vnc.frame_recovery_started",
        message: "Restarting VNC display subscription",
        ids: { session_id: this.options.sessionId },
        data: { reason },
      });
    }
    this.scheduleRecovery();
  }

  /** Recovery succeeds only after the full snapshot has actually been drawn. */
  presented(): void {
    if (this.disposed || !this.awaitingPresentation) return;
    this.awaitingPresentation = false;
    this.recovering = false;
    this.needsFullFrame = false;
    this.failures = 0;
    clearTimeout(this.deadline);
    this.deadline = undefined;
    if (performance.now() - this.lastResultLog >= 5000) {
      this.lastResultLog = performance.now();
      logger.info({
        domain: "session.lifecycle",
        event: "vnc.frame_recovered",
        message: "VNC display recovered from a complete frame",
        ids: { session_id: this.options.sessionId },
      });
    }
  }

  /** Remote reconnect resets sequence numbers; it must not restart the session itself. */
  resetForReconnect(): void {
    if (this.disposed) return;
    this.clearTimers();
    this.recovering = false;
    this.awaitingFull = false;
    this.awaitingPresentation = false;
    this.failures = 0;
    this.receivedFrame = false;
    this.needsFullFrame = false;
    this.lastSequence = null;
    this.options.onReset();
    if (!this.controller && (this.visible || runtime === "web"))
      this.attach(false);
  }

  stop(): void {
    this.disposed = true;
    this.clearTimers();
    this.detach();
  }

  private clearTimers(): void {
    clearTimeout(this.retryTimer);
    clearTimeout(this.deadline);
    this.retryTimer = undefined;
    this.deadline = undefined;
    this.awaitingPresentation = false;
  }

  private detach(): void {
    const controller = this.controller;
    this.controller = undefined;
    controller?.abort();
    this.unsubscribe?.();
    this.unsubscribe = undefined;
  }

  private scheduleRecovery(): void {
    this.clearTimers();
    this.detach();
    const delay = Math.max(0, 1000 - (performance.now() - this.lastAttempt));
    this.retryTimer = setTimeout(() => {
      this.retryTimer = undefined;
      if (this.disposed || (!this.visible && runtime === "desktop")) return;
      this.options.onReset();
      this.attach(this.visible);
    }, delay);
  }

  private failAttempt(error: Error): void {
    if (this.disposed) return;
    this.clearTimers();
    this.detach();
    if (++this.failures >= 3) {
      this.recovering = false;
      logger.error({
        domain: "session.lifecycle",
        event: "vnc.frame_recovery_failed",
        message: "VNC display recovery failed",
        ids: { session_id: this.options.sessionId },
        error,
      });
      this.options.onFailure(error);
      return;
    }
    this.recovering = true;
    if (this.visible || runtime === "web") this.scheduleRecovery();
  }

  private attach(awaitFull: boolean): void {
    if (this.disposed) return;
    const controller = new AbortController();
    this.controller = controller;
    this.awaitingFull = awaitFull;
    this.lastSequence = null;
    this.lastAttempt = performance.now();
    const current = () =>
      !this.disposed &&
      this.controller === controller &&
      !controller.signal.aborted;
    if (awaitFull) {
      this.recovering = true;
      this.deadline = setTimeout(() => {
        if (current())
          this.failAttempt(new Error("VNC complete frame recovery timed out"));
      }, 5000);
    }
    const onFrame = (frame: ArrayBuffer) => {
      if (!current() || !this.visible) return;
      try {
        const patch = decodeRemoteDesktopFramePatch(frame);
        if (this.lastSequence !== null && patch.sequence <= this.lastSequence)
          return;
        if (this.awaitingFull && !isFullDesktopFrame(patch)) return;
        if (this.awaitingFull) {
          this.options.onReset();
          this.awaitingFull = false;
          this.awaitingPresentation = true;
        }
        this.receivedFrame = true;
        this.lastSequence = patch.sequence;
        this.options.onPatch(patch);
      } catch (error) {
        if (this.recovering) this.failAttempt(asError(error));
        else this.recover("decode-failed");
      }
    };
    void subscribeVncFrames(
      this.options.sessionId,
      onFrame,
      (error) => {
        if (current()) this.failAttempt(error);
      },
      controller.signal,
    )
      .then((unsubscribe) => {
        if (current()) this.unsubscribe = unsubscribe;
        else unsubscribe();
      })
      .catch((error: unknown) => {
        if (current()) this.failAttempt(asError(error));
      });
  }
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
