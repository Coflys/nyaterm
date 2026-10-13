import { inferConnectionIconKeyFromRemoteSystem } from "@/components/icons";
import type { SavedConnection } from "@/types/global";
import { invoke } from "./invoke";
import { logger } from "./logger";
import { getRemoteStats } from "./remoteStats";

type AutoIconSessionStartOptions = {
  connectionId?: string | null;
  sessionId: string;
  remoteStatsEnabled: boolean;
};

const pendingDetections = new Set<string>();

function isConnectionIconAutoDetectEnabled(connection: SavedConnection): boolean {
  return connection.icon_auto_detect ?? !connection.icon;
}

async function loadConnection(connectionId: string): Promise<SavedConnection | null> {
  const connections = await invoke<SavedConnection[]>("get_saved_connections");
  return connections.find((connection) => connection.id === connectionId) ?? null;
}

export async function updateConnectionAutoIconAfterSessionStart({
  connectionId,
  sessionId,
  remoteStatsEnabled,
}: AutoIconSessionStartOptions): Promise<void> {
  if (!connectionId || !remoteStatsEnabled) return;

  const pendingKey = `${connectionId}:${sessionId}`;
  if (pendingDetections.has(pendingKey)) return;

  pendingDetections.add(pendingKey);
  try {
    const initialConnection = await loadConnection(connectionId);
    if (
      !initialConnection ||
      initialConnection.type !== "ssh" ||
      initialConnection.ssh_profile === "network_device" ||
      !isConnectionIconAutoDetectEnabled(initialConnection)
    ) {
      return;
    }

    const stats = await getRemoteStats(sessionId);
    const iconKey = inferConnectionIconKeyFromRemoteSystem(stats.system);
    if (!iconKey) return;

    const currentConnection = await loadConnection(connectionId);
    if (
      !currentConnection ||
      currentConnection.type !== "ssh" ||
      currentConnection.ssh_profile === "network_device" ||
      !isConnectionIconAutoDetectEnabled(currentConnection) ||
      currentConnection.icon === iconKey
    ) {
      return;
    }

    await invoke("update_connection_icon", {
      connectionId,
      icon: iconKey,
      iconAutoDetect: true,
    });
  } catch (error) {
    logger.error({
      domain: "ui.error",
      event: "connection.auto_icon_update_failed",
      message: "Failed to update auto-detected connection icon",
      ids: { connection_id: connectionId, session_id: sessionId },
      error,
    });
  } finally {
    pendingDetections.delete(pendingKey);
  }
}
