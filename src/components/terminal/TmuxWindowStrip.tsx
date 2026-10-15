import { Columns2, Plus, Rows2, Unplug, X } from "lucide-react";
import { useCallback, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useApp } from "@/context/AppContext";
import { invoke } from "@/lib/invoke";
import type { Tab } from "@/types/global";

interface TmuxWindowStripProps {
  /** SSH session id whose channel is running `tmux -CC`. */
  controlSessionId: string;
  /** Window sub-tabs of this control session, in order. */
  windowTabs: Tab[];
}

/** Common tmux commands offered as suggestions in the command input. */
const TMUX_COMMON_COMMANDS = [
  "resize-pane -Z",
  "select-layout even-horizontal",
  "select-layout even-vertical",
  "select-layout main-horizontal",
  "select-layout tiled",
  "kill-pane",
  "respawn-pane -k",
  "rename-window ",
  "break-pane",
  "swap-pane -U",
  "swap-pane -D",
  "last-window",
  "synchronize-panes",
] as const;

/**
 * WindTerm-style status bar inside the SSH leaf hosting tmux: the tmux icon
 * on the left marks the leaf as tmux-driven, followed by one sub-tab per tmux
 * window (select → `select-window`, × → `kill-window`) and a `+` for
 * `new-window`; the right side carries pane splits and a free-form
 * control command input — the `-CC` channel swallows tmux prefix keys, so
 * tmux-level actions must go through the control channel instead.
 */
