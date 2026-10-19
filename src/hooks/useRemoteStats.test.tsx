import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { RemoteStats } from "@/types/global";
import { useRemoteStats } from "./useRemoteStats";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@/lib/remoteStats", () => ({
  getRemoteStats: (sessionId: string) => mocks.invoke("get_remote_stats", { sessionId }),
}));

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason?: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe("useRemoteStats session ownership", () => {
  it("evicts closed sessions while preserving live caches without restarting their polling", async () => {
    const statsA = remoteStats("Session A", "aggregate");
    const statsB = remoteStats("Session B", "aggregate");
    const refreshA = deferred<RemoteStats>();
    const refreshB = deferred<RemoteStats>();
    mocks.invoke
      .mockResolvedValueOnce(statsA)
      .mockResolvedValueOnce(statsB)
      .mockReturnValueOnce(refreshA.promise)
      .mockReturnValueOnce(refreshB.promise);
    const allSessions = new Set(["session-a", "session-b"]);
    const { result, rerender } = renderHook(
      ({ sessionId, liveSessionIds }) => useRemoteStats(sessionId, true, 60, liveSessionIds),
      {
        initialProps: {
          sessionId: "session-a",
          liveSessionIds: allSessions as ReadonlySet<string> | null,
        },
      },
    );
    await waitFor(() => expect(result.current.stats).toBe(statsA));
    rerender({ sessionId: "session-b", liveSessionIds: allSessions });
    await waitFor(() => expect(result.current.stats).toBe(statsB));

    // An unknown list must not evict caches; removing A must not restart B's polling.
    rerender({ sessionId: "session-b", liveSessionIds: null });
    rerender({ sessionId: "session-b", liveSessionIds: new Set(["session-b"]) });
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    expect(result.current.stats).toBe(statsB);

    rerender({ sessionId: "session-a", liveSessionIds: allSessions });
    expect(result.current.stats).toBeNull();
    rerender({ sessionId: "session-b", liveSessionIds: allSessions });
    expect(result.current.stats).toBe(statsB);
    await act(async () => refreshA.resolve(remoteStats("Stale A", "aggregate")));
    await act(async () => refreshB.resolve(remoteStats("Fresh B", "aggregate")));
  });

  it("stops monitoring a closed active session and ignores its pending response", async () => {
    vi.useFakeTimers();
    const pendingRefresh = deferred<RemoteStats>();
    const reopenedRefresh = deferred<RemoteStats>();
    mocks.invoke
      .mockResolvedValueOnce(remoteStats("Session A", "aggregate"))
      .mockReturnValueOnce(pendingRefresh.promise)
      .mockReturnValueOnce(reopenedRefresh.promise);
    const liveSessions = new Set(["session-a"]);
    const { result, rerender } = renderHook(
      ({ liveSessionIds }) => useRemoteStats("session-a", true, 3, liveSessionIds),
      { initialProps: { liveSessionIds: liveSessions } },
    );
    await act(async () => Promise.resolve());
    act(() => result.current.refresh());
    expect(result.current.isManualRefreshing).toBe(true);

    rerender({ liveSessionIds: new Set<string>() });
    expect(result.current.sessionId).toBeNull();
    expect(result.current.stats).toBeNull();
    expect(result.current.isManualRefreshing).toBe(false);
    act(() => result.current.refresh());
    await act(async () => pendingRefresh.resolve(remoteStats("Closed A", "warming_up")));
    await act(async () => vi.advanceTimersByTimeAsync(9000));
    expect(mocks.invoke).toHaveBeenCalledTimes(2);

    rerender({ liveSessionIds: liveSessions });
    expect(result.current.stats).toBeNull();
    await act(async () => reopenedRefresh.resolve(remoteStats("Fresh A", "aggregate")));
    expect(result.current.stats?.system.os).toBe("Fresh A");
  });

  it("restores each session's cached snapshot while immediately fetching fresh stats", async () => {
    const initialA = remoteStats("Initial A", "aggregate");
    const initialB = remoteStats("Initial B", "aggregate");
    const refreshedA = remoteStats("Refreshed A", "aggregate");
    const refreshA = deferred<RemoteStats>();
    const refreshB = deferred<RemoteStats>();
    mocks.invoke
      .mockResolvedValueOnce(initialA)
      .mockResolvedValueOnce(initialB)
      .mockReturnValueOnce(refreshA.promise)
      .mockReturnValueOnce(refreshB.promise);

    const { result, rerender } = renderHook(
      ({ sessionId }) => useRemoteStats(sessionId, true, 60),
      { initialProps: { sessionId: "session-a" } },
    );
    await waitFor(() => expect(result.current.stats).toBe(initialA));
    rerender({ sessionId: "session-b" });
    await waitFor(() => expect(result.current.stats).toBe(initialB));

    rerender({ sessionId: "session-a" });
    expect(result.current.sessionId).toBe("session-a");
    expect(result.current.stats).toBe(initialA);
    expect(result.current.isManualRefreshing).toBe(false);
    expect(mocks.invoke).toHaveBeenNthCalledWith(3, "get_remote_stats", {
      sessionId: "session-a",
    });

    await act(async () => refreshA.resolve(refreshedA));
    expect(result.current.stats).toBe(refreshedA);

    rerender({ sessionId: "session-b" });
    expect(result.current.sessionId).toBe("session-b");
    expect(result.current.stats).toBe(initialB);
    expect(mocks.invoke).toHaveBeenNthCalledWith(4, "get_remote_stats", {
      sessionId: "session-b",
    });
    await act(async () => refreshB.resolve(remoteStats("Refreshed B", "aggregate")));
    expect(result.current.stats?.system.os).toBe("Refreshed B");
  });

  it("polls only the active session and hides cached stats while monitoring is disabled", async () => {
    vi.useFakeTimers();
    const statsA = remoteStats("Session A", "aggregate");
    mocks.invoke.mockImplementation((_command: string, args: { sessionId: string }) =>
      Promise.resolve(
        args.sessionId === "session-a" ? statsA : remoteStats("Session B", "aggregate"),
      ),
    );
    const { result, rerender } = renderHook(
      ({ sessionId, enabled }) => useRemoteStats(sessionId, enabled, 3),
      { initialProps: { sessionId: "session-a", enabled: true } },
    );
    await act(async () => Promise.resolve());
    rerender({ sessionId: "session-b", enabled: true });
    await act(async () => vi.advanceTimersByTimeAsync(9000));
    expect(mocks.invoke.mock.calls.map(([, args]) => args.sessionId)).toEqual([
      "session-a",
      "session-b",
      "session-b",
      "session-b",
      "session-b",
    ]);

    rerender({ sessionId: "session-a", enabled: false });
    expect(result.current.sessionId).toBeNull();
    expect(result.current.stats).toBeNull();
    await act(async () => vi.advanceTimersByTimeAsync(9000));
    expect(mocks.invoke).toHaveBeenCalledTimes(5);

    const refreshA = deferred<RemoteStats>();
    mocks.invoke.mockReturnValueOnce(refreshA.promise);
    rerender({ sessionId: "session-a", enabled: true });
    expect(result.current.stats).toBe(statsA);
    expect(mocks.invoke).toHaveBeenCalledTimes(6);
    await act(async () => refreshA.resolve(remoteStats("Refreshed A", "aggregate")));
  });

  it("hides session A stats immediately while session B is pending", async () => {
    const sessionB = deferred<RemoteStats>();
    mocks.invoke.mockImplementation((_command: string, args: { sessionId: string }) =>
      args.sessionId === "session-a"
        ? Promise.resolve(remoteStats("Ubuntu", "aggregate"))
        : sessionB.promise,
    );

    const { result, rerender } = renderHook(
      ({ sessionId }) => useRemoteStats(sessionId, true, 60),
      { initialProps: { sessionId: "session-a" } },
    );

    await waitFor(() => expect(result.current.stats?.system.os).toBe("Ubuntu"));

    rerender({ sessionId: "session-b" });

    expect(result.current.sessionId).toBe("session-b");
    expect(result.current.stats).toBeNull();
    expect(result.current.error).toBe(false);
  });

  it("keeps session B when session A resolves after it", async () => {
    const sessionA = deferred<RemoteStats>();
    const sessionB = deferred<RemoteStats>();
    mocks.invoke.mockImplementation((_command: string, args: { sessionId: string }) =>
      args.sessionId === "session-a" ? sessionA.promise : sessionB.promise,
    );

    const { result, rerender } = renderHook(
      ({ sessionId }) => useRemoteStats(sessionId, true, 60),
      { initialProps: { sessionId: "session-a" } },
    );
    rerender({ sessionId: "session-b" });

    await act(async () => sessionB.resolve(remoteStats("SwitchOS", "aggregate")));
    expect(result.current.sessionId).toBe("session-b");
    expect(result.current.stats?.system.os).toBe("SwitchOS");

    await act(async () => sessionA.resolve(remoteStats("Ubuntu", "aggregate")));
    expect(result.current.sessionId).toBe("session-b");
    expect(result.current.stats?.system.os).toBe("SwitchOS");
  });

  it("never exposes session A stats while session B requests fail", async () => {
    let sessionBCalls = 0;
    mocks.invoke.mockImplementation((_command: string, args: { sessionId: string }) => {
      if (args.sessionId === "session-a") {
        return Promise.resolve(remoteStats("Ubuntu", "aggregate"));
      }
      sessionBCalls += 1;
      return Promise.reject(new Error(`failure-${sessionBCalls}`));
    });

    const { result, rerender } = renderHook(
      ({ sessionId }) => useRemoteStats(sessionId, true, 60),
      { initialProps: { sessionId: "session-a" } },
    );
    await waitFor(() => expect(result.current.stats?.system.os).toBe("Ubuntu"));

    rerender({ sessionId: "session-b" });
    await waitFor(() => expect(sessionBCalls).toBe(1));
    expect(result.current.sessionId).toBe("session-b");
    expect(result.current.stats).toBeNull();

    for (let expectedCalls = 2; expectedCalls <= 3; expectedCalls += 1) {
      act(() => result.current.refresh());
      await waitFor(() => expect(sessionBCalls).toBe(expectedCalls));
      expect(result.current.sessionId).toBe("session-b");
      expect(result.current.stats).toBeNull();
    }
  });

  it("preserves current-session stats until the existing failure threshold is reached", async () => {
    let calls = 0;
    mocks.invoke.mockImplementation(() => {
      calls += 1;
      return calls === 1
        ? Promise.resolve(remoteStats("Ubuntu", "aggregate"))
        : Promise.reject(new Error(`failure-${calls}`));
    });

    const { result, rerender } = renderHook(
      ({ sessionId }) => useRemoteStats(sessionId, true, 60),
      { initialProps: { sessionId: "session-a" } },
    );
    await waitFor(() => expect(result.current.stats?.system.os).toBe("Ubuntu"));

    for (let failure = 1; failure <= 3; failure += 1) {
      act(() => result.current.refresh());
      await waitFor(() => expect(calls).toBe(failure + 1));
      await act(async () => Promise.resolve());
      expect(result.current.error).toBe(true);
      if (failure < 3) {
        expect(result.current.stats?.system.os).toBe("Ubuntu");
      } else {
        expect(result.current.stats).toBeNull();
      }
    }

    rerender({ sessionId: "session-b" });
    rerender({ sessionId: "session-a" });
    expect(result.current.stats).toBeNull();
  });

  it("rejects stale responses and warmup timers across a rapid A to B to A switch", async () => {
    vi.useFakeTimers();
    const firstA = deferred<RemoteStats>();
    const sessionB = deferred<RemoteStats>();
    const finalA = deferred<RemoteStats>();
    const requests = [firstA.promise, sessionB.promise, finalA.promise];
    mocks.invoke.mockImplementation(() => requests.shift());

    const { result, rerender } = renderHook(
      ({ sessionId }) => useRemoteStats(sessionId, true, 60),
      { initialProps: { sessionId: "session-a" } },
    );
    rerender({ sessionId: "session-b" });
    rerender({ sessionId: "session-a" });
    expect(mocks.invoke).toHaveBeenCalledTimes(3);

    await act(async () => finalA.resolve(remoteStats("Final A", "aggregate")));
    await act(async () => firstA.resolve(remoteStats("Stale A", "warming_up")));
    await act(async () => sessionB.resolve(remoteStats("Stale B", "aggregate")));

    expect(result.current.sessionId).toBe("session-a");
    expect(result.current.stats?.system.os).toBe("Final A");

    await act(async () => vi.advanceTimersByTimeAsync(1000));
    expect(mocks.invoke).toHaveBeenCalledTimes(3);
  });

  it("does not let an old manual refresh completion clear the new session state", async () => {
    const oldManualRefresh = deferred<RemoteStats>();
    const returningRefresh = deferred<RemoteStats>();
    let sessionACalls = 0;
    mocks.invoke.mockImplementation((_command: string, args: { sessionId: string }) => {
      if (args.sessionId === "session-a") {
        sessionACalls += 1;
        if (sessionACalls === 1) {
          return Promise.resolve(remoteStats("Initial A", "aggregate"));
        }
        return sessionACalls === 2 ? oldManualRefresh.promise : returningRefresh.promise;
      }
      return Promise.resolve(remoteStats("Session B", "aggregate"));
    });

    const { result, rerender } = renderHook(
      ({ sessionId }) => useRemoteStats(sessionId, true, 60),
      { initialProps: { sessionId: "session-a" } },
    );
    await waitFor(() => expect(result.current.stats?.system.os).toBe("Initial A"));

    act(() => result.current.refresh());
    expect(result.current.isManualRefreshing).toBe(true);
    rerender({ sessionId: "session-b" });
    await waitFor(() => expect(result.current.stats?.system.os).toBe("Session B"));
    expect(result.current.isManualRefreshing).toBe(false);

    await act(async () => oldManualRefresh.resolve(remoteStats("Stale manual A", "aggregate")));
    expect(result.current.sessionId).toBe("session-b");
    expect(result.current.stats?.system.os).toBe("Session B");
    expect(result.current.isManualRefreshing).toBe(false);

    rerender({ sessionId: "session-a" });
    expect(result.current.stats?.system.os).toBe("Initial A");
    expect(result.current.isManualRefreshing).toBe(false);
    await act(async () => returningRefresh.resolve(remoteStats("Fresh A", "aggregate")));
    expect(result.current.stats?.system.os).toBe("Fresh A");
  });
});

function remoteStats(os: string, usageSource: RemoteStats["cpu"]["usage_source"]): RemoteStats {
  return {
    system: { hostname: os.toLowerCase(), uptime_sec: 100, os, arch: "x86_64" },
    load: { load1: 0.1, load5: 0.2, load15: 0.3 },
    cpu: {
      model: "CPU",
      cores: 8,
      usage: 10,
      per_core: [],
      sample_window_ms: 1000,
      usage_source: usageSource,
    },
    memory: { used: 1024, available: 1024, cached: 0 },
    networks: [],
    network_summary: { rx_bytes_per_sec: 0, tx_bytes_per_sec: 0 },
    disks: [],
  };
}
