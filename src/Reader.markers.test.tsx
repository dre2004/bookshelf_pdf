// @vitest-environment jsdom
// Pure position math for the scrollbar search markers.
import { describe, expect, it } from "vitest";
import { markerFractions } from "./Reader";
import type { SearchMatch } from "./types";

describe("markerFractions", () => {
  const hits: SearchMatch[] = [
    { page: 0, start: 0, end: 3 },
    { page: 1, start: 0, end: 3 },
    { page: 2, start: 0, end: 3 },
  ];
  const pageMetrics = new Map([
    [0, { top: 0, height: 800 }],
    [1, { top: 800, height: 1000 }],
    // page 2 has no metrics (never measured): -1
  ]);
  const hitY = new Map([
    [0, 0.5], // middle of page 0
    [1, 0], // top of page 1
  ]);
  const fractions = markerFractions(hits, pageMetrics, hitY, 4000);

  it("places hits by page offset plus in-page fraction", () => {
    expect(fractions[0]).toBeCloseTo((0 + 0.5 * 800) / 4000); // 0.1
    expect(fractions[1]).toBeCloseTo((800 + 0) / 4000); // 0.2
  });

  it("returns -1 for unknown geometry", () => {
    expect(fractions[2]).toBe(-1);
    expect(markerFractions(hits, new Map(), new Map(), 0).every((f) => f === -1)).toBe(true);
  });
});
