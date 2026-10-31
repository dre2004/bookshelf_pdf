import { describe, expect, it } from "vitest";
import { ZOOM_STEPS, stepZoom, zoomLabel } from "./zoom";

describe("zoomLabel", () => {
  it("labels fit modes and percentages", () => {
    expect(zoomLabel({ kind: "fitWidth" })).toBe("Fit width");
    expect(zoomLabel({ kind: "fitPage" })).toBe("Fit page");
    expect(zoomLabel({ kind: "percent", value: 125 })).toBe("125%");
  });
});

describe("stepZoom", () => {
  it("steps up through the zoom ladder", () => {
    expect(stepZoom({ kind: "percent", value: 100 }, 1)).toEqual({ kind: "percent", value: 125 });
    expect(stepZoom({ kind: "percent", value: 125 }, 1)).toEqual({ kind: "percent", value: 150 });
  });

  it("steps down through the zoom ladder", () => {
    expect(stepZoom({ kind: "percent", value: 75 }, -1)).toEqual({ kind: "percent", value: 50 });
    expect(stepZoom({ kind: "percent", value: 150 }, -1)).toEqual({ kind: "percent", value: 125 });
  });

  it("treats fit modes as the 100% step", () => {
    expect(stepZoom({ kind: "fitWidth" }, 1)).toEqual({ kind: "percent", value: 125 });
    expect(stepZoom({ kind: "fitPage" }, -1)).toEqual({ kind: "percent", value: 75 });
  });

  it("clamps at both ends of the ladder", () => {
    const min = { kind: "percent", value: ZOOM_STEPS[0] } as const;
    const max = { kind: "percent", value: ZOOM_STEPS[ZOOM_STEPS.length - 1] } as const;
    expect(stepZoom(min, -1)).toEqual({ kind: "percent", value: ZOOM_STEPS[0] });
    expect(stepZoom(max, 1)).toEqual({ kind: "percent", value: ZOOM_STEPS[ZOOM_STEPS.length - 1] });
  });
});
