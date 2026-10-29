import { describe, expect, it } from "vitest";
import { greetingParts, longestDelayedTask } from "./greetingPresentation";
import type { TaskBoard, TaskView } from "./types";

const task = (id: number, delayDays = 0, done = false): TaskView => ({
  id,
  delayDays,
  done,
  title: `任务${id}`,
  plannedDate: "2026-10-23",
  createdAt: "2026-10-23T09:00:00",
  doneAt: null,
  dueAt: null,
  remindAt: null,
  note: null,
  recur: null,
  link: null,
  version: 1,
});
const board: TaskBoard = {
  today: [task(1)],
  legacy: [task(2, 7), task(3, 9)],
  done: [task(4, 0, true)],
  upcoming: [],
};

describe("问候语重点", () => {
  it("按实时记录校正并强调中文或阿拉伯数字，保留其余语气", () => {
    for (const text of ["还有六件事等你翻牌。", "还有 6 件事等你翻牌。"]) {
      const parts = greetingParts(text, board);
      expect(parts.filter((p) => p.emphasis).map((p) => p.text)).toEqual(["3"]);
      expect(
        parts
          .map((p) => p.text)
          .join("")
          .replace(/ /g, ""),
      ).toBe("还有3件事等你翻牌。");
    }
    expect(greetingParts("今天已完成三件事，清单轻了。", board).filter((p) => p.emphasis)).toEqual([
      { text: "1", emphasis: true },
    ]);
  });

  it("不强调日期、型号、项目数量或刚完成一件的单次反馈", () => {
    for (const text of [
      "10月30日，CJG10项目有进展。",
      "今天完成了3个项目。",
      "昨天已完成3件事。",
      "明天还有3件事。",
      "「还有3件事」项目有了进展。",
      "又勾掉一件，清单轻了一点。",
      "新的一页，咱们慢慢写。",
    ]) {
      expect(greetingParts(text, board)).toEqual([{ text, emphasis: false }]);
    }
  });

  it("满七天提醒，选择最久延期，完成或移除后立即换成下一条", () => {
    expect(longestDelayedTask(board)?.id).toBe(3);
    expect(longestDelayedTask({ ...board, legacy: [task(2, 7), task(3, 9, true)] })?.id).toBe(2);
    expect(longestDelayedTask({ ...board, legacy: [task(2, 6)] })).toBeNull();
    expect(longestDelayedTask({ ...board, legacy: [] })).toBeNull();
  });
});
