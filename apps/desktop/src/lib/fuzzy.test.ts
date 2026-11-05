import { describe, expect, it } from "vitest";
import { rank, score, step } from "./fuzzy";

describe("fuzzy", () => {
  it("prefers prefix, then word start, then substring, then scattered", () => {
    expect(score("new", "New issue")).toBe(0);
    expect(score("iss", "New issue")).toBe(1);
    expect(score("ssu", "New issue")).toBe(7);
    expect(score("nis", "New issue")).toBe(105);
    expect(score("xyz", "New issue")).toBeNull();
    expect(score("  ", "anything")).toBe(0);
  });

  it("ranks stably", () => {
    expect(rank(["b issue", "issue a", "issue b"], "issue", (s) => s)).toEqual([
      "issue a",
      "issue b",
      "b issue",
    ]);
  });

  it("wraps the highlight", () => {
    expect(step(0, 3, -1)).toBe(2);
    expect(step(2, 3, 1)).toBe(0);
    expect(step(0, 0, 1)).toBe(0);
  });
});
