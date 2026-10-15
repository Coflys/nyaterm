import { describe, expect, it } from "vitest";
import {
  collectSessionPanes,
  collapseTmuxPanes,
  createSessionPane,
  createWorkspaceId,
  getTabDisplayName,
  serializeTabsForPersistence,
} from "@/lib/workspaceTabs";
import type { PaneNode, SessionPane, SplitPane, Tab } from "@/types/global";
import {
  buildWindowPaneTree,
  activateWorkspacePane,
  resolveWorkspaceActivePane,
  findTmuxHostTab,
  reconcileTmuxWindowTabs,
  tmuxPaneLeafIds,
  tmuxWindowTabs,
} from "./tree";
import type { TmuxLayoutNode, TmuxSessionState, TmuxWindowInfo } from "./types";

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

function hostTab(panes = 1): Tab {
  const ssh = leaf("ssh-1", CTRL, "prod-box");
  const root = panes > 1 ? split(localLeaf("local-1"), ssh) : (ssh as PaneNode);
  return {
    id: "host-tab",
    persistOrder: 0,
    activePaneId: ssh.id,
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

function window(
  id: string,
  name: string,
  tree: TmuxLayoutNode | null,
  active = false,
): TmuxWindowInfo {
  return { id, name, index: Number(id.slice(1)), active, tree };
}

function state(
  windows: TmuxWindowInfo[],
  activeWindowId = "@1",
  exited = false,
): TmuxSessionState {
  return {
    controlSessionId: CTRL,
    exited,
    windows,
    activeWindowId,
    activePaneSessionId: `${CTRL}::%0`,
  };
}

describe("buildWindowPaneTree", () => {
  it("builds leaves marked with the tmux pane identity", () => {
    const host = hostTab();
    const basePane = collectSessionPanes(host.root).find(
      (p) => p.sessionId === CTRL,
    );
    const win = window("@1", "bash", tmuxTree("%0", "%1"), true);
    const built = buildWindowPaneTree(state([win]), win, new Map(), basePane)!;

    const panes = collectSessionPanes(built.root);
    expect(panes).toHaveLength(2);
    expect(
      panes.every(
        (p) =>
          p.tmux?.controlSessionId === CTRL &&
          p.sessionId.startsWith(`${CTRL}::`),
      ),
    ).toBe(true);
    // Connection identity comes from the host leaf so the tab titles and
    // restores like the originating SSH session.
    expect(panes[0].name).toBe("prod-box");
    expect(panes[0].connectionId).toBe("conn-1");
  });

  it("reuses leaf ids from the map so xterm instances survive relayout", () => {
    const win = window("@1", "bash", tmuxTree("%0", "%1"));
    const leafIds = new Map([["%1", "kept-leaf"]]);
    const built = buildWindowPaneTree(state([win]), win, leafIds, undefined)!;
    const panes = collectSessionPanes(built.root);
    expect(panes.find((p) => p.tmux?.paneId === "%1")!.id).toBe("kept-leaf");
  });

  it("selects the tmux active pane only on the active window", () => {
    const active = window("@1", "a", tmuxTree("%0", "%1"), true);
    const inactive = window("@2", "b", tmuxTree("%0", "%1"));
    const s = state([active, inactive]);

    const activeBuilt = buildWindowPaneTree(s, active, new Map(), undefined)!;
    const activePanes = collectSessionPanes(activeBuilt.root);
    // activePaneSessionId points at %0's virtual session.
    expect(activeBuilt.activePaneId).toBe(activePanes[0].id);

    const inactiveBuilt = buildWindowPaneTree(
      s,
      inactive,
      new Map(),
      undefined,
    )!;
    // Inactive windows just take their first pane.
    expect(inactiveBuilt.activePaneId).toBe(
      collectSessionPanes(inactiveBuilt.root)[0].id,
    );
  });

  it("returns null while the window layout is unknown", () => {
    const win = window("@3", "pending", null);
    expect(
      buildWindowPaneTree(state([win]), win, new Map(), undefined),
    ).toBeNull();
  });
});

describe("reconcileTmuxWindowTabs", () => {
  it("creates one tab per window after the host tab, in window order", () => {
    const other = {
      id: "other-tab",
      persistOrder: 1,
      activePaneId: "x",
      root: localLeaf("x") as PaneNode,
    };
    const tabs = [other, hostTab()];
    const s = state([
      window("@1", "bash", tmuxTree("%0"), true),
      window("@2", "logs", tmuxTree("%4", "%5")),
    ]);

    const { tabs: next } = reconcileTmuxWindowTabs(tabs, s, hostTab());
    const windowTabs = tmuxWindowTabs(next, CTRL);
    expect(next).toHaveLength(4);
    expect(windowTabs).toHaveLength(2);
    // Contiguous, right after the host tab.
    expect(next.map((t) => t.id)).toEqual([
      "other-tab",
      "host-tab",
      windowTabs[0].id,
      windowTabs[1].id,
    ]);
    expect(windowTabs.map((t) => t.tmux!.windowId)).toEqual(["@1", "@2"]);
    expect(windowTabs.map((t) => t.tmux!.windowName)).toEqual(["bash", "logs"]);
    expect(windowTabs.map((t) => t.tmux!.active)).toEqual([true, false]);
    expect(collectSessionPanes(windowTabs[1].root)).toHaveLength(2);
    expect(next.find((t) => t.id === "host-tab")).toBe(tabs[1]);
  });

  it("keeps the host tab's own split tree untouched through attach and teardown", () => {
    // Regression for the review's P1: an SSH pane nested inside local
    // splits must not lose its siblings when tmux attaches or detaches —
    // reconcile never rewrites the host or its root.
    const host = hostTab(2);
    const tabs = [host];
    const { tabs: attached } = reconcileTmuxWindowTabs(
      tabs,
      state([window("@1", "bash", tmuxTree("%0"), true)]),
      host,
    );
    const attachedHost = attached.find((tab) => tab.id === host.id)!;
    expect(attachedHost.root).toEqual(host.root);
    expect(attachedHost).toBe(host);

    const { tabs: after } = reconcileTmuxWindowTabs(
      attached,
      state([]),
      attachedHost,
    );
    expect(tmuxWindowTabs(after, CTRL)).toHaveLength(0);
    expect(after.find((tab) => tab.id === host.id)!.root).toEqual(host.root);
  });

  it("drops tabs whose window closed remotely", () => {
    const host = hostTab();
    const first = reconcileTmuxWindowTabs(
      [host],
      state([
        window("@1", "a", tmuxTree("%0"), true),
        window("@2", "b", tmuxTree("%1")),
      ]),
      host,
    ).tabs;
    expect(tmuxWindowTabs(first, CTRL)).toHaveLength(2);

    const next = reconcileTmuxWindowTabs(
      first,
      state([window("@1", "a", tmuxTree("%0"), true)]),
      host,
    ).tabs;
    expect(tmuxWindowTabs(next, CTRL).map((t) => t.tmux!.windowId)).toEqual([
      "@1",
    ]);
  });

  it("moves the active flag when the remote switches windows", () => {
    const host = hostTab();
    const tabs1 = reconcileTmuxWindowTabs(
      [host],
      state([
        window("@1", "a", tmuxTree("%0"), true),
        window("@2", "b", tmuxTree("%1")),
      ]),
      host,
    ).tabs;

    const tabs2 = reconcileTmuxWindowTabs(
      tabs1,
      state(
        [
          window("@1", "a", tmuxTree("%0")),
          window("@2", "b", tmuxTree("%1"), true),
        ],
        "@2",
      ),
      host,
    ).tabs;
    expect(tmuxWindowTabs(tabs2, CTRL).map((t) => t.tmux!.active)).toEqual([
      false,
      true,
    ]);
  });

  it("keeps pane leaf ids stable when a window's layout changes", () => {
    const host = hostTab();
    const tabs1 = reconcileTmuxWindowTabs(
      [host],
      state([window("@1", "a", tmuxTree("%0", "%1"), true)]),
      host,
    ).tabs;
    const before = tmuxPaneLeafIds(tabs1, CTRL);

    // %1 splits into %1 + %2.
    const tabs2 = reconcileTmuxWindowTabs(
      tabs1,
      state([window("@1", "a", tmuxTree("%0", "%1", "%2"), true)]),
      host,
    ).tabs;
    const panes = collectSessionPanes(tmuxWindowTabs(tabs2, CTRL)[0].root);
    expect(panes.find((p) => p.tmux?.paneId === "%0")!.id).toBe(
      before.get("%0"),
    );
    expect(panes.find((p) => p.tmux?.paneId === "%1")!.id).toBe(
      before.get("%1"),
    );
    expect(panes).toHaveLength(3);
  });

  it("applies renames even while a window's layout is pending", () => {
    const host = hostTab();
    const tabs1 = reconcileTmuxWindowTabs(
      [host],
      state([window("@1", "old", tmuxTree("%0"), true)]),
      host,
    ).tabs;

    const tabs2 = reconcileTmuxWindowTabs(
      tabs1,
      state([window("@1", "renamed", null, true)]),
      host,
    ).tabs;
    const tab = tmuxWindowTabs(tabs2, CTRL)[0];
    expect(tab.tmux!.windowName).toBe("renamed");
    // Root untouched while the layout is unknown.
    expect(collectSessionPanes(tab.root)[0].tmux?.paneId).toBe("%0");
  });

  it("keeps the user's active pane when it still exists in the new tree", () => {
    const host = hostTab();
    const tabs1 = reconcileTmuxWindowTabs(
      [host],
      state([window("@1", "a", tmuxTree("%0", "%1"), true)]),
      host,
    ).tabs;
    const tab1 = tmuxWindowTabs(tabs1, CTRL)[0];
    const secondPane = collectSessionPanes(tab1.root).find(
      (p) => p.tmux?.paneId === "%1",
    )!;
    const withSelection = tabs1.map((t) =>
      t.id === tab1.id ? { ...t, activePaneId: secondPane.id } : t,
    );

    const tabs2 = reconcileTmuxWindowTabs(
      withSelection,
      state([window("@1", "a", tmuxTree("%0", "%1"), true)]),
      host,
    ).tabs;
    expect(tmuxWindowTabs(tabs2, CTRL)[0].activePaneId).toBe(secondPane.id);
  });
});

describe("findTmuxHostTab", () => {
  it("finds the tab containing the control channel's leaf", () => {
    const host = hostTab(2);
    const tabs = [{ ...host, id: "h" }, hostTab()];
    expect(findTmuxHostTab(tabs, CTRL)?.id).toBe("h");
    expect(findTmuxHostTab(tabs, "session-other")).toBeNull();
  });
});

describe("getTabDisplayName", () => {
  it("titles a window tab by its window name", () => {
    const host = hostTab();
    const tabs = reconcileTmuxWindowTabs(
      [host],
      state([window("@1", "editor", tmuxTree("%0"), true)]),
      host,
    ).tabs;
    const winTab = tmuxWindowTabs(tabs, CTRL)[0];
    expect(getTabDisplayName(winTab)).toBe("editor");
    expect(getTabDisplayName({ ...winTab, customName: "pinned" })).toBe(
      "pinned",
    );
    expect(
      getTabDisplayName({
        ...winTab,
        tmux: { ...winTab.tmux!, windowName: "" },
      }),
    ).toBe("tmux");
  });
});

describe("serializeTabsForPersistence", () => {
  it("drops tmux window tabs and keeps the host tab restorable", () => {
    const host = hostTab();
    const tabs = reconcileTmuxWindowTabs(
      [host],
      state([
        window("@1", "a", tmuxTree("%0", "%1"), true),
        window("@2", "b", tmuxTree("%2")),
      ]),
      host,
    ).tabs;

    const persisted = serializeTabsForPersistence(tabs);
    expect(persisted).toHaveLength(1);
    expect(persisted[0].root?.kind).toBe("leaf");
    if (persisted[0].root?.kind === "leaf") {
      expect(persisted[0].root.session_type).toBe("SSH");
      expect(persisted[0].root.connection_id).toBe("conn-1");
    }
  });
});

describe("collapseTmuxPanes", () => {
  it("collapses a tmux subtree to its first leaf as a safety net", () => {
    const win = window("@1", "a", tmuxTree("%0", "%1"));
    const built = buildWindowPaneTree(
      state([win]),
      win,
      new Map(),
      leaf("ssh-1", CTRL, "prod-box"),
    )!;
    const collapsed = collapseTmuxPanes(built.root);
    expect(collapsed.kind).toBe("leaf");
    const plain = collapsed as SessionPane;
    expect(plain.type).toBe("SSH");
    expect(plain.connectionId).toBe("conn-1");
  });
});

describe("remote window ordering", () => {
  it("reorders existing tabs on swap-window and move-window without replacing panes", () => {
    const host = hostTab();
    const windows = [
      window("@1", "a", tmuxTree("%0"), true),
      window("@2", "b", tmuxTree("%1")),
      window("@3", "c", tmuxTree("%2")),
    ];
    const before = reconcileTmuxWindowTabs([host], state(windows), host).tabs;
    const oldTabs = tmuxWindowTabs(before, CTRL);
    const ids = new Map(
      oldTabs.map((tab) => [tab.tmux!.windowId, { tabId: tab.id, paneId: tab.activePaneId }]),
    );
    const unrelated = { ...hostTab(), id: "other", root: localLeaf("other-pane") };
    const interleaved = [host, oldTabs[0], unrelated, ...oldTabs.slice(1)];
    const swapped = reconcileTmuxWindowTabs(
      interleaved,
      state([
        { ...windows[0], index: 8 },
        { ...windows[1], index: 1 },
        { ...windows[2], index: 3 },
      ]),
      host,
    ).tabs;
    expect(swapped.filter((tab) => tab.tmux).map((tab) => tab.tmux!.windowId)).toEqual([
      "@2",
      "@3",
      "@1",
    ]);
    expect(swapped[2]).toBe(unrelated);
    const moved = reconcileTmuxWindowTabs(
      swapped,
      state([
        { ...windows[0], index: 8, tree: null },
        { ...windows[1], index: 1 },
        { ...windows[2], index: 0 },
      ]),
      host,
    ).tabs;
    expect(tmuxWindowTabs(moved, CTRL).map((tab) => tab.tmux!.windowId)).toEqual([
      "@3",
      "@2",
      "@1",
    ]);
    for (const tab of tmuxWindowTabs(moved, CTRL)) {
      expect({ tabId: tab.id, paneId: tab.activePaneId }).toEqual(ids.get(tab.tmux!.windowId));
    }
  });
});

describe("tmux leaf activation", () => {
  it("keeps the host active and routes AI actions according to the selected local leaf", () => {
    const host = hostTab(2);
    const tabs = reconcileTmuxWindowTabs(
      [host],
      state([window("@1", "bash", tmuxTree("%0", "%1"), true)]),
      host,
    ).tabs;
    const child = tmuxWindowTabs(tabs, CTRL)[0];
    const target = collectSessionPanes(child.root)[1];
    const selected = activateWorkspacePane(tabs, child.id, target.id)!;
    expect(selected.activeTabId).toBe(host.id);
    expect(selected.tabs.find((tab) => tab.id === host.id)!.activePaneId).toBe("ssh-1");
    expect(resolveWorkspaceActivePane(selected.tabs, selected.tabs[0])?.sessionId).toBe(
      target.sessionId,
    );
    const local = activateWorkspacePane(selected.tabs, host.id, "local-1")!;
    expect(local.activeTabId).toBe(host.id);
    expect(resolveWorkspaceActivePane(local.tabs, local.tabs[0])?.id).toBe("local-1");
    expect(activateWorkspacePane(tabs, child.id, "missing")).toBeNull();
  });

  it("routes two SSH control leaves independently through detach", () => {
    const a = leaf("ssh-a", CTRL);
    const b = leaf("ssh-b", "ctrl-b");
    const host = { ...hostTab(), root: split(a, b), activePaneId: a.id };
    let tabs = reconcileTmuxWindowTabs(
      [host],
      state([window("@1", "a", tmuxTree("%0"), true)]),
      host,
    ).tabs;
    const bState = {
      ...state([
        window(
          "@2",
          "b",
          { kind: "pane", paneId: "%5", sessionId: "ctrl-b::pane5", name: "b" },
          true,
        ),
      ]),
      controlSessionId: "ctrl-b",
    };
    tabs = reconcileTmuxWindowTabs(tabs, bState, host).tabs;
    const bWindow = tmuxWindowTabs(tabs, "ctrl-b")[0];
    const selected = activateWorkspacePane(tabs, bWindow.id, bWindow.activePaneId)!;
    const selectedHost = selected.tabs.find((tab) => tab.id === host.id)!;
    expect(selectedHost.activePaneId).toBe(b.id);
    expect(resolveWorkspaceActivePane(selected.tabs, selectedHost)?.sessionId).toBe(
      "ctrl-b::pane5",
    );
    const detached = reconcileTmuxWindowTabs(
      selected.tabs,
      { ...state([]), controlSessionId: "ctrl-b" },
      selectedHost,
    ).tabs;
    expect(resolveWorkspaceActivePane(detached, selectedHost)?.sessionId).toBe("ctrl-b");
    expect(tmuxWindowTabs(detached, CTRL)).toHaveLength(1);
  });
});
