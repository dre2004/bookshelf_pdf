import { describe, expect, it } from "vitest";
import { sortVoices } from "./narration";

function voice(name: string, local: boolean): SpeechSynthesisVoice {
  return { name, localService: local } as SpeechSynthesisVoice;
}

describe("sortVoices", () => {
  it("puts local voices before remote ones", () => {
    const sorted = sortVoices([voice("Zira (cloud)", false), voice("David (local)", true)]);
    expect(sorted[0].name).toBe("David (local)");
  });

  it("sorts within each group by name", () => {
    const sorted = sortVoices([
      voice("Zeta", true),
      voice("Alpha", false),
      voice("Beta", true),
    ]);
    expect(sorted.map((v) => v.name)).toEqual(["Beta", "Zeta", "Alpha"]);
  });

  it("does not mutate the input list", () => {
    const input = [voice("B", true), voice("A", true)];
    sortVoices(input);
    expect(input[0].name).toBe("B");
  });
});
