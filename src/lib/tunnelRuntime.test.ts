import { describe, expect, it } from "vitest";
import type { TunnelRuntimeState } from "@/types/global";
import { mergeTunnelRuntimeStates } from "./tunnelRuntime";

describe("tunnel runtime ordering", () => {
  it("keeps a newer stop when an old running event or snapshot arrives", () => {
    const stopped: TunnelRuntimeState = {
      tunnelId: "t",
      status: "stopped",
      retryAttempt: 0,
      revision: 3,
    };
    const stale: TunnelRuntimeState = {
      ...stopped,
      status: "running",
      revision: 2,
    };
    expect(mergeTunnelRuntimeStates({ t: stopped }, [stale]).t).toBe(stopped);
  });

  it("accepts new events without dropping other tunnel states", () => {
    const running: TunnelRuntimeState = {
      tunnelId: "t",
      status: "running",
      retryAttempt: 0,
      revision: 5,
    };
    const other: TunnelRuntimeState = { ...running, tunnelId: "other" };
    expect(mergeTunnelRuntimeStates({ other }, [running])).toEqual({
      t: running,
      other,
    });
  });

  it("does not replace an active event with an absent-runtime snapshot", () => {
    const reconnecting: TunnelRuntimeState = {
      tunnelId: "t",
      status: "reconnecting",
      retryAttempt: 2,
      revision: 4,
    };
    expect(
      mergeTunnelRuntimeStates({ t: reconnecting }, [
        { ...reconnecting, status: "disconnected", revision: 0 },
      ]).t,
    ).toBe(reconnecting);
  });
});
