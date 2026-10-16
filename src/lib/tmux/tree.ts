/** Map backend tmux layout trees onto the workspace `PaneNode` model. */

import {
  collectSessionPanes,
  createWorkspaceId,
  ensureActivePane,
  getActivePane,
  findPaneById,
  getNextPersistOrder,
  insertTabAfter,
} from "@/lib/workspaceTabs";
import type { PaneNode, SessionPane, Tab } from "@/types/global";
import type { TmuxLayoutNode, TmuxSessionState, TmuxWindowInfo } from "./types";

/**
 * The tab holding the SSH leaf whose channel entered `tmux -CC` — the leaf
 * keeps `sessionId === controlSessionId` and is never replaced: it stays a
 * normal shell pane (frozen while the control session owns the channel, live
 * again after detach). Returns null once that session is gone.
 */
export function findTmuxHostTab(
  tabs: Tab[],
  controlSessionId: string,
): Tab | null {
  for (const tab of tabs) {
    for (const pane of collectSessionPanes(tab.root)) {
      if (
        pane.paneKind === "terminal" &&
        pane.type === "SSH" &&
        pane.sessionId === controlSessionId
      )
        return tab;
    }
  }
  return null;
}

/** Tabs rendering windows of `controlSessionId`, in tab-bar order. */
export function tmuxWindowTabs(tabs: Tab[], controlSessionId: string): Tab[] {
  return tabs
    .filter((tab) => tab.tmux?.controlSessionId === controlSessionId)
    .sort((a, b) => a.tmux!.windowIndex - b.tmux!.windowIndex);
}

/** The workspace pane id currently rendering each tmux pane — leaf ids are
 * preserved across layout changes so React keys and xterm instances stay
 * stable, and a pane `join-pane`-d into another window keeps its terminal.
 */
export function tmuxPaneLeafIds(tabs: Tab[], controlSessionId: string) {
  const ids = new Map<string, string>();
  for (const tab of tmuxWindowTabs(tabs, controlSessionId)) {
    for (const pane of collectSessionPanes(tab.root)) {
      if (pane.tmux?.controlSessionId === controlSessionId) {
        ids.set(pane.tmux.paneId, pane.id);
      }
    }
  }
  return ids;
}

/**
 * Build the workspace pane tree for one tmux window. `basePane` — the host
 * SSH leaf — supplies connection identity and restore config; `leafIds`
 * (from {@link tmuxPaneLeafIds}) keeps leaf ids stable across updates.
 */
export function buildWindowPaneTree(
  state: TmuxSessionState,
  window: TmuxWindowInfo,
  leafIds: Map<string, string>,
  basePane: SessionPane | undefined,
): { root: PaneNode; activePaneId: string } | null {
  if (!window.tree) return null;

  const convert = (node: TmuxLayoutNode): PaneNode => {
    if (node.kind === "pane") {
      const pane: SessionPane = {
        id: leafIds.get(node.paneId) ?? createWorkspaceId("pane"),
        kind: "leaf",
        paneKind: "terminal",
        sessionId: node.sessionId,
        // Keep the connection's session name so the tab title falls back to
        // the connection label instead of the pane's current command.
        name: basePane?.name ?? node.name,
        type: "SSH",
        connectionId: basePane?.connectionId,
        // Carry the originating leaf's restore config so persistence could
        // serialize a restorable SSH leaf if the window tab ever survived.
        temporaryConfig: basePane?.temporaryConfig,
        sshRuntimeMode: basePane?.sshRuntimeMode,
        tmux: {
          controlSessionId: state.controlSessionId,
          paneId: node.paneId,
        },
      };
      return pane;
    }
    return {
      id: createWorkspaceId("pane"),
      kind: "split",
      direction: node.direction,
      // tmux-owned layout — the divider is fixed at 50/50 and not draggable.
      // Feeding tmux's real ratio back here closes a feedback loop: renderer
      // reports drive resize-pane, whose layout echo would shift the
      // containers, which report new sizes — a ratchet that runs away on any
      // px-level asymmetry. Fixed ratios keep renderer sizes stable so the
      // backend sync converges instead.
      ratio: 0.5,
      first: convert(node.first),
      second: convert(node.second),
    };
  };

  const root = convert(window.tree);
  const panes = collectSessionPanes(root);
  const activePaneId =
    (window.active && state.activePaneSessionId
      ? panes.find((pane) => pane.sessionId === state.activePaneSessionId)?.id
      : undefined) ?? panes[0]?.id;
  if (!activePaneId) return null;

  return { root, activePaneId };
}

export interface WindowTabReconcile {
  tabs: Tab[];
}

/**
 * Reconcile the native tabs of one control session with the backend's window
 * list — the pure half of `applyTmuxState`. Tabs whose window vanished
 * remotely are dropped, windows without a tab get one inserted contiguously
 * after the session's existing tabs (host tab first, then window tabs in
 * order), and surviving tabs refresh their pane tree and window name in
 * place. Activation and remote side effects stay with the caller.
 */
