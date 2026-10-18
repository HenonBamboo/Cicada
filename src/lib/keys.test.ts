import { describe, expect, it } from "vitest";
import { isModeShortcut } from "./keys";

const key = (
  k: string,
  mods: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {},
) => ({
  key: k,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe("isModeShortcut", () => {
  it("Tab 切换模式", () => {
    expect(isModeShortcut(key("Tab"))).toBe(true);
  });

  it("Shift+Tab、Ctrl+M 和其他组合不拦截", () => {
    expect(isModeShortcut(key("Tab", { shiftKey: true }))).toBe(false);
    expect(isModeShortcut(key("m"))).toBe(false);
    expect(isModeShortcut(key("m", { ctrlKey: true }))).toBe(false);
    expect(isModeShortcut(key("Tab", { ctrlKey: true }))).toBe(false);
  });
});
