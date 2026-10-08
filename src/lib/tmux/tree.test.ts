import { describe, expect, it } from "vitest";
import { collapseTmuxPanes, serializeTabsForPersistence } from "@/lib/workspaceTabs";
import { createSessionPane, createWorkspaceId, collectSessionPanes } from "@/lib/workspaceTabs";
import type { PaneNode, SessionPane, SplitPane, Tab } from "@/types/global";
import {
  buildTmuxPaneTree,
  replaceTmuxAnchor,
  tmuxFallbackLeaf,
} from "./tree";
import type { TmuxLayoutNode, TmuxSessionState } from "./types";

const CTRL = "session-ctrl";

function leaf(id: string, sessionId: string, name = "ssh"): SessionPane {
  return createSessionPane(name, "SSH", "conn-1", {
    id,
    sessionId,
    sshRuntimeMode: "standard",
  });
}

function localLeaf(id: string): SessionPane {
  return createSessionPane("local", "Local", undefined, { id });
}

function split(first: PaneNode, second: PaneNode): SplitPane {
  return {
    id: createWorkspaceId("pane"),
    kind: "split",
    direction: "vertical",
    ratio: 0.5,
    first,
    second,
  };
}

function tabWith(root: PaneNode, activePaneId: string): Tab {
  return {
    id: createWorkspaceId("tab"),
    persistOrder: 0,
    activePaneId,
    root,
  };
}

function tmuxTree(...paneIds: string[]): TmuxLayoutNode {
  const nodes = paneIds.map(
    (paneId): TmuxLayoutNode => ({
      kind: "pane",
      paneId,
      sessionId: `${CTRL}::${paneId}`,
      name: "ssh",
    }),
  );
  return nodes.slice(1).reduce<TmuxLayoutNode>(
    (acc, node) => ({
      kind: "split",
      direction: "vertical",
      ratio: 0.5,
      first: acc,
      second: node,
    }),
    nodes[0],
  );
}

function state(tree: TmuxLayoutNode, exited = false): TmuxSessionState {
  return {
    controlSessionId: CTRL,
    exited,
    windows: [],
    activePaneSessionId: `${CTRL}::%0`,
    tree,
  };
}

