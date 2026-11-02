import { describe, expect, it } from "vitest";
import { followScrollTarget } from "./Reader";

describe("followScrollTarget", () => {
  it("keeps a comfortably visible line untouched", () => {
    // Viewport 600, comfort band ends at 70% (420); line at 100..130.
    expect(followScrollTarget(0, 600, 100, 130)).toBeNull();
  });

  it("scrolls a line that dropped below the band to ~40% from the top", () => {
    // Band ends at 420; the line's bottom (500) is past it.
    // Target: 450 - 600 * 0.4 = 210.
    expect(followScrollTarget(0, 600, 450, 500)).toBe(210);
  });

  it("never scrolls backwards even when the anchor math asks for it", () => {
    // Line at 700..1000 with scrollTop 500: the 40% anchor would be
    // 700 - 240 = 460, below the current position — clamped to 500.
    expect(followScrollTarget(500, 600, 700, 1000)).toBe(500);
  });

  it("ignores degenerate viewports", () => {
    expect(followScrollTarget(0, 0, 100, 200)).toBeNull();
  });
});
