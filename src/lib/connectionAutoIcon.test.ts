import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SavedConnection } from "@/types/global";
import { updateConnectionAutoIconAfterSessionStart } from "./connectionAutoIcon";

const mocks = vi.hoisted(() => ({
  getRemoteStats: vi.fn(),
  invoke: vi.fn(),
}));

vi.mock("./invoke", () => ({ invoke: mocks.invoke }));
vi.mock("./remoteStats", () => ({ getRemoteStats: mocks.getRemoteStats }));

describe("connection auto icon monitoring eligibility", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("skips remote stats detection for a Network Device connection", async () => {
    const connection: SavedConnection = {
      id: "conn-network",
      name: "Switch",
      type: "ssh",
      ssh_profile: "network_device",
      icon_auto_detect: true,
    };
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "get_saved_connections") return Promise.resolve([connection]);
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });

    await updateConnectionAutoIconAfterSessionStart({
      connectionId: connection.id,
      sessionId: "session-network",
      remoteStatsEnabled: true,
    });

    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith("get_saved_connections");
    expect(mocks.getRemoteStats).not.toHaveBeenCalled();
  });

  it("short-circuits before loading a connection when remote stats are disabled", async () => {
    await updateConnectionAutoIconAfterSessionStart({
      connectionId: "conn-network",
      sessionId: "session-network",
      remoteStatsEnabled: false,
    });

    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.getRemoteStats).not.toHaveBeenCalled();
  });
});
