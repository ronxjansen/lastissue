import { describe, expect, it } from "vitest";
import { ago, displayName } from "./format";

describe("format", () => {
  it("names", () => {
    expect(displayName("Ada Lovelace <ada@example.com>")).toBe("Ada Lovelace");
    expect(displayName('"Bob" <bob@example.com>')).toBe("Bob");
    expect(displayName("<carol@example.com>")).toBe("carol");
    expect(displayName("dave@example.com")).toBe("dave");
  });

  it("relative times", () => {
    const now = Date.parse("2026-11-02T12:00:00Z");
    expect(ago(Date.parse("2026-11-02T11:59:30Z"), now)).toBe("now");
    expect(ago(Date.parse("2026-11-02T11:55:00Z"), now)).toBe("5m");
    expect(ago(Date.parse("2026-11-02T09:00:00Z"), now)).toBe("3h");
    expect(ago(Date.parse("2026-10-31T12:00:00Z"), now)).toBe("2d");
    expect(ago(Date.parse("2025-03-04T12:00:00Z"), now)).toBe("Mar 4, 2025");
  });
});
