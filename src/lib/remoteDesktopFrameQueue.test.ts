import { describe, expect, it } from "vitest";
import type { RemoteDesktopFramePatch } from "./remoteDesktopFrame";
import { RemoteDesktopFrameQueue } from "./remoteDesktopFrameQueue";

const patch = (
  overrides: Partial<RemoteDesktopFramePatch> = {},
): RemoteDesktopFramePatch => ({
  sequence: 1n,
  desktopWidth: 10,
  desktopHeight: 10,
  x: 1,
  y: 1,
  width: 1,
  height: 1,
  stride: 4,
  pixelFormat: "RGBA8888",
  payload: new Uint8Array(4),
  ...overrides,
});

describe("RemoteDesktopFrameQueue", () => {
  it("releases all patches on count overflow while RAF is stalled", () => {
    const queue = new RemoteDesktopFrameQueue();
    for (let i = 0; i < 128; i++) expect(queue.push(patch())).toBe(true);
    expect(queue.push(patch())).toBe(false);
    expect(queue.take()).toEqual([]);
    expect(queue.bytes).toBe(0);
    expect(queue.peakCount).toBe(128);
  });

  it("counts retained backing buffers rather than just payload views", () => {
    const queue = new RemoteDesktopFrameQueue();
    const buffer = new ArrayBuffer(64 * 1024 * 1024);
    const view = new Uint8Array(buffer, 0, 4);
    expect(queue.push(patch({ payload: view }))).toBe(true);
    expect(queue.push(patch({ payload: view }))).toBe(true);
    expect(queue.bytes).toBe(buffer.byteLength);
    expect(queue.push(patch())).toBe(false);
    expect(queue.bytes).toBe(0);
  });

  it("lets a 4K snapshot supersede old patches and keeps subsequent deltas", () => {
    const queue = new RemoteDesktopFrameQueue();
    const full = patch({
      desktopWidth: 3840,
      desktopHeight: 2160,
      x: 0,
      y: 0,
      width: 3840,
      height: 2160,
      stride: 15360,
      payload: new Uint8Array(3840 * 2160 * 4 + 44),
    });
    const delta = {
      ...patch(),
      desktopWidth: 3840,
      desktopHeight: 2160,
      sequence: 2n,
    };
    queue.push(patch());
    expect(queue.push(full)).toBe(true);
    expect(queue.push(delta)).toBe(true);
    expect(queue.take()).toEqual([full, delta]);
    expect(queue.bytes).toBe(0);
  });

  it("accepts the maximum supported desktop snapshot but caps retained bytes at 128 MiB", () => {
    const queue = new RemoteDesktopFrameQueue();
    const full = patch({
      desktopWidth: 7680,
      desktopHeight: 4320,
      x: 0,
      y: 0,
      width: 7680,
      height: 4320,
      stride: 30720,
      payload: new Uint8Array(7680 * 4320 * 4 + 44),
    });
    expect(queue.push(full)).toBe(true);
    expect(
      queue.push({ ...full, x: 1, width: 1, payload: new Uint8Array(4) }),
    ).toBe(false);
    expect(
      queue.push(patch({ payload: new Uint8Array(128 * 1024 * 1024 + 1) })),
    ).toBe(false);
  });

  it("discards patches belonging to a previous desktop size", () => {
    const queue = new RemoteDesktopFrameQueue();
    queue.push(patch());
    const resized = patch({ desktopWidth: 20 });
    queue.push(resized);
    expect(queue.take()).toEqual([resized]);
  });
});
