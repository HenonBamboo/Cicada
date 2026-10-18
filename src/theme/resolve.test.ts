import { describe, expect, it } from "vitest";
import { applyAppearance, resolveTheme, type ThemeTarget } from "./resolve";
import type { Appearance } from "../lib/types";

const base: Appearance = {
  theme: "light",
  accent: null,
  fontSize: "medium",
  radius: "medium",
  motion: true,
};

function fakeTarget() {
  const props = new Map<string, string>();
  const target: ThemeTarget = {
    dataset: {},
    style: {
      setProperty: (k, v) => void props.set(k, v),
      removeProperty: (k) => void props.delete(k),
    },
  };
  return { target, props };
}

describe("resolveTheme", () => {
  it("跟随系统时取浅色或深色", () => {
    expect(resolveTheme("system", false, true)).toBe("light");
    expect(resolveTheme("system", true, true)).toBe("dark");
  });

  it("毛玻璃不支持时降级为浅色", () => {
    expect(resolveTheme("glass", false, true)).toBe("glass");
    expect(resolveTheme("glass", true, false)).toBe("light");
  });

  it("其他主题原样生效", () => {
    for (const t of ["light", "dark", "paper", "forest"] as const) {
      expect(resolveTheme(t, true, false)).toBe(t);
    }
  });
});

describe("applyAppearance", () => {
  it("写入 data 属性", () => {
    const { target } = fakeTarget();
    applyAppearance(
      target,
      { ...base, theme: "paper", fontSize: "large", motion: false },
      false,
      true,
    );
    expect(target.dataset).toMatchObject({
      theme: "paper",
      themeChoice: "paper",
      fontsize: "large",
      radius: "medium",
      motion: "off",
    });
  });

  it("自定义强调色覆盖主题默认值，清空后恢复", () => {
    const { target, props } = fakeTarget();
    applyAppearance(target, { ...base, accent: "#336699" }, false, true);
    expect(props.get("--accent")).toBe("#336699");
    applyAppearance(target, base, false, true);
    expect(props.has("--accent")).toBe(false);
  });
});
