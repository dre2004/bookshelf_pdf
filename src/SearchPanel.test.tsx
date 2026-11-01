// @vitest-environment jsdom
import { describe, expect, it, vi, beforeAll, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import SearchPanel from "./SearchPanel";
import type { SearchMatch } from "./types";

beforeAll(() => {
  // jsdom lacks scrollIntoView, used to keep the active row visible.
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(cleanup);

const results: SearchMatch[] = [
  { page: 10, start: 40, end: 44 },
  { page: 30, start: 100, end: 104 },
];

type PanelProps = React.ComponentProps<typeof SearchPanel>;

function renderPanel(overrides: Partial<PanelProps> = {}) {
  const props: PanelProps = {
    query: "read",
    onQueryChange: vi.fn(),
    caseSensitive: false,
    wholeWords: false,
    onToggleCase: vi.fn(),
    onToggleWholeWords: vi.fn(),
    rows: results.map((match, index) => ({
      match,
      snippet: index === 0 ? (["you should ", "read", " this book"] as [string, string, string]) : null,
    })),
    activeIndex: 0,
    searching: false,
    truncated: false,
    onSelect: vi.fn(),
    onNext: vi.fn(),
    onPrev: vi.fn(),
    onClose: vi.fn(),
    ...overrides,
  };
  render(<SearchPanel {...props} />);
  return props;
}

describe("SearchPanel", () => {
  it("renders count, rows with page numbers and snippets", () => {
    renderPanel();
    expect(screen.getByText("1/2")).toBeTruthy();
    expect(screen.getAllByText(/p\. \d+/).length).toBe(2);
    expect(screen.getByText("read", { selector: "mark" })).toBeTruthy();
  });

  it("toggles call back", () => {
    const props = renderPanel();
    fireEvent.click(screen.getByTitle("Match case"));
    expect(props.onToggleCase).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByTitle("Whole words"));
    expect(props.onToggleWholeWords).toHaveBeenCalledTimes(1);
  });

  it("clicking a row selects it; Enter cycles; Escape closes", () => {
    const props = renderPanel();
    fireEvent.click(screen.getAllByText(/p\. \d+/)[1]);
    expect(props.onSelect).toHaveBeenCalledWith(1);

    const input = screen.getByPlaceholderText("Search in document…");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(props.onNext).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(props.onPrev).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(input, { key: "Escape" });
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  it("shows the truncated note when the result cap was hit", () => {
    renderPanel({ truncated: true });
    expect(screen.getByText(/Result limit reached/)).toBeTruthy();
  });

  it("empty state before any query", () => {
    renderPanel({ query: "", rows: [] });
    expect(screen.getByText("Type to search the document.")).toBeTruthy();
  });
});
