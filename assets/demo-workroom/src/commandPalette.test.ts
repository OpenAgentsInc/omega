import { describe, expect, it } from "vitest";
import { filterCommands, nextActiveIndex } from "./commandPalette";

describe("command palette", () => {
  it("wraps keyboard focus in both directions", () => {
    expect(nextActiveIndex(2, "next", 3)).toBe(0);
    expect(nextActiveIndex(0, "previous", 3)).toBe(2);
  });

  it("returns an honest empty state", () => {
    expect(nextActiveIndex(0, "next", 0)).toBe(-1);
  });

  it("matches labels and keywords", () => {
    const commands = [
      { id: "new", label: "New note", keywords: ["create"], run() {} },
      { id: "share", label: "Share note", keywords: ["invite"], run() {} },
    ];

    expect(filterCommands(commands, "invite").map(({ id }) => id)).toEqual([
      "share",
    ]);
  });
});
