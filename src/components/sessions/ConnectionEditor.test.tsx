import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect, useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConnectionEditor, EditorSection } from "./ConnectionEditor";
import {
  type AlgorithmTab,
  CONNECTION_CATEGORIES,
  type EditorCategory,
  type EditorIssue,
  type EditorProtocol,
  type ForwardingTab,
} from "./connectionEditorModel";

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
let width = 900;
const originalObserver = globalThis.ResizeObserver;
let callbacks: Set<() => void>;
const resize = () => {
  for (const callback of [...callbacks]) callback();
};
const mounted = vi.fn();
function Controller() {
  const [value, setValue] = useState("");
  useEffect(() => {
    mounted();
  }, []);
  return (
    <>
      <EditorSection category="basic">
        <input
          aria-label="draft"
          value={value}
          onChange={(event) => setValue(event.target.value)}
        />
      </EditorSection>
      <EditorSection category="authentication">
        <input aria-label="username" data-editor-field="username" />
      </EditorSection>
    </>
  );
}
function Harness({ protocol = "ssh", issue }: { protocol?: EditorProtocol; issue?: EditorIssue }) {
  const [category, setCategory] = useState<EditorCategory>("basic");
  const [algorithmTab, setAlgorithmTab] = useState<AlgorithmTab>("kex");
  const [forwardingTab, setForwardingTab] = useState<ForwardingTab>("agent");
  const [focusIssue, setFocusIssue] = useState<EditorIssue | null>(null);
  return (
    <>
      <button
        onClick={() => {
          if (issue) {
            setCategory(issue.category);
            setFocusIssue({ ...issue });
          }
        }}
      >
        locate
      </button>
      <ConnectionEditor
        protocol={protocol}
        category={category}
        onCategoryChange={(next) => {
          setFocusIssue(null);
          setCategory(next);
        }}
        algorithmTab={algorithmTab}
        setAlgorithmTab={setAlgorithmTab}
        forwardingTab={forwardingTab}
        setForwardingTab={setForwardingTab}
        onAlgorithmsLoaded={vi.fn()}
        issues={issue ? [issue] : []}
        focusIssue={focusIssue}
      >
        <Controller />
      </ConnectionEditor>
    </>
  );
}

beforeEach(() => {
  width = 900;
  mounted.mockClear();
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => ({
    width,
    height: 500,
    x: 0,
    y: 0,
    top: 0,
    bottom: 500,
    left: 0,
    right: width,
    toJSON: () => ({}),
  }));
  callbacks = new Set();
  globalThis.ResizeObserver = class {
    constructor(private callback: () => void) {
      callbacks.add(callback);
    }
    observe() {}
    disconnect() {
      callbacks.delete(this.callback);
    }
    unobserve() {}
  } as unknown as typeof ResizeObserver;
});
afterEach(() => {
  vi.restoreAllMocks();
  globalThis.ResizeObserver = originalObserver;
});

describe("connection editor navigation", () => {
  it.each(
    Object.keys(CONNECTION_CATEGORIES) as EditorProtocol[],
  )("exposes the %s categories in order", (protocol) => {
    render(<Harness protocol={protocol} />);
    expect(screen.getAllByRole("tab").map((tab) => tab.getAttribute("data-state"))).toHaveLength(
      CONNECTION_CATEGORIES[protocol].length,
    );
  });

  it("unmounts hidden fields while preserving their controller and draft", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.type(screen.getByRole("textbox", { name: "draft" }), "unfinished draft");
    await user.click(
      screen.getByRole("tab", { name: "connectionEditor.categories.authentication" }),
    );
    expect(screen.queryByRole("textbox", { name: "draft" })).toBeNull();
    await user.click(screen.getByRole("tab", { name: "connectionEditor.categories.basic" }));
    expect((screen.getByRole("textbox", { name: "draft" }) as HTMLInputElement).value).toBe(
      "unfinished draft",
    );
    expect(mounted).toHaveBeenCalledTimes(1);
  });

  it("keeps keyboard focus on navigation and Tab enters the selected page", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    screen.getByRole("tab", { name: "connectionEditor.categories.basic" }).focus();
    await user.keyboard("{ArrowDown}");
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("tab", { name: "connectionEditor.categories.authentication" }),
      ),
    );
    await user.tab();
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "username" }));
  });

  it("uses the editor width and transfers navigation focus across the exact breakpoint", async () => {
    render(<Harness />);
    screen.getByRole("tab", { name: "connectionEditor.categories.basic" }).focus();
    act(() => {
      width = 759;
      resize();
    });
    expect(screen.queryByRole("tab")).toBeNull();
    expect(document.activeElement).toBe(
      screen.getByRole("combobox", { name: "connectionEditor.category" }),
    );
    act(() => {
      width = 760;
      resize();
    });
    expect(document.activeElement).toBe(
      screen.getByRole("tab", { name: "connectionEditor.categories.basic" }),
    );
  });

  it("remembers independent category scroll positions", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const content = screen.getByRole("region");
    content.scrollTop = 120;
    fireEvent.scroll(content);
    await user.click(
      screen.getByRole("tab", { name: "connectionEditor.categories.authentication" }),
    );
    expect(content.scrollTop).toBe(0);
    content.scrollTop = 60;
    fireEvent.scroll(content);
    await user.click(screen.getByRole("tab", { name: "connectionEditor.categories.basic" }));
    expect(content.scrollTop).toBe(120);
  });

  it("mounts, focuses and scrolls a hidden error; manual navigation cancels stale focus", async () => {
    const user = userEvent.setup();
    render(
      <Harness issue={{ category: "authentication", field: "username", message: "required" }} />,
    );
    await user.click(screen.getByRole("button", { name: "locate" }));
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "username" })),
    );
    expect(Element.prototype.scrollIntoView).toHaveBeenCalled();
    await user.click(screen.getByRole("tab", { name: "connectionEditor.categories.basic" }));
    expect(document.activeElement).toBe(
      screen.getByRole("tab", { name: "connectionEditor.categories.basic" }),
    );
  });
});
