import { describe, expect, it } from "vitest";
import { rangeBoxes, snippetAround } from "./textOverlay";
import type { PageTextData } from "./types";

function page(chars: string, boxes: number[]): PageTextData {
  return { chars, boxes, width: 500, height: 800 };
}

// A page with two 20-char lines: y=760 (line 1) and y=740 (line 2).
function twoLinePage(): PageTextData {
  const chars = "a".repeat(20) + "b".repeat(20);
  const boxes: number[] = [];
  for (let i = 0; i < 40; i++) {
    const line = i < 20 ? 0 : 1;
    boxes.push(10 + (i % 20) * 20, line === 0 ? 760 : 740, 12, 12);
  }
  return page(chars, boxes);
}

describe("rangeBoxes", () => {
  const text = twoLinePage();

  it("merges same-line characters into one box", () => {
    const boxes = rangeBoxes(text, 0, 5, 500, 800);
    expect(boxes.length).toBe(1);
    // chars at x = 10, 30, 50, 70, 90, each 12 wide → left 10, right 102
    expect(boxes[0].left).toBeCloseTo(10);
    expect(boxes[0].width).toBeCloseTo(92);
    expect(boxes[0].height).toBeCloseTo(12);
  });

  it("produces separate boxes per line", () => {
    const boxes = rangeBoxes(text, 18, 22, 500, 800);
    expect(boxes.length).toBe(2);
    const [first, second] = boxes;
    expect(first.top).toBeCloseTo(800 - (760 + 12));
    expect(second.top).toBeCloseTo(800 - (740 + 12));
  });

  it("scales to the requested CSS size", () => {
    const boxes = rangeBoxes(text, 0, 1, 1000, 1600);
    expect(boxes[0].left).toBeCloseTo(20);
    expect(boxes[0].width).toBeCloseTo(24);
  });

  it("clamps out-of-range indices and skips empty boxes", () => {
    expect(rangeBoxes(text, -5, 3, 500, 800).length).toBe(1);
    const empty = page("ab", [0, 0, 0, 0, 10, 10, 5, 5]);
    expect(rangeBoxes(empty, 0, 2, 100, 100).length).toBe(1);
  });
});

describe("snippetAround", () => {
  it("wraps trimmed snippets with ellipses when context exists", () => {
    const text = page("word ".repeat(30).trim(), []);
    const start = text.chars.indexOf("word", 100);
    const snippet = snippetAround(text, start, start + 4);
    expect(snippet.startsWith("…")).toBe(true);
    expect(snippet.endsWith("…")).toBe(true);
    expect(snippet).toContain("word");
  });

  it("has no ellipsis at document edges", () => {
    const text = page("one two three", []);
    const snippet = snippetAround(text, 0, 3);
    expect(snippet.startsWith("one")).toBe(true);
    expect(snippet.startsWith("…")).toBe(false);
  });
});
