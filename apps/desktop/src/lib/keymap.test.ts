import { describe, expect, it } from "vitest";
import { Resolver, merge, normalizeKeys, pretty, stroke } from "./keymap";

const ev = (key: string, mods: Partial<KeyboardEvent> = {}) => ({
  key,
  code: "",
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

describe("keymap", () => {
  it("turns key events into strokes", () => {
    expect(stroke(ev("k", { metaKey: true }))).toBe("cmd-k");
    expect(stroke(ev("H", { shiftKey: true }))).toBe("shift-h");
    expect(stroke(ev("?", { shiftKey: true }))).toBe("?");
    expect(stroke(ev("ArrowDown"))).toBe("down");
    expect(stroke(ev("Enter", { metaKey: true }))).toBe("cmd-enter");
    expect(stroke(ev("Shift", { shiftKey: true }))).toBeNull();
  });

  it("normalizes modifier order", () => {
    expect(normalizeKeys("Shift-Cmd-C")).toBe("cmd-shift-c");
    expect(normalizeKeys("g  i")).toBe("g i");
  });

  it("resolves by context specificity and sequences", () => {
    const { keymap } = merge({});
    const r = new Resolver(keymap);
    expect(r.feed("l", ["global", "workspace", "issues", "list"])).toBe("setLabels");
    expect(r.feed("l", ["global", "workspace", "issues", "board"])).toBe("columnNext");
    expect(r.feed("g", ["global", "workspace"])).toBe("pending");
    expect(r.feed("i", ["global", "workspace"])).toBe("goMyIssues");
    // A broken sequence falls back to the key alone.
    expect(r.feed("g", ["global", "workspace", "issues"])).toBe("pending");
    expect(r.feed("j", ["global", "workspace", "issues"])).toBe("cursorDown");
    // Only global bindings in a text input.
    expect(r.feed("c", ["global"])).toBeNull();
    expect(r.feed("cmd-k", ["global"])).toBe("openPalette");
  });

  it("applies user overrides and reports bad ones", () => {
    const { keymap, errors } = merge({
      "//": "a comment",
      issues: { x: null, "Shift-X": "toggleSelect", y: "nope", z: 3 },
      list: 1,
      nowhere: {},
    });
    expect(keymap.issues.x).toBeUndefined();
    expect(keymap.issues["shift-x"]).toBe("toggleSelect");
    expect(errors).toHaveLength(4);
  });

  it("pretty-prints", () => {
    expect(pretty("cmd-shift-c")).toBe("⌘⇧C");
    expect(pretty("g i")).toBe("g i");
    expect(pretty("cmd-backspace")).toBe("⌘⌫");
  });
});
