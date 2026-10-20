import { memo, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { MdDelete, MdFlashOn, MdHistory, MdTipsAndUpdates } from "react-icons/md";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  getSuggestionPopupStyle,
  type SuggestionCursorPosition,
} from "@/lib/terminalSuggestionPosition";
import type { FuzzyResult } from "@/types/global";

interface CommandSuggestionsProps {
  suggestions: FuzzyResult[];
  visible: boolean;
  selectedIndex: number;
  cursorPosition: SuggestionCursorPosition;
  onSelect: (command: string) => void;
  onDismiss: () => void;
  onDeleteHistory?: (command: string) => void;
}

/** Per-source icon used as a prefix to distinguish item origins. */
const SOURCE_ICON: Record<string, React.ElementType> = {
  history: MdHistory,
  quickCommand: MdFlashOn,
};

/** Render a single suggestion with matched characters highlighted. */
function HighlightedCommand({ text, indices }: { text: string; indices: number[] }) {
  const indexSet = new Set(indices);
  const parts: { text: string; highlighted: boolean; start: number }[] = [];

  let i = 0;
  while (i < text.length) {
    const start = i;
    const isHighlighted = indexSet.has(i);
    let j = i + 1;
    while (j < text.length && indexSet.has(j) === isHighlighted) {
      j++;
    }
    parts.push({ text: text.slice(i, j), highlighted: isHighlighted, start });
    i = j;
  }

  return (
    <span className="font-mono text-[0.75rem] whitespace-pre">
      {parts.map((part) =>
        part.highlighted ? (
          <span
            key={`${part.start}-${part.highlighted}-${part.text}`}
            className="font-semibold"
            style={{ color: "var(--df-accent)" }}
          >
            {part.text}
          </span>
        ) : (
          <span
            key={`${part.start}-${part.highlighted}-${part.text}`}
            style={{ color: "var(--df-text)" }}
          >
            {part.text}
          </span>
        ),
      )}
    </span>
  );
}

/** Suggestion text with truncation-aware tooltip. Only shows tooltip when text overflows. */
function SuggestionText({
  result,
  isSelected,
}: {
  result: FuzzyResult;
  isSelected: boolean;
}) {
  const textRef = useRef<HTMLSpanElement>(null);
  const [truncated, setTruncated] = useState(false);
  const [hovered, setHovered] = useState(false);

  useEffect(() => {
    const el = textRef.current;
    if (el) {
      setTruncated(el.scrollWidth > el.clientWidth);
    }
  }, [result.display]);

  if (!truncated) {
    return (
      <span ref={textRef} className="min-w-0 flex-1 truncate">
        <HighlightedCommand text={result.display} indices={result.indices} />
      </span>
    );
  }

  return (
    <Tooltip open={isSelected || hovered}>
      <TooltipTrigger asChild>
        <span
          ref={textRef}
          className="min-w-0 flex-1 truncate"
          onMouseEnter={() => setHovered(true)}
          onMouseLeave={() => setHovered(false)}
        >
          <HighlightedCommand text={result.display} indices={result.indices} />
        </span>
      </TooltipTrigger>
      <TooltipContent
        side="top"
        className="z-[10000] max-w-[600px]"
        onMouseDown={(e) => e.preventDefault()}
      >
        <span className="font-mono text-[0.75rem] break-all">{result.command}</span>
      </TooltipContent>
    </Tooltip>
  );
}

