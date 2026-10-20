import { fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FuzzyResult } from "@/types/global";
import CommandSuggestions from "./CommandSuggestions";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

afterEach(() => vi.restoreAllMocks());

function suggestion(
  display: string,
  indices: number[] = [],
  overrides: Partial<FuzzyResult> = {},
): FuzzyResult {
  return {
    command: display,
    display,
    indices,
    score: 10,
    source: "history",
    ...overrides,
  };
}

function renderSuggestion(result: FuzzyResult, selectedIndex = -1) {
  const onSelect = vi.fn();
  const view = render(
    <CommandSuggestions
      suggestions={[result]}
      visible
      selectedIndex={selectedIndex}
      cursorPosition={{ top: 100, left: 100 }}
      onSelect={onSelect}
      onDismiss={vi.fn()}
    />,
  );
  const text = view.container.querySelector(
    "span.truncate > span",
  ) as HTMLSpanElement;
  expect(text).not.toBeNull();
  return { ...view, text, onSelect };
}

describe("CommandSuggestions whitespace rendering", () => {
  it.each([
    {
      label: "a separate unhighlighted space between matches",
      display: "ulimit -c",
      indices: [0, 1, 2, 3, 4, 5, 7],
      fragments: ["ulimit", " ", "-", "c"],
      highlighted: [true, false, true, false],
    },
    {
      label: "a highlighted space",
      display: "ulimit -c",
      indices: [6],
      fragments: ["ulimit", " ", "-c"],
      highlighted: [false, true, false],
    },
    {
      label: "consecutive spaces including leading and trailing spaces",
      display: "  echo  hello  ",
      indices: [2, 8],
      fragments: ["  ", "e", "cho  ", "h", "ello  "],
      highlighted: [false, true, false, true, false],
    },
    {
      label: "plain unhighlighted command text",
      display: " echo  hello ",
      indices: [],
      fragments: [" echo  hello "],
      highlighted: [false],
    },
  ])(
    "preserves $label without wrapping",
    ({ display, indices, fragments, highlighted }) => {
      const { text } = renderSuggestion(suggestion(display, indices));

      expect(text.textContent).toBe(display);
      expect(text.classList.contains("whitespace-pre")).toBe(true);
      expect(Array.from(text.children, (part) => part.textContent)).toEqual(
        fragments,
      );
      expect(
        Array.from(text.children, (part) =>
          part.classList.contains("font-semibold"),
        ),
      ).toEqual(highlighted);
      expect(
        Array.from(text.children, (part) => (part as HTMLElement).style.color),
      ).toEqual(
        highlighted.map((match) =>
          match ? "var(--df-accent)" : "var(--df-text)",
        ),
      );
    },
  );

  it("keeps Quick Command display separate from the original command on selection", () => {
    const result = suggestion("Restart  service", [0, 1], {
      source: "quickCommand",
      command: "sudo  systemctl restart service",
    });
    const { text, onSelect } = renderSuggestion(result);

    expect(text.textContent).toBe("Restart  service");
    expect(text.classList.contains("whitespace-pre")).toBe(true);
    fireEvent.click(text.closest(".cursor-pointer") as HTMLElement);
    expect(onSelect).toHaveBeenCalledExactlyOnceWith(
      "sudo  systemctl restart service",
    );
  });

  it("keeps long suggestions within the single-line truncated row and selected tooltip", async () => {
    vi.spyOn(Element.prototype, "scrollWidth", "get").mockReturnValue(900);
    vi.spyOn(Element.prototype, "clientWidth", "get").mockReturnValue(200);
    const result = suggestion("Long  command display ".repeat(12), [0, 1], {
      source: "quickCommand",
      command: "echo  original  command",
    });
    const { container, text } = renderSuggestion(result, 0);

    expect(container.firstElementChild?.classList.contains("w-[380px]")).toBe(
      true,
    );
    expect(text.parentElement?.classList.contains("truncate")).toBe(true);
    expect(text.classList.contains("whitespace-pre")).toBe(true);
    await waitFor(() => {
      const tooltip = document.querySelector('[data-slot="tooltip-content"]');
      expect(tooltip?.firstElementChild?.textContent).toBe(
        "echo  original  command",
      );
    });
  });
});
