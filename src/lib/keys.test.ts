import { describe, expect, it } from "vitest";
import { canAdoptExample, exampleBody, isModeShortcut } from "./keys";

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
  it("Ctrl+Tab 切换模式", () => {
    expect(isModeShortcut(key("Tab", { ctrlKey: true }))).toBe(true);
  });

  it("Shift+Tab、Ctrl+M 和其他组合不拦截", () => {
    expect(isModeShortcut(key("Tab", { shiftKey: true }))).toBe(false);
    expect(isModeShortcut(key("m"))).toBe(false);
    expect(isModeShortcut(key("m", { ctrlKey: true }))).toBe(false);
    expect(isModeShortcut(key("Tab"))).toBe(false);
  });
});

describe("采用示例", () => {
  it("只在空输入、无图片且非忙时拦截普通 Tab", () => {
    expect(canAdoptExample(key("Tab"), "", 0, false, "添加任务")).toBe(true);
    expect(canAdoptExample(key("Tab"), "草稿", 0, false, "添加任务")).toBe(false);
    expect(canAdoptExample(key("Tab"), "", 1, false, "添加任务")).toBe(false);
    expect(canAdoptExample(key("Tab"), "", 0, true, "添加任务")).toBe(false);
    expect(canAdoptExample(key("Tab", { shiftKey: true }), "", 0, false, "添加任务")).toBe(false);
    expect(canAdoptExample(key("Tab", { ctrlKey: true }), "", 0, false, "添加任务")).toBe(false);
    expect(canAdoptExample(key("Tab"), "", 0, false, "")).toBe(false);
  });
  it("填入正文时去掉展示前缀", () => {
    expect(exampleBody("比如：明天下午三点提醒我交周报")).toBe("明天下午三点提醒我交周报");
    expect(exampleBody("查看项目进度")).toBe("查看项目进度");
  });
});
