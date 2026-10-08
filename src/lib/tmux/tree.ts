/** Map backend tmux layout trees onto the workspace `PaneNode` model. */

import {
  collectSessionPanes,
  createWorkspaceId,
} from "@/lib/workspaceTabs";
import type { PaneNode, SessionPane, Tab } from "@/types/global";
import type { TmuxLayoutNode, TmuxSessionState } from "./types";

/** The control session owning this tab's tmux panes, if the tab is in tmux mode. */
export function findTmuxControlSessionId(tab: Tab): string | null {
  for (const pane of collectSessionPanes(tab.root)) {
    if (pane.tmux) return pane.tmux.controlSessionId;
  }
  return null;
}

/**
 * Locate the tab that owns `controlSessionId` — either the session is still a
 * plain leaf (control mode was just detected) or one of its virtual panes.
 */
export function findTmuxTab(tabs: Tab[], controlSessionId: string): Tab | null {
  for (const tab of tabs) {
    for (const pane of collectSessionPanes(tab.root)) {
      if (
        pane.sessionId === controlSessionId ||
        pane.tmux?.controlSessionId === controlSessionId
      ) {
        return tab;
      }
    }
  }
  return null;
}

/**
 * True when every leaf under `node` belongs to `controlSessionId` — either
 * the plain SSH leaf the user ran `tmux -CC` in, or the pane subtree the
 * projection previously replaced it with. Applied top-down, so the first
 * match is the minimal subtree anchoring the projection; native splits
 * holding other panes stay outside it.
 */
function isTmuxAnchor(node: PaneNode, controlSessionId: string): boolean {
  const panes = collectSessionPanes(node);
  return (
    panes.length > 0 &&
    panes.every(
      (pane) =>
        pane.sessionId === controlSessionId ||
        pane.tmux?.controlSessionId === controlSessionId,
    )
  );
}

/**
 * Swap the subtree anchoring `controlSessionId` inside `root` for
 * `replacement`, leaving every pane outside the projection untouched.
 * Returns `root` unchanged when no anchor exists.
 */
export function replaceTmuxAnchor(
  root: PaneNode,
  controlSessionId: string,
  replacement: PaneNode,
): PaneNode {
  if (isTmuxAnchor(root, controlSessionId)) return replacement;
  if (root.kind !== "split") return root;
  const first = replaceTmuxAnchor(root.first, controlSessionId, replacement);
  const second = replaceTmuxAnchor(root.second, controlSessionId, replacement);
  // Keep node identity when nothing changed so React skips untouched
  // subtrees and persistence comparisons stay cheap.
  if (first === root.first && second === root.second) return root;
  return { ...root, first, second };
}

/**
 * Build the workspace pane tree for a tmux window. Existing leaf ids are
 * preserved per tmux pane id so React keys and xterm instances stay stable
 * across layout changes.
 */
export function buildTmuxPaneTree(
  state: TmuxSessionState,
  existing: Tab,
): { root: PaneNode; activePaneId: string } | null {
  if (!state.tree) return null;

  const paneIdByTmuxPane = new Map<string, string>();
  for (const pane of collectSessionPanes(existing.root)) {
    if (pane.tmux?.controlSessionId === state.controlSessionId) {
      paneIdByTmuxPane.set(pane.tmux.paneId, pane.id);
    }
  }

  const basePane = collectSessionPanes(existing.root).find(
    (pane) =>
      pane.sessionId === state.controlSessionId ||
      pane.tmux?.controlSessionId === state.controlSessionId,
  );

  const convert = (node: TmuxLayoutNode): PaneNode => {
    if (node.kind === "pane") {
      const pane: SessionPane = {
        id: paneIdByTmuxPane.get(node.paneId) ?? createWorkspaceId("pane"),
        kind: "leaf",
        paneKind: "terminal",
        sessionId: node.sessionId,
        // Keep the connection's session name so the tab title stays the
        // connection label instead of the pane's current command.
        name: basePane?.name ?? node.name,
        type: "SSH",
        connectionId: basePane?.connectionId,
        // Carry the originating leaf's restore config so detach can rebuild
        // it and persistence serializes a restorable SSH leaf.
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

  const root = convert(state.tree);
  const paneIds = new Set(collectSessionPanes(root).map((pane) => pane.id));
  const activePaneId =
    (paneIds.has(existing.activePaneId) ? existing.activePaneId : null) ??
    (state.activePaneSessionId
      ? collectSessionPanes(root).find(
          (pane) => pane.sessionId === state.activePaneSessionId,
        )?.id
      : undefined) ??
    collectSessionPanes(root)[0]?.id ??
    existing.activePaneId;

  return { root, activePaneId };
}

/** Leaf shown after the tmux control client detached: a plain shell pane. */
export function tmuxFallbackLeaf(
  state: TmuxSessionState,
  existing: Tab,
): SessionPane {
  const panes = collectSessionPanes(existing.root);
  // Prefer a pane of this control session so the restored leaf inherits the
  // originating connection's name/config; reusing its id keeps the leaf
  // mounted when the anchor was already a plain shell pane.
  const basePane =
    panes.find(
      (pane) =>
        pane.sessionId === state.controlSessionId ||
        pane.tmux?.controlSessionId === state.controlSessionId,
    ) ?? panes[0];
  const pane: SessionPane = {
    id: basePane?.id ?? createWorkspaceId("pane"),
    kind: "leaf",
    paneKind: "terminal",
    sessionId: state.controlSessionId,
    name: basePane?.name ?? "shell",
    type: "SSH",
    connectionId: basePane?.connectionId,
    temporaryConfig: basePane?.temporaryConfig,
    sshRuntimeMode: basePane?.sshRuntimeMode,
  };
  return pane;
}
