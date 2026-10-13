import { afterEach, describe, expect, it, vi } from "vitest";
import type { RemoteStats } from "@/types/global";
import { getRemoteStats } from "./remoteStats";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("./invoke", () => ({ invoke: mocks.invoke }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("getRemoteStats", () => {
  it("coalesces concurrent requests for the same session", async () => {
    const pending = deferred<RemoteStats>();
    mocks.invoke.mockReturnValue(pending.promise);

    const first = getRemoteStats("session-a");
    const second = getRemoteStats("session-a");

    expect(second).toBe(first);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith("get_remote_stats", {
      sessionId: "session-a",
    });

    const stats = {} as RemoteStats;
    pending.resolve(stats);
    await expect(first).resolves.toBe(stats);
  });

  it("starts a fresh request after the previous request settles", async () => {
    const firstStats = {} as RemoteStats;
    const secondStats = {} as RemoteStats;
    mocks.invoke.mockResolvedValueOnce(firstStats).mockResolvedValueOnce(secondStats);

    await expect(getRemoteStats("session-a")).resolves.toBe(firstStats);
    await expect(getRemoteStats("session-a")).resolves.toBe(secondStats);

    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it("does not share requests between sessions", async () => {
    const pendingA = deferred<RemoteStats>();
    const pendingB = deferred<RemoteStats>();
    mocks.invoke.mockReturnValueOnce(pendingA.promise).mockReturnValueOnce(pendingB.promise);

    const requestA = getRemoteStats("session-a");
    const requestB = getRemoteStats("session-b");

    expect(requestA).not.toBe(requestB);
    expect(mocks.invoke).toHaveBeenCalledTimes(2);

    pendingA.resolve({} as RemoteStats);
    pendingB.resolve({} as RemoteStats);
    await Promise.all([requestA, requestB]);
  });

  it("clears a rejected request so it can be retried", async () => {
    const error = new Error("stats failed");
    mocks.invoke.mockRejectedValueOnce(error).mockResolvedValueOnce({} as RemoteStats);

    await expect(getRemoteStats("session-a")).rejects.toBe(error);
    await expect(getRemoteStats("session-a")).resolves.toBeDefined();

    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });
});
