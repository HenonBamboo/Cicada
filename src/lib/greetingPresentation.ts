import type { TaskBoard, TaskView } from "./types";

export interface GreetingPart {
  text: string;
  emphasis: boolean;
}

/** 只识别任务数量，不给日期、项目型号或普通句子中的数字着色。 */
export function greetingParts(text: string, tasks: TaskBoard): GreetingPart[] {
  const parts: GreetingPart[] = [];
  const quantities =
    /(?:^|[，。！？；：,\s])((?:今天|今日)?(?:还有|还剩|剩余|待办)|(?:今天|今日)(?:已经|已)?完成了?)\s*([0-9零〇一二两三四五六七八九十百千]+)(?=\s*(?:件(?:事|任务)?|项(?:任务|待办)|个(?:任务|待办)))/g;
  let end = 0;
  for (const match of text.matchAll(quantities)) {
    const start = match.index + match[0].length - match[2].length;
    parts.push({ text: text.slice(end, start), emphasis: false });
    const count = match[1].includes("完成")
      ? tasks.done.length
      : tasks.today.length + tasks.legacy.length;
    parts.push({ text: String(count), emphasis: true });
    end = start + match[2].length;
  }
  parts.push({ text: text.slice(end), emphasis: false });
  return parts;
}

/** 与后端一致：只看未完成的遗留任务，满七天提醒，优先最长延期。 */
export function longestDelayedTask(tasks: TaskBoard): TaskView | null {
  return tasks.legacy.reduce<TaskView | null>(
    (oldest, task) =>
      !task.done && task.delayDays >= 7 && (!oldest || task.delayDays > oldest.delayDays)
        ? task
        : oldest,
    null,
  );
}
