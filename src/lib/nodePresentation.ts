import { monthDay, timeOf } from "./dates";
import { markdownToPlainText } from "./markdown";
import type { ProjectNode } from "./types";

export function nodeDeadline(at: string): string {
  return `${monthDay(at)}${at.length > 10 ? ` ${timeOf(at)}` : ""}`;
}

export function nodeDescription(node: ProjectNode): string {
  return [
    markdownToPlainText(node.title),
    node.note && markdownToPlainText(node.note),
    node.dueAt && `截止 ${node.dueAt.replace("T", " ")}`,
  ]
    .filter(Boolean)
    .join("\n");
}
