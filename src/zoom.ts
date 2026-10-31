import type { ZoomMode } from "./types";

export const ZOOM_STEPS = [50, 75, 100, 125, 150, 175, 200, 300, 400, 600];

export function zoomLabel(zoom: ZoomMode): string {
  switch (zoom.kind) {
    case "fitWidth":
      return "Fit width";
    case "fitPage":
      return "Fit page";
    case "percent":
      return `${zoom.value}%`;
  }
}

/** Fit modes sit at the 100% step so stepping up/down moves from there. */
export function stepZoom(zoom: ZoomMode, direction: 1 | -1): ZoomMode {
  const currentPercent = zoom.kind === "percent" ? zoom.value : 100;
  let index = ZOOM_STEPS.findIndex((s) => s >= currentPercent);
  if (index === -1) index = ZOOM_STEPS.length - 1;
  const next = Math.min(ZOOM_STEPS.length - 1, Math.max(0, index + direction));
  return { kind: "percent", value: ZOOM_STEPS[next] };
}
