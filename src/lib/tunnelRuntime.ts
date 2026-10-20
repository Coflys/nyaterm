import type { TunnelRuntimeState } from "@/types/global";

/** Snapshots and events can arrive out of order across IPC requests. */
export function mergeTunnelRuntimeStates(
  current: Record<string, TunnelRuntimeState>,
  incoming: TunnelRuntimeState[],
): Record<string, TunnelRuntimeState> {
  const next = { ...current };
  for (const state of incoming) {
    const previous = next[state.tunnelId];
    const revision = state.revision ?? state.updatedAt ?? 0;
    const previousRevision = previous?.revision ?? previous?.updatedAt ?? 0;
    if (!previous || revision >= previousRevision) next[state.tunnelId] = state;
  }
  return next;
}
