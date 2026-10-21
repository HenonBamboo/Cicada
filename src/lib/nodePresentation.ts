import { monthDay, timeOf } from "./dates";
import { markdownToPlainText } from "./markdown";
import type { ProjectNode } from "./types";

export interface ItemDescription {
  title: string;
  lines: string[];
  note: string | null;
}

export function nodeDeadline(at: string): string {
  return `${monthDay(at)}${at.length > 10 ? ` ${timeOf(at)}` : ""}`;
}

/** 名称与记录信息只用于悬停详情，不增加列表内容。 */
export function itemDescription(
  item: Pick<ProjectNode, "title" | "note" | "dueAt" | "createdAt" | "doneAt" | "done"> & {
    plannedDate?: string;
    remindAt?: string | null;
    recur?: string | null;
    link?: { path: string } | null;
  },
): string {
  const lines = [
    `录入时间：${item.createdAt.replace("T", " ")}`,
    `完成时间：${!item.done ? "未完成" : item.doneAt?.replace("T", " ") || "未记录"}`,
    item.plannedDate && `计划日期：${item.plannedDate}`,
    item.remindAt && `提醒时间：${item.remindAt.replace("T", " ")}`,
    item.dueAt && `截止时间：${item.dueAt.replace("T", " ")}`,
    item.recur && `重复：${item.recur}`,
    item.link && `关联项目：${item.link.path}`,
  ].filter((line): line is string => !!line);
  return JSON.stringify({
    title: markdownToPlainText(item.title),
    lines,
    note: item.note,
  } satisfies ItemDescription);
}
