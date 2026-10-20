import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { SupportedSshAlgorithms } from "@/types/global";
import {
  type AlgorithmTab,
  CONNECTION_CATEGORIES,
  type EditorCategory,
  type EditorIssue,
  type EditorProtocol,
  editorCategoryKey,
  type ForwardingTab,
} from "./connectionEditorModel";

interface EditorContextValue {
  category: EditorCategory;
  algorithmTab: AlgorithmTab;
  setAlgorithmTab: (tab: AlgorithmTab) => void;
  forwardingTab: ForwardingTab;
  onAlgorithmsLoaded: (algorithms: SupportedSshAlgorithms) => void;
}
const EditorContext = createContext<EditorContextValue | null>(null);
export const useConnectionEditor = () => useContext(EditorContext);

/** Dialogs opened without a Radix Trigger still return focus to their actual opener. */
export function useEditorDialogState() {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLElement | null>(null);
  const restoreFocus = useCallback((event: Event) => {
    if (trigger.current?.isConnected) {
      event.preventDefault();
      trigger.current.focus();
    }
  }, []);
  const changeOpen = useCallback((next: boolean) => {
    if (next && document.activeElement instanceof HTMLElement)
      trigger.current = document.activeElement;
    setOpen(next);
  }, []);
  return [open, changeOpen, restoreFocus] as const;
}

/** Keep protocol controllers alive while mounting only the selected field view. */
export function EditorSection({
  category,
  tab,
  children,
}: {
  category: EditorCategory;
  tab?: ForwardingTab;
  children: ReactNode;
}) {
  const editor = useConnectionEditor();
  if (editor && (editor.category !== category || (tab && editor.forwardingTab !== tab)))
    return null;
  return <div className="min-w-0 space-y-3">{children}</div>;
}

export function ConnectionBasicInformation({ children }: { children: ReactNode }) {
  return <EditorSection category="basic">{children}</EditorSection>;
}

interface ConnectionEditorProps extends EditorContextValue {
  protocol: EditorProtocol;
  onCategoryChange: (category: EditorCategory) => void;
  setForwardingTab: (tab: ForwardingTab) => void;
  issues: EditorIssue[];
  focusIssue: EditorIssue | null;
  children: ReactNode;
}

