import type { ReadMode, Theme, ZoomMode } from "./types";

export interface ReaderSettings {
  mode: ReadMode;
  theme: Theme;
  zoom: ZoomMode;
}

export function readModeFrom(v: unknown): ReadMode | null {
  return v === "paged" || v === "scroll" ? v : null;
}

export function themeFrom(v: unknown): Theme | null {
  return v === "day" || v === "sepia" || v === "night" ? v : null;
}

export function zoomFrom(v: unknown): ZoomMode | null {
  if (typeof v !== "object" || v === null) return null;
  const z = v as { kind?: unknown; value?: unknown };
  if (z.kind === "fitWidth" || z.kind === "fitPage") return { kind: z.kind };
  if (z.kind === "percent" && typeof z.value === "number" && z.value >= 25 && z.value <= 600) {
    return { kind: "percent", value: z.value };
  }
  return null;
}

/**
 * Hydrate reader settings from the persisted key-value store. Values are
 * JSON strings; anything missing or malformed falls back to defaults.
 */
export function settingsFromStorage(
  stored: Record<string, string | undefined>,
  defaults: ReaderSettings,
): ReaderSettings {
  const parse = <T>(raw: string | undefined, guard: (v: unknown) => T | null, fallback: T): T => {
    if (!raw) return fallback;
    try {
      return guard(JSON.parse(raw)) ?? fallback;
    } catch {
      return fallback;
    }
  };
  return {
    mode: parse(stored.readMode, readModeFrom, defaults.mode),
    theme: parse(stored.theme, themeFrom, defaults.theme),
    zoom: parse(stored.zoom, zoomFrom, defaults.zoom),
  };
}
