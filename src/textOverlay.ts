import type { PageTextData } from "./types";

export interface OverlayBox {
  /** CSS pixels relative to the page frame. */
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Geometry of a character range on a page, as line-merged boxes in CSS
 * pixels. Boxes are grouped into lines (by top edge) and merged left-to-right
 * so a highlight renders as a handful of rectangles rather than thousands.
 */
export function rangeBoxes(
  text: PageTextData,
  start: number,
  end: number,
  cssWidth: number,
  cssHeight: number,
): OverlayBox[] {
  const sx = cssWidth / text.width;
  const sy = cssHeight / text.height;
  const lines: { left: number; right: number; top: number; bottom: number }[] = [];

  for (let i = Math.max(0, start); i < Math.min(end, text.chars.length); i++) {
    const x = text.boxes[i * 4] ?? 0;
    const y = text.boxes[i * 4 + 1] ?? 0;
    const w = text.boxes[i * 4 + 2] ?? 0;
    const h = text.boxes[i * 4 + 3] ?? 0;
    if (w <= 0 || h <= 0) continue;

    // PDF origin is bottom-left; CSS origin is top-left.
    const left = x * sx;
    const right = (x + w) * sx;
    const top = (text.height - (y + h)) * sy;
    const bottom = (text.height - y) * sy;

    const line = lines.find((l) => Math.abs(l.top - top) < Math.max(2, (bottom - top) * 0.4));
    if (line) {
      line.left = Math.min(line.left, left);
      line.right = Math.max(line.right, right);
      line.top = Math.min(line.top, top);
      line.bottom = Math.max(line.bottom, bottom);
    } else {
      lines.push({ left, right, top, bottom });
    }
  }

  return lines.map((l) => ({
    left: l.left,
    top: l.top,
    width: l.right - l.left,
    height: l.bottom - l.top,
  }));
}

/** Snippet around a character range for search/highlight lists. */
export function snippetAround(text: PageTextData, start: number, end: number, radius = 24): string {
  const from = Math.max(0, start - radius);
  const to = Math.min(text.chars.length, end + radius);
  const raw = text.chars.slice(from, to).replace(/\s+/g, " ").trim();
  const prefix = from > 0 ? "…" : "";
  const suffix = to < text.chars.length ? "…" : "";
  return `${prefix}${raw}${suffix}`;
}