export function ConnectionEditor(props: ConnectionEditorProps) {
  const { t } = useTranslation();
  const { protocol, category, onCategoryChange, issues, focusIssue, children } = props;
  const rootRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const positions = useRef(new Map<string, number>());
  const [wide, setWide] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuOpenRef = useRef(false);
  const widthMode = useRef(false);
  const scrollProtocol = useRef(protocol);
  const navFocus = useRef(false);
  const categories = CONNECTION_CATEGORIES[protocol];
  const label = (value: EditorCategory) => t(editorCategoryKey(protocol, value));
  const hasError = (value: EditorCategory) => issues.some((issue) => issue.category === value);

  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const resize = () => {
      const nextWide = root.getBoundingClientRect().width >= 760;
      if (nextWide === widthMode.current) return;
      widthMode.current = nextWide;
      const focused = document.activeElement;
      navFocus.current =
        menuOpenRef.current ||
        (focused instanceof HTMLElement && !!focused.closest("[data-editor-navigation]"));
      menuOpenRef.current = false;
      setMenuOpen(false);
      setWide(nextWide);
    };
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(root);
    return () => observer.disconnect();
  }, []);

  useLayoutEffect(() => {
    if (navFocus.current) {
      rootRef.current
        ?.querySelector<HTMLElement>(
          wide
            ? "[data-editor-navigation] [data-state=active]"
            : "[data-editor-navigation] [role=combobox]",
        )
        ?.focus();
      navFocus.current = false;
    }
  }, [wide]);

  useLayoutEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    if (scrollProtocol.current !== protocol) {
      positions.current.clear();
      scrollProtocol.current = protocol;
    }
    const key = `${protocol}:${category}`;
    content.scrollTop = positions.current.get(key) ?? 0;
  }, [protocol, category]);

  useLayoutEffect(() => {
    if (!focusIssue || focusIssue.category !== category) return;
    // Layout is committed here, including a newly selected algorithm/forwarding tab.
    const frame = requestAnimationFrame(() => {
      const candidates = contentRef.current?.querySelectorAll<HTMLElement>("[data-editor-field]");
      const matching = Array.from(candidates ?? []).filter(
        (element) =>
          element.dataset.editorField === focusIssue.field &&
          (focusIssue.endpointIndex === undefined ||
            element.dataset.endpointIndex === String(focusIssue.endpointIndex)),
      );
      const target = matching.find((element) => element.matches("input,textarea")) ?? matching[0];
      const control =
        target?.matches("input,textarea,button") || focusIssue.field === "forwardingEndpoints"
          ? target
          : (target?.querySelector<HTMLElement>(
              "input:not(:disabled),textarea:not(:disabled),button:not(:disabled),[tabindex='0']",
            ) ?? (target?.hasAttribute("tabindex") ? target : undefined));
      const destination = control ?? headingRef.current;
      destination?.focus({ preventScroll: true });
      destination?.scrollIntoView?.({ block: "nearest" });
    });
    return () => cancelAnimationFrame(frame);
  }, [focusIssue, category]);

  const navigate = (value: string) => onCategoryChange(value as EditorCategory);
  return (
    <EditorContext.Provider value={props}>
      <div ref={rootRef} className="flex min-h-0 min-w-0 flex-1 flex-col">
        {!wide && (
          <div className="shrink-0 px-4 py-3" data-editor-navigation>
            <Select
              value={category}
              onValueChange={navigate}
              open={menuOpen}
              onOpenChange={(open) => {
                menuOpenRef.current = open;
                setMenuOpen(open);
              }}
            >
              <SelectTrigger aria-label={t("connectionEditor.category")} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {categories.map((value) => (
                  <SelectItem key={value} value={value}>
                    {label(value)}
                    {hasError(value) ? ` · ${t("connectionEditor.hasErrors")}` : ""}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
        <div className="flex min-h-0 min-w-0 flex-1">
          {wide && (
            <Tabs
              value={category}
              onValueChange={navigate}
              orientation="vertical"
              activationMode="automatic"
              className="w-[180px] shrink-0 overflow-y-auto border-r p-3"
              data-editor-navigation
            >
              <TabsList
                className="h-auto w-full items-stretch gap-1 bg-transparent p-0"
                aria-label={t("connectionEditor.category")}
              >
                {categories.map((value) => (
                  <TabsTrigger
                    key={value}
                    value={value}
                    aria-controls="connection-editor-content"
                    className="h-auto min-h-9 flex-none whitespace-normal break-words px-3 py-2 text-left text-xs"
                  >
                    <span className="min-w-0 flex-1">{label(value)}</span>
                    {hasError(value) && (
                      <span
                        role="img"
                        className="text-destructive"
                        aria-label={t("connectionEditor.hasErrors")}
                      >
                        ●
                      </span>
                    )}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
          )}
          <section
            ref={contentRef}
            id="connection-editor-content"
            aria-labelledby="connection-editor-heading"
            onScroll={(event) =>
              positions.current.set(`${protocol}:${category}`, event.currentTarget.scrollTop)
            }
            className="@container/editor min-h-0 min-w-0 flex-1 overflow-y-auto overflow-x-hidden p-4 sm:p-5"
          >
            <h2
              ref={headingRef}
              id="connection-editor-heading"
              tabIndex={-1}
              className="mb-4 text-sm font-semibold outline-none"
            >
              {label(category)}
            </h2>
            {category === "forwarding" && (
              <Tabs
                value={props.forwardingTab}
                onValueChange={(value) => props.setForwardingTab(value as ForwardingTab)}
                className="mb-3"
              >
                <TabsList className="w-full">
                  <TabsTrigger value="agent" className="whitespace-normal">
                    Agent Forwarding
                  </TabsTrigger>
                  <TabsTrigger value="x11">X11</TabsTrigger>
                </TabsList>
              </Tabs>
            )}
            <div className="space-y-3">{children}</div>
            {issues
              .filter((issue) => issue.category === category)
              .map((issue) => (
                <p
                  key={`${issue.field}:${issue.endpointIndex ?? ""}`}
                  className="mt-3 break-words text-xs text-destructive"
                >
                  {issue.message}
                </p>
              ))}
          </section>
        </div>
      </div>
    </EditorContext.Provider>
  );
}
