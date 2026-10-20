import { describe, expect, it } from "vitest";
import { consecutiveUnique, historyDirection, stepHistory } from "./inputHistory";

const key = (key: string, patch = {}) => ({
  key,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  isComposing: false,
  ...patch,
});

describe("输入历史", () => {
  it("多行仅在文本两端切换，组合键、选择区和输入法不拦截", () => {
    const text = "第一行\n第二行";
    expect(historyDirection(key("ArrowUp"), text, 0, 0, true)).toBe(1);
    expect(historyDirection(key("ArrowUp"), text, 4, 4, true)).toBeNull();
    expect(historyDirection(key("ArrowDown"), text, text.length, text.length, true)).toBe(-1);
    expect(historyDirection(key("ArrowUp", { isComposing: true }), text, 0, 0, false)).toBeNull();
    expect(historyDirection(key("ArrowUp", { shiftKey: true }), text, 0, 0, false)).toBeNull();
    expect(historyDirection(key("ArrowUp"), text, 0, 2, false)).toBeNull();
  });
  it("浏览后恢复未发送草稿，并只去除连续重复记录", () => {
    const entries = consecutiveUnique(["最新", "最新", "更早", "最新"]);
    expect(entries).toEqual(["最新", "更早", "最新"]);
    const first = stepHistory(entries, { index: -1, draft: "" }, 1, "未发草稿")!;
    const older = stepHistory(entries, first.position, 1, first.text)!;
    expect(older.text).toBe("更早");
    const recent = stepHistory(entries, older.position, -1, older.text)!;
    expect(stepHistory(entries, recent.position, -1, recent.text)?.text).toBe("未发草稿");
  });
});