describe("replaceTmuxAnchor", () => {
  it("replaces only the SSH leaf entering tmux, preserving native splits", () => {
    const local = localLeaf("local-1");
    const ssh = leaf("ssh-1", CTRL);
    const tab = tabWith(split(local, ssh), ssh.id);

    const built = buildTmuxPaneTree(state(tmuxTree("%0", "%1")), tab);
    expect(built).not.toBeNull();
    const root = replaceTmuxAnchor(tab.root, CTRL, built!.root);

    expect(root.kind).toBe("split");
    const [first, second] = [root as SplitPane].map((n) => [n.first, n.second])[0];
    expect(first).toBe(local);
    const tmuxPanes = collectSessionPanes(second);
    expect(tmuxPanes).toHaveLength(2);
    expect(tmuxPanes.every((p) => p.tmux?.controlSessionId === CTRL)).toBe(true);
  });

  it("updates the projected subtree in place on layout changes", () => {
    const local = localLeaf("local-1");
    const ssh = leaf("ssh-1", CTRL);
    const tab = tabWith(split(local, ssh), ssh.id);

    const first = buildTmuxPaneTree(state(tmuxTree("%0", "%1")), tab)!;
    const root1 = replaceTmuxAnchor(tab.root, CTRL, first.root);
    const tab1 = tabWith(root1, first.activePaneId);
    const paneIdBefore = collectSessionPanes(root1).find(
      (p) => p.tmux?.paneId === "%0",
    )!.id;

    // Remote side splits %1 into %1 + %2.
    const next = buildTmuxPaneTree(state(tmuxTree("%0", "%1", "%2")), tab1)!;
    const root2 = replaceTmuxAnchor(tab1.root, CTRL, next.root);

    const [sib, tmuxSubtree] = [root2 as SplitPane].map((n) => [
      n.first,
      n.second,
    ])[0];
    expect(sib).toBe(local);
    const panes = collectSessionPanes(tmuxSubtree);
    expect(panes.map((p) => p.tmux?.paneId).sort()).toEqual(["%0", "%1", "%2"]);
    // Leaf ids are stable across layout changes so xterm instances survive.
    expect(panes.find((p) => p.tmux?.paneId === "%0")!.id).toBe(paneIdBefore);
  });

  it("collapses the projection back to a shell leaf on exit", () => {
    const local = localLeaf("local-1");
    const ssh = leaf("ssh-1", CTRL, "prod-box");
    const tab = tabWith(split(local, ssh), ssh.id);

    const built = buildTmuxPaneTree(state(tmuxTree("%0", "%1")), tab)!;
    const tmuxTab = tabWith(
      replaceTmuxAnchor(tab.root, CTRL, built.root),
      built.activePaneId,
    );

    const fallback = tmuxFallbackLeaf(state(tmuxTree("%0"), true), tmuxTab);
    const root = replaceTmuxAnchor(tmuxTab.root, CTRL, fallback);

    expect(root.kind).toBe("split");
    const [sib, shell] = [root as SplitPane].map((n) => [n.first, n.second])[0];
    expect(sib).toBe(local);
    expect(shell.kind).toBe("leaf");
    const shellLeaf = shell as SessionPane;
    expect(shellLeaf.sessionId).toBe(CTRL);
    expect(shellLeaf.tmux).toBeUndefined();
    expect(shellLeaf.name).toBe("prod-box");
    expect(shellLeaf.connectionId).toBe("conn-1");
    expect(shellLeaf.sshRuntimeMode).toBe("standard");
  });

  it("replaces the whole root when the tab holds no foreign panes", () => {
    const ssh = leaf("ssh-1", CTRL);
    const tab = tabWith(ssh, ssh.id);
    const built = buildTmuxPaneTree(state(tmuxTree("%0", "%1")), tab)!;
    const root = replaceTmuxAnchor(tab.root, CTRL, built.root);
    expect(root).toBe(built.root);
  });

  it("leaves the tree untouched when no anchor exists", () => {
    const local = localLeaf("local-1");
    const other = leaf("ssh-1", "session-other");
    const tab = tabWith(split(local, other), other.id);
    const built = buildTmuxPaneTree(
      state(tmuxTree("%0")),
      tabWith(leaf("x", CTRL), "x"),
    )!;
    const root = replaceTmuxAnchor(tab.root, CTRL, built.root);
    expect(root).toBe(tab.root);
  });
});

describe("collapseTmuxPanes", () => {
  it("collapses only the tmux subtree inside a mixed tab", () => {
    const local = localLeaf("local-1");
    const ssh = leaf("ssh-1", CTRL);
    const tab = tabWith(split(local, ssh), ssh.id);
    const built = buildTmuxPaneTree(state(tmuxTree("%0", "%1")), tab)!;
    const root = replaceTmuxAnchor(tab.root, CTRL, built.root);

    const collapsed = collapseTmuxPanes(root);
    expect(collapsed.kind).toBe("split");
    const [sib, plain] = [collapsed as SplitPane].map((n) => [
      n.first,
      n.second,
    ])[0];
    expect(sib).toBe(local);
    expect(plain.kind).toBe("leaf");
    expect((plain as SessionPane).type).toBe("SSH");
    expect((plain as SessionPane).connectionId).toBe("conn-1");
  });
});

describe("serializeTabsForPersistence", () => {
  it("persists a mixed tab with the tmux subtree as one restorable leaf", () => {
    const local = localLeaf("local-1");
    const ssh = leaf("ssh-1", CTRL);
    const tab = tabWith(split(local, ssh), ssh.id);
    const built = buildTmuxPaneTree(state(tmuxTree("%0", "%1")), tab)!;
    const mixed = tabWith(
      replaceTmuxAnchor(tab.root, CTRL, built.root),
      built.activePaneId,
    );

    const [persisted] = serializeTabsForPersistence([mixed]);
    const root = persisted.root;
    if (!root || root.kind !== "split") {
      throw new Error("expected the mixed tab to persist its split root");
    }
    expect(root.second.kind).toBe("leaf");
    if (root.second.kind === "leaf") {
      expect(root.second.session_type).toBe("SSH");
      expect(root.second.connection_id).toBe("conn-1");
    }
  });
});
