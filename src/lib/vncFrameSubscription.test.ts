import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRemoteDesktopFramePatchBuffer } from "./remoteDesktopFrame";
import { VncFrameSubscription } from "./vncFrameSubscription";

const { subscribe, environment, subscriptions } = vi.hoisted(() => ({
  subscribe: vi.fn(),
  environment: { runtime: "desktop" },
  subscriptions: [] as Array<{
    frame: (buffer: ArrayBuffer) => void;
    error: (error: Error) => void;
    signal: AbortSignal;
    stop: ReturnType<typeof vi.fn>;
  }>,
}));
vi.mock("./vncFrames", () => ({ subscribeVncFrames: subscribe }));
vi.mock("./backend/runtime", () => environment);
vi.mock("./logger", () => ({
  logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

function frame(sequence: bigint, full = false) {
  return createRemoteDesktopFramePatchBuffer({
    sequence,
    desktopWidth: 2,
    desktopHeight: 1,
    x: full ? 0 : 1,
    y: 0,
    width: full ? 2 : 1,
    height: 1,
    stride: full ? 8 : 4,
    pixelFormat: "RGBA8888",
    payload: new Uint8Array(full ? 8 : 4),
  });
}
function setup(visible = true) {
  const options = {
    sessionId: "vnc-test",
    onPatch: vi.fn(),
    onReset: vi.fn(),
    onFailure: vi.fn(),
  };
  const stream = new VncFrameSubscription(options);
  stream.start(visible);
  return { stream, ...options };
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
  environment.runtime = "desktop";
  subscriptions.length = 0;
  subscribe.mockReset().mockImplementation((_id, frame, error, signal) => {
    const stop = vi.fn();
    subscriptions.push({ frame, error, signal, stop });
    return Promise.resolve(stop);
  });
});
afterEach(() => vi.useRealTimers());

describe("VncFrameSubscription", () => {
  it("accepts initial incremental frames, then recovers only from a full snapshot", async () => {
    const { stream, onPatch } = setup();
    subscriptions[0].frame(frame(7n));
    expect(onPatch).toHaveBeenCalledOnce();
    stream.recover("queue-overflow");
    expect(subscriptions[0].signal.aborted).toBe(true);
    subscriptions[0].frame(frame(8n));
    await vi.advanceTimersByTimeAsync(1000);
    const next = subscriptions[1];
    next.frame(frame(9n));
    expect(onPatch).toHaveBeenCalledOnce();
    next.frame(frame(10n, true));
    stream.presented();
    next.frame(frame(11n));
    next.frame(frame(10n));
    expect(onPatch.mock.calls.map(([patch]) => patch.sequence)).toEqual([
      7n,
      10n,
      11n,
    ]);
    await vi.advanceTimersByTimeAsync(6000);
    expect(subscribe).toHaveBeenCalledTimes(2);
    stream.stop();
  });

  it("counts failed presentations toward the retry limit even when complete frames arrive", async () => {
    const { stream, onFailure } = setup();
    stream.recover("render-failed");
    for (let attempt = 1; attempt <= 3; attempt++) {
      await vi.advanceTimersByTimeAsync(1000);
      subscriptions[attempt].frame(frame(BigInt(attempt), true));
      stream.recover("render-failed");
    }
    expect(onFailure).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(subscribe).toHaveBeenCalledTimes(4);
    stream.stop();
  });

  it("merges recovery requests, times out three times, and then exposes failure", async () => {
    const { stream, onFailure } = setup();
    stream.recover("context-lost");
    stream.recover("render-failed");
    await vi.advanceTimersByTimeAsync(17_000);
    expect(subscribe).toHaveBeenCalledTimes(4);
    expect(onFailure).toHaveBeenCalledOnce();
    expect(subscriptions.every((item) => item.signal.aborted)).toBe(true);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(subscribe).toHaveBeenCalledTimes(4);
    stream.stop();
  });

  it("stops hidden Desktop frames and resumes from the current complete desktop", async () => {
    const { stream, onPatch } = setup();
    subscriptions[0].frame(frame(1n));
    stream.setVisible(false);
    expect(subscriptions[0].signal.aborted).toBe(true);
    subscriptions[0].frame(frame(2n));
    await vi.advanceTimersByTimeAsync(31_000);
    expect(subscribe).toHaveBeenCalledOnce();
    stream.setVisible(true);
    subscriptions[1].frame(frame(3n));
    subscriptions[1].frame(frame(4n, true));
    expect(onPatch).toHaveBeenCalledTimes(2);
    stream.stop();
  });

  it("keeps a hidden Web socket alive beyond 30s without decoding incoming frames", async () => {
    environment.runtime = "web";
    const { stream, onPatch, onFailure } = setup();
    subscriptions[0].frame(frame(1n));
    stream.setVisible(false);
    subscriptions[0].frame(new ArrayBuffer(1));
    await vi.advanceTimersByTimeAsync(31_000);
    expect(subscriptions[0].signal.aborted).toBe(false);
    expect(subscribe).toHaveBeenCalledOnce();
    expect(onFailure).not.toHaveBeenCalled();
    expect(onPatch).toHaveBeenCalledOnce();
    stream.setVisible(true);
    expect(subscriptions[0].signal.aborted).toBe(true);
    subscriptions[1].frame(frame(2n, true));
    expect(onPatch).toHaveBeenCalledTimes(2);
    stream.stop();
  });

  it("cancels queued recovery on hide and ignores late attachment resolution", async () => {
    let resolve: (stop: () => void) => void = () => {};
    subscribe.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const { stream } = setup();
    stream.recover("queue-overflow");
    stream.setVisible(false);
    const lateStop = vi.fn();
    resolve(lateStop);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(lateStop).toHaveBeenCalledOnce();
    expect(subscribe).toHaveBeenCalledOnce();
    stream.stop();
  });

  it("resets sequence numbers and recovery timers on a remote reconnect", async () => {
    const { stream, onPatch, onFailure } = setup();
    subscriptions[0].frame(frame(100n));
    stream.recover("render-failed");
    await vi.advanceTimersByTimeAsync(1000);
    stream.resetForReconnect();
    subscriptions[1].frame(frame(0n));
    await vi.advanceTimersByTimeAsync(20_000);
    expect(onPatch.mock.calls.map(([patch]) => patch.sequence)).toEqual([
      100n,
      0n,
    ]);
    expect(onFailure).not.toHaveBeenCalled();
    stream.stop();
  });

  it("recovers malformed frame data without throwing from the transport callback", async () => {
    const { stream, onPatch } = setup();
    expect(() => subscriptions[0].frame(new ArrayBuffer(1))).not.toThrow();
    await vi.advanceTimersByTimeAsync(1000);
    subscriptions[1].frame(frame(1n, true));
    expect(onPatch).toHaveBeenCalledOnce();
    stream.stop();
  });

  it("never attaches a hidden Desktop pane and cancels all work on stop", async () => {
    const { stream } = setup(false);
    expect(subscribe).not.toHaveBeenCalled();
    stream.setVisible(true);
    stream.recover("render-failed");
    stream.stop();
    await vi.advanceTimersByTimeAsync(20_000);
    expect(subscribe).toHaveBeenCalledOnce();
    expect(subscriptions[0].signal.aborted).toBe(true);
  });

  it("requires a complete frame after recovery is interrupted before the first usable frame", async () => {
    const { stream, onPatch } = setup();
    subscriptions[0].frame(new ArrayBuffer(1));
    stream.setVisible(false);
    stream.setVisible(true);
    subscriptions[1].frame(frame(1n));
    expect(onPatch).not.toHaveBeenCalled();
    subscriptions[1].frame(frame(2n, true));
    stream.presented();
    await vi.advanceTimersByTimeAsync(6000);
    expect(onPatch).toHaveBeenCalledOnce();
    expect(subscribe).toHaveBeenCalledTimes(2);
    stream.stop();
  });
});
