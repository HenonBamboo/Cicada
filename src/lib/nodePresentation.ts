import { monthDay, timeOf } from "./dates";
import { markdownToPlainText } from "./markdown";
import type { ProjectNode } from "./types";

export function nodeDeadline(at: string): string {
  return `${monthDay(at)}${at.length > 10 ? ` ${timeOf(at)}` : ""}`;
}

/** 名称与记录信息只用于悬停详情，不增加列表内容。 */
export function itemDescription(
  item: Pick<ProjectNode, "title" | "note" | "dueAt" | "createdAt" | "doneAt" | "done">,
): string {
  return [
    markdownToPlainText(item.title),
    `录入时间：${item.createdAt.replace("T", " ")}`,
    `完成时间：${!item.done ? "未完成" : item.doneAt?.replace("T", " ") || "未记录"}`,
    item.note && `备注：${markdownToPlainText(item.note)}`,
    item.dueAt && `截止时间：${item.dueAt.replace("T", " ")}`,
  ]
    .filter(Boolean)
    .join("\n");
}
