import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useRemoteStats } from "@/hooks/useRemoteStats";
import type { RemoteStats, SavedConnection } from "@/types/global";
import { updateConnectionAutoIconAfterSessionStart } from "./connectionAutoIcon";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  collectStats: vi.fn(),
  logError: vi.fn(),
}));

// 保留真实的 Hook、图标识别和请求合并逻辑，仅替换 IPC 与日志出口。
vi.mock("./invoke", () => ({ invoke: mocks.invoke }));
vi.mock("./logger", () => ({ logger: { error: mocks.logError } }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function remoteStats(usageSource: RemoteStats["cpu"]["usage_source"]): RemoteStats {
  return {
    system: { hostname: "host", uptime_sec: 100, os: "Ubuntu 24.04 LTS", arch: "x86_64" },
    load: { load1: 0.1, load5: 0.2, load15: 0.3 },
    cpu: {
      model: "CPU",
      cores: 8,
      usage: usageSource === "warming_up" ? null : 10,
      per_core: [],
      sample_window_ms: usageSource === "warming_up" ? null : 1000,
      usage_source: usageSource,
    },
    memory: { used: 1024, available: 1024, cached: 0 },
    networks: [],
    network_summary: { rx_bytes_per_sec: 0, tx_bytes_per_sec: 0 },
    disks: [],
  };
}

function mockConnection(connectionId: string) {
  const connection: SavedConnection = {
    id: connectionId,
    name: "Host",
    type: "ssh",
    icon_auto_detect: true,
  };
  mocks.invoke.mockImplementation((command: string) => {
    if (command === "get_saved_connections") return Promise.resolve([connection]);
    if (command === "get_remote_stats") return mocks.collectStats();
    if (command === "update_connection_icon") return Promise.resolve();
    return Promise.reject(new Error(`Unexpected command: ${command}`));
  });
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("remote stats and auto icon request sharing", () => {
  it.each([
    "monitor",
    "auto-icon",
  ] as const)("shares the startup request when %s starts first and preserves warmup and polling", async (firstConsumer) => {
    const sessionId = `shared-${firstConsumer}`;
    const connectionId = `conn-${firstConsumer}`;
    const initial = deferred<RemoteStats>();
    const warmup = deferred<RemoteStats>();
    const poll = deferred<RemoteStats>();
    const initialStats = remoteStats("warming_up");
    const warmupStats = remoteStats("aggregate");
    const polledStats = remoteStats("aggregate");
    mockConnection(connectionId);
    mocks.collectStats
      .mockReturnValueOnce(initial.promise)
      .mockReturnValueOnce(warmup.promise)
      .mockReturnValueOnce(poll.promise);

    const options = { connectionId, sessionId, remoteStatsEnabled: true };
    const earlyIconUpdate =
      firstConsumer === "auto-icon" ? updateConnectionAutoIconAfterSessionStart(options) : null;
    if (earlyIconUpdate) {
      await act(async () => Promise.resolve());
      expect(mocks.collectStats).toHaveBeenCalledTimes(1);
    }
    const { result, unmount } = renderHook(() => useRemoteStats(sessionId, true, 3));
    const iconUpdate = earlyIconUpdate ?? updateConnectionAutoIconAfterSessionStart(options);
    await act(async () => Promise.resolve());

    expect(mocks.collectStats).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith("get_remote_stats", { sessionId });
    expect(result.current.stats).toBeNull();

    await act(async () => {
      initial.resolve(initialStats);
      await iconUpdate;
    });
    expect(result.current.stats).toBe(initialStats);
    expect(mocks.invoke).toHaveBeenCalledWith("update_connection_icon", {
      connectionId,
      icon: "ubuntu",
      iconAutoDetect: true,
    });
    expect(mocks.collectStats).toHaveBeenCalledTimes(1);
    expect(mocks.logError).not.toHaveBeenCalled();

    await act(async () => vi.advanceTimersByTimeAsync(1000));
    expect(mocks.collectStats).toHaveBeenCalledTimes(2);
    await act(async () => warmup.resolve(warmupStats));
    expect(result.current.stats).toBe(warmupStats);

    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(mocks.collectStats).toHaveBeenCalledTimes(3);
    await act(async () => poll.resolve(polledStats));
    expect(result.current.stats).toBe(polledStats);
    unmount();
  });

  it("delivers a shared failure to both consumers and allows a shared retry", async () => {
    const sessionId = "shared-retry";
    const connectionId = "conn-retry";
    const initial = deferred<RemoteStats>();
    const retry = deferred<RemoteStats>();
    const error = new Error("Remote command timed out");
    const stats = remoteStats("aggregate");
    mockConnection(connectionId);
    mocks.collectStats.mockReturnValueOnce(initial.promise).mockReturnValueOnce(retry.promise);

    const { result, unmount } = renderHook(() => useRemoteStats(sessionId, true, 3));
    const options = { connectionId, sessionId, remoteStatsEnabled: true };
    const iconUpdate = updateConnectionAutoIconAfterSessionStart(options);
    await act(async () => Promise.resolve());
    expect(mocks.collectStats).toHaveBeenCalledTimes(1);

    await act(async () => {
      initial.reject(error);
      await iconUpdate;
    });
    expect(result.current.error).toBe(true);
    expect(result.current.stats).toBeNull();
    expect(mocks.logError).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ error }));
    expect(mocks.invoke).not.toHaveBeenCalledWith("update_connection_icon", expect.anything());

    act(() => result.current.refresh());
    const retryIconUpdate = updateConnectionAutoIconAfterSessionStart(options);
    await act(async () => Promise.resolve());
    expect(mocks.collectStats).toHaveBeenCalledTimes(2);

    await act(async () => {
      retry.resolve(stats);
      await retryIconUpdate;
    });
    expect(result.current.error).toBe(false);
    expect(result.current.stats).toBe(stats);
    expect(result.current.isManualRefreshing).toBe(false);
    expect(mocks.invoke).toHaveBeenCalledWith("update_connection_icon", {
      connectionId,
      icon: "ubuntu",
      iconAutoDetect: true,
    });
    expect(mocks.collectStats).toHaveBeenCalledTimes(2);
    expect(mocks.logError).toHaveBeenCalledTimes(1);
    unmount();
  });
});
