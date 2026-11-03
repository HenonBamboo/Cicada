import { describe, expect, it } from "vitest";
import { isInboxRequest, needsInboxAttention } from "./inbox";
import type { InboxItem } from "./types";

describe("收件箱入口", () => {
  const item: InboxItem = {
    id: "example",
    mode: "task",
    content: "查询今天的任务",
    receivedAt: "2026-11-03T10:00:00",
    status: "queued",
    statusReason: null,
    lastError: null,
    resultSummary: null,
  };
  it("普通排队和处理中不提示，需介入时提示", () => {
    for (const status of ["queued", "processing", "done"] as const) {
      expect(needsInboxAttention({ ...item, status })).toBe(false);
    }
    for (const status of ["awaiting", "failed"] as const) {
      expect(needsInboxAttention({ ...item, status })).toBe(true);
    }
    for (const statusReason of ["offline", "unconfigured"]) {
      expect(needsInboxAttention({ ...item, statusReason })).toBe(true);
    }
  });
  it("明确查看收件箱在本地处理，不截取普通任务内容", () => {
    for (const text of ["收件箱", "查看收件箱", "收件箱还有什么？", "打开我的收件箱"])
      expect(isInboxRequest(text)).toBe(true);
    for (const text of ["明天整理收件箱", "删除收件箱里的所有内容", "查看收件箱并添加一个任务"])
      expect(isInboxRequest(text)).toBe(false);
  });
});