function TmuxWindowStrip({
  controlSessionId,
  windowTabs,
}: TmuxWindowStripProps) {
  const { t } = useTranslation();
  const { closeTab } = useApp();
  const [command, setCommand] = useState("");
  const [suggestionsOpen, setSuggestionsOpen] = useState(false);

  // Unique clipPath id — several host tabs can mount a strip at once.
  const iconClipId = useId();

  const send = useCallback(
    (line: string) => {
      const trimmed = line.trim();
      if (!trimmed) return;
      void invoke("tmux_send_command", {
        sessionId: controlSessionId,
        command: trimmed,
      }).catch(() => {});
    },
    [controlSessionId],
  );

  const detach = useCallback(() => {
    void invoke("tmux_detach", { sessionId: controlSessionId }).catch(() => {});
  }, [controlSessionId]);

  const submitCommand = useCallback(() => {
    if (!command.trim()) return;
    send(command);
    setCommand("");
  }, [command, send]);

  return (
    <div
      className="flex h-9 shrink-0 items-center gap-0.5 border-t px-2"
      style={{
        borderColor: "var(--df-border)",
        background: "var(--df-bg-panel)",
      }}
      data-testid="tmux-window-strip"
    >
      {/* Passive mark — the control channel drives this tab. Inlined so
          dark theme gets muted fills instead of the light asset's white. */}
      <svg
        viewBox="0 0 256 256"
        role="img"
        aria-label="tmux"
        className="h-4.5 w-4.5 shrink-0 select-none"
      >
        <clipPath id={iconClipId}>
          <rect width="256" height="256" rx="36" ry="36" />
        </clipPath>
        <g clipPath={`url(#${iconClipId})`}>
          <rect
            width="256"
            height="256"
            className="fill-white dark:fill-[#262626]"
          />
          <rect
            width="120"
            height="232"
            className="fill-[#b0b0b0] dark:fill-[#8f8f8f]"
          />
          <rect
            x="134"
            width="122"
            height="105"
            className="fill-[#b0b0b0] dark:fill-[#8f8f8f]"
          />
          <rect
            x="134"
            y="120"
            width="122"
            height="112"
            className="fill-[#b0b0b0] dark:fill-[#8f8f8f]"
          />
          <rect
            y="232"
            width="256"
            height="24"
            className="fill-[#1fb82a] dark:fill-[#17a124]"
          />
        </g>
      </svg>
      {/* Detach is a session-level action, so it sits with the session
          badge rather than among the per-window controls. */}
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="ml-1.5 h-7 w-7"
        title={t("tmux.detach")}
        onClick={detach}
      >
        <Unplug className="h-4 w-4" />
      </Button>
      <div
        className="mx-1.5 h-5 w-px"
        style={{ background: "var(--df-border)" }}
      />
      {windowTabs.map((windowTab) => {
        const tmux = windowTab.tmux;
        if (!tmux) return null;
        return (
          // The active underline lives on the group so it spans the name
          // and the close button's slot.
          <span
            key={windowTab.id}
            className="group flex items-center pr-1.5"
            style={{
              borderBottom: tmux.active
                ? "2px solid var(--df-primary)"
                : "2px solid transparent",
            }}
          >
            <button
              type="button"
              title={tmux.windowName}
              onClick={() => send(`select-window -t '${tmux.windowId}'`)}
              className={`flex items-center gap-1 rounded-t py-1 pl-2.5 pr-1 font-mono text-xs ${
                tmux.active ? "font-semibold" : "opacity-70 hover:opacity-100"
              }`}
            >
              <span className="opacity-60">{tmux.windowIndex}:</span>
              {tmux.windowName}
            </button>
            {/* The × keeps its slot while hidden so hovering never shifts
                the sub-tab row. */}
            <button
              type="button"
              title={t("tmux.killWindow")}
              aria-label={t("tmux.killWindow")}
              onClick={() => closeTab(windowTab.id)}
              className="invisible flex h-4 w-4 items-center justify-center rounded group-hover:visible hover:bg-accent"
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        );
      })}
      <button
        type="button"
        title={t("tmux.newWindow")}
        onClick={() => send("new-window")}
        className="flex items-center rounded px-1.5 py-1 opacity-70 hover:opacity-100"
      >
        <Plus className="h-4 w-4" />
      </button>

      <div className="flex-1" />

      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="h-7 w-7"
        title={t("tmux.splitVertical")}
        onClick={() => send("split-window -h")}
      >
        <Columns2 className="h-4 w-4" />
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="h-7 w-7"
        title={t("tmux.splitHorizontal")}
        onClick={() => send("split-window -v")}
      >
        <Rows2 className="h-4 w-4" />
      </Button>

      <div
        className="mx-1.5 h-5 w-px"
        style={{ background: "var(--df-border)" }}
      />

      <div className="relative w-56">
        <Input
          value={command}
          onChange={(event) => setCommand(event.target.value)}
          onFocus={() => setSuggestionsOpen(true)}
          onBlur={() => setSuggestionsOpen(false)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              submitCommand();
            } else if (event.key === "Escape") {
              setSuggestionsOpen(false);
            }
          }}
          placeholder={t("tmux.commandPlaceholder")}
          className="h-7 w-full border-none bg-transparent px-1 font-mono text-xs shadow-none focus-visible:ring-0"
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
        />
        {suggestionsOpen ? (
          <div
            className="absolute bottom-full right-0 z-50 mb-1 max-h-64 w-64 overflow-auto rounded-md border p-1 shadow-md"
            style={{
              borderColor: "var(--df-border)",
              // -solid keeps the suggestion list opaque over pane content.
              background: "var(--df-bg-panel-solid, var(--df-bg-panel))",
            }}
          >
            {TMUX_COMMON_COMMANDS.filter((entry) =>
              entry.trim().includes(command.trim()),
            ).map((entry) => (
              <button
                key={entry}
                type="button"
                // Picking a suggestion fires the command immediately.
                onMouseDown={(event) => {
                  event.preventDefault();
                  send(entry);
                  setCommand("");
                  setSuggestionsOpen(false);
                }}
                className="block w-full rounded px-2 py-1.5 text-left font-mono text-xs hover:bg-accent"
              >
                {entry}
              </button>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}

export default TmuxWindowStrip;