/** Popup list of fuzzy-matched suggestions from multiple providers. */
function CommandSuggestions({
  suggestions,
  visible,
  selectedIndex,
  cursorPosition,
  onSelect,
  onDismiss: _onDismiss,
  onDeleteHistory,
}: CommandSuggestionsProps) {
  const { t } = useTranslation();
  const listRef = useRef<HTMLDivElement>(null);
  const selectedRef = useRef<HTMLDivElement>(null);

  // Scroll the selected item into view
  useEffect(() => {
    if (selectedRef.current) {
      selectedRef.current.scrollIntoView({ block: "nearest" });
    }
  });

  if (!visible || suggestions.length === 0) {
    return null;
  }

  const popupWidth = 380;
  const popupStyle = getSuggestionPopupStyle(cursorPosition, popupWidth);

  return (
    <div
      className="fixed z-[9999] w-[380px] overflow-y-auto rounded-lg border backdrop-blur-sm shadow-2xl terminal-scroll"
      ref={listRef}
      style={{
        ...popupStyle,
        backgroundColor: "color-mix(in srgb, var(--df-bg-panel) 95%, transparent)",
        borderColor: "var(--df-border)",
      }}
      onMouseDown={(e) => {
        // Prevent the terminal from losing focus
        e.preventDefault();
      }}
    >
      {/* Unified header */}
      <div
        className="px-2 py-1.5 text-[0.625rem] uppercase tracking-wider border-b flex items-center gap-1.5"
        style={{ color: "var(--df-text-dimmed)", borderColor: "var(--df-border)" }}
      >
        <MdTipsAndUpdates className="text-[0.75rem]" />
        <span>{t("suggestions.title")}</span>
        <span className="ml-auto" style={{ color: "var(--df-text-dimmed)" }}>
          {suggestions.length}{" "}
          {suggestions.length !== 1 ? t("suggestions.matches") : t("suggestions.match")}
        </span>
      </div>

      {/* Items — prefix icon distinguishes source */}
      {suggestions.map((result, index) => {
        const Icon = SOURCE_ICON[result.source] ?? MdHistory;
        const deleteHistory = result.source === "history" ? onDeleteHistory : undefined;
        return (
          <div
            key={`${result.source}-${result.display}-${index}`}
            ref={index === selectedIndex ? selectedRef : null}
            className={`px-3 py-1.5 cursor-pointer flex items-center gap-2 transition-colors border-l-2 ${
              index === selectedIndex ? "" : "border-transparent"
            } ${index !== selectedIndex ? "df-hover" : ""}`}
            style={{
              backgroundColor:
                index === selectedIndex
                  ? "color-mix(in srgb, var(--df-primary) 20%, transparent)"
                  : undefined,
              borderLeftColor: index === selectedIndex ? "var(--df-primary)" : "transparent",
            }}
            onClick={() => onSelect(result.command)}
          >
            <Icon
              className="text-[0.75rem] shrink-0"
              style={{
                color: index === selectedIndex ? "var(--df-accent)" : "var(--df-text-dimmed)",
              }}
            />
            <SuggestionText
              result={result}
              isSelected={index === selectedIndex}
            />
            {deleteHistory && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-xs"
                    className="h-6 w-6 rounded p-0 opacity-70 hover:opacity-100"
                    aria-label={t("suggestions.deleteHistory")}
                    style={{ color: "var(--df-text-dimmed)" }}
                    onMouseDown={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                    }}
                    onClick={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      deleteHistory(result.command);
                    }}
                  >
                    <MdDelete className="text-[0.75rem]" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="right">{t("suggestions.deleteHistory")}</TooltipContent>
              </Tooltip>
            )}
          </div>
        );
      })}

      <div
        className="px-2 py-1 border-t flex items-center gap-3 text-[0.625rem]"
        style={{ borderColor: "var(--df-border)", color: "var(--df-text-dimmed)" }}
      >
        <span>
          <Kbd
            className="px-1 py-0.5 rounded text-[0.5625rem]"
            style={{ backgroundColor: "var(--df-bg-hover)", color: "var(--df-text-muted)" }}
          >
            ↑↓
          </Kbd>{" "}
          {t("suggestions.select")}
        </span>
        <span>
          <Kbd
            className="px-1 py-0.5 rounded text-[0.5625rem]"
            style={{ backgroundColor: "var(--df-bg-hover)", color: "var(--df-text-muted)" }}
          >
            Enter
          </Kbd>{" "}
          {t("suggestions.execute")}
        </span>
        <span>
          <Kbd
            className="px-1 py-0.5 rounded text-[0.5625rem]"
            style={{ backgroundColor: "var(--df-bg-hover)", color: "var(--df-text-muted)" }}
          >
            Tab
          </Kbd>{" "}
          {t("suggestions.fill")}
        </span>
        <span>
          <Kbd
            className="px-1 py-0.5 rounded text-[0.5625rem]"
            style={{ backgroundColor: "var(--df-bg-hover)", color: "var(--df-text-muted)" }}
          >
            Esc
          </Kbd>{" "}
          {t("suggestions.dismiss")}
        </span>
      </div>
    </div>
  );
}

export default memo(CommandSuggestions);