export function reconcileTmuxWindowTabs(
  tabs: Tab[],
  state: TmuxSessionState,
  hostTab: Tab,
  options?: {
    /**
     * Adopt the remote active pane in the active window's tab. The caller
     * sets this only when `state.activePaneSessionId` actually changed —
     * remote-initiated `select-pane` then follows through to the UI while a
     * local click is not reverted by a stale echo.
     */
    followRemoteActivePane?: boolean;
  },
): WindowTabReconcile {
  const controlSessionId = state.controlSessionId;
  const basePane = collectSessionPanes(hostTab.root).find(
    (pane) => pane.sessionId === controlSessionId,
  );
  const leafIds = tmuxPaneLeafIds(tabs, controlSessionId);
  const liveWindowIds = new Set(state.windows.map((window) => window.id));

  // Tabs for windows that no longer exist remotely are removed here; tmux
  // itself owns the close, so the tab simply disappears.
  let nextTabs = tabs.filter(
    (tab) =>
      !(
        tab.tmux?.controlSessionId === controlSessionId &&
        !liveWindowIds.has(tab.tmux.windowId)
      ),
  );

  // New window tabs insert contiguously after the session's existing tabs.
  const existingWindowTabs = tmuxWindowTabs(nextTabs, controlSessionId);
  let insertAfter =
    existingWindowTabs.length > 0
      ? existingWindowTabs[existingWindowTabs.length - 1].id
      : hostTab.id;
  let nextPersistOrder = getNextPersistOrder(nextTabs);

  const byWindow = new Map(
    tmuxWindowTabs(nextTabs, controlSessionId).map((tab) => [
      tab.tmux!.windowId,
      tab,
    ]),
  );

  for (const window of state.windows) {
    const existing = byWindow.get(window.id);
    const built = buildWindowPaneTree(state, window, leafIds, basePane);
    if (!built) {
      // Layout for this window has not arrived yet — keep an existing tab's
      // name and active flag current so the strip never goes stale.
      if (
        existing &&
        (existing.tmux!.windowName !== window.name ||
          existing.tmux!.windowIndex !== window.index ||
          existing.tmux!.active !== window.active)
      ) {
        const id = existing.id;
        const tmux = {
          ...existing.tmux!,
          windowName: window.name,
          windowIndex: window.index,
          active: window.active,
        };
        nextTabs = nextTabs.map((tab) => (tab.id === id ? { ...tab, tmux } : tab));
      }
      continue;
    }
    if (existing) {
      const activePaneId =
        window.active && options?.followRemoteActivePane
          ? built.activePaneId
          : findPaneById(built.root, existing.activePaneId)
            ? existing.activePaneId
            : built.activePaneId;
      nextTabs = nextTabs.map((tab) =>
        tab.id === existing.id
          ? {
              ...tab,
              root: built.root,
              activePaneId,
              tmux: {
                ...tab.tmux!,
                windowName: window.name,
                windowIndex: window.index,
                active: window.active,
              },
            }
          : tab,
      );
    } else {
      const newTab: Tab = {
        id: createWorkspaceId("tab"),
        persistOrder: nextPersistOrder++,
        activePaneId: built.activePaneId,
        root: built.root,
        tmux: {
          controlSessionId,
          windowId: window.id,
          windowName: window.name,
          windowIndex: window.index,
          active: window.active,
        },
      };
      nextTabs = insertTabAfter(nextTabs, insertAfter, newTab);
      insertAfter = newTab.id;
    }
  }
  const ordered = tmuxWindowTabs(nextTabs, controlSessionId);
  let index = 0;
  nextTabs = nextTabs.map((tab) =>
    tab.tmux?.controlSessionId === controlSessionId ? ordered[index++] : tab,
  );
  return { tabs: nextTabs };
}

/** Resolve actions against the visible pane of the selected host leaf only. */
export function resolveWorkspaceActivePane(tabs: Tab[], tab: Tab): SessionPane | null {
  const pane = getActivePane(tab);
  if (!pane || pane.paneKind !== "terminal" || pane.tmux) return pane;
  const window = tmuxWindowTabs(tabs, pane.sessionId).find((tab) => tab.tmux?.active);
  return window ? getActivePane(window) : pane;
}

/** Select a child pane and its host leaf atomically, keeping a native top-level tab. */
export function activateWorkspacePane(tabs: Tab[], tabId: string, paneId: string) {
  const target = tabs.find((tab) => tab.id === tabId);
  if (!target || !findPaneById(target.root, paneId)) return null;
  const host = target.tmux ? findTmuxHostTab(tabs, target.tmux.controlSessionId) : target;
  if (!host) return null;
  const hostPane = target.tmux
    ? collectSessionPanes(host.root).find(
        (pane) => pane.paneKind === "terminal" && pane.sessionId === target.tmux!.controlSessionId,
      )
    : null;
  if (target.tmux && !hostPane) return null;
  return {
    activeTabId: host.id,
    tabs: tabs.map((tab) => {
      if (tab.id === target.id) return ensureActivePane({ ...tab, activePaneId: paneId });
      if (tab.id === host.id && hostPane) return { ...tab, activePaneId: hostPane.id };
      return tab;
    }),
  };
}
