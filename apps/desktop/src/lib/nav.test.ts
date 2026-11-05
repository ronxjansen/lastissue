import { describe, expect, it } from "vitest";
import { horizontal, toggle, vertical } from "./nav";

const cols = [["a", "b", "c"], [], ["d"]];

describe("nav", () => {
  it("starts at the first issue and clamps vertically", () => {
    expect(vertical(cols, null, 1)).toBe("a");
    expect(vertical(cols, "a", 1)).toBe("b");
    expect(vertical(cols, "c", 1)).toBe("c");
    expect(vertical(cols, "a", -1)).toBe("a");
    expect(vertical(cols, "gone", 1)).toBe("a");
  });

  it("skips empty columns and clamps the row", () => {
    expect(horizontal(cols, "c", 1)).toBe("d");
    expect(horizontal(cols, "d", -1)).toBe("a");
    expect(horizontal(cols, "d", 1)).toBe("d");
  });

  it("toggles in order", () => {
    expect(toggle(toggle(["a"], "b"), "a")).toEqual(["b"]);
  });
});
