import { describe, expect, it } from "vitest";
import { settingsFromStorage } from "./settings";

const DEFAULTS = {
  mode: "paged" as const,
  theme: "day" as const,
  zoom: { kind: "fitWidth" } as const,
  spread: false,
};

describe("settingsFromStorage", () => {
  it("returns defaults for an empty store", () => {
    expect(settingsFromStorage({}, DEFAULTS)).toEqual(DEFAULTS);
  });

  it("hydrates valid stored values", () => {
    expect(
      settingsFromStorage(
        {
          readMode: '"scroll"',
          theme: '"night"',
          zoom: '{"kind":"percent","value":150}',
          spread: "true",
        },
        DEFAULTS,
      ),
    ).toEqual({
      mode: "scroll",
      theme: "night",
      zoom: { kind: "percent", value: 150 },
      spread: true,
    });
  });

  it("falls back per-key on invalid values", () => {
    const result = settingsFromStorage(
      { readMode: '"audiobook"', theme: '"sepia"', zoom: "not json", spread: '"yes"' },
      DEFAULTS,
    );
    expect(result).toEqual({ mode: "paged", theme: "sepia", zoom: { kind: "fitWidth" }, spread: false });
  });

  it("rejects out-of-range zoom percentages", () => {
    expect(settingsFromStorage({ zoom: '{"kind":"percent","value":5000}' }, DEFAULTS).zoom).toEqual(
      { kind: "fitWidth" },
    );
    expect(settingsFromStorage({ zoom: '{"kind":"percent","value":0}' }, DEFAULTS).zoom).toEqual({
      kind: "fitWidth",
    });
  });

  it("rejects malformed zoom kinds", () => {
    expect(settingsFromStorage({ zoom: '{"kind":"fitHeight"}' }, DEFAULTS).zoom).toEqual({
      kind: "fitWidth",
    });
  });
});
