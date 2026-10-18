import { addDays, parseDate, weekdayIndex } from "./dates";
import type { HeatDay } from "./types";

export const WEEKS = 53;

export interface HeatCell {
  date: string;
  tasks: number;
  nodes: number;
  level: 0 | 1 | 2 | 3 | 4;
  future: boolean;
  today: boolean;
}

export interface HeatGrid {
  /** 按列排列：每列一周（周一到周日），共 53 列，今天所在的周是最后一列 */
  cells: HeatCell[];
  months: { label: string; span: number }[];
  total: number;
  streak: number;
}

export function level(v: number): HeatCell["level"] {
  if (v === 0) return 0;
  if (v <= 2) return 1;
  if (v <= 4) return 2;
  if (v <= 6) return 3;
  return 4;
}

export function buildHeatGrid(today: string, days: HeatDay[]): HeatGrid {
  const byDate = new Map(days.map((d) => [d.date, d]));
  const start = addDays(today, -weekdayIndex(today) - (WEEKS - 1) * 7);
  const cells: HeatCell[] = [];
  for (let i = 0; i < WEEKS * 7; i++) {
    const date = addDays(start, i);
    const hit = byDate.get(date);
    const tasks = hit?.tasks ?? 0;
    const nodes = hit?.nodes ?? 0;
    cells.push({
      date,
      tasks,
      nodes,
      level: level(tasks + nodes),
      future: date > today,
      today: date === today,
    });
  }

  const spans: { month: number; span: number }[] = [];
  for (let c = 0; c < WEEKS; c++) {
    const month = parseDate(cells[c * 7].date).getUTCMonth();
    const last = spans[spans.length - 1];
    if (last && last.month === month) last.span++;
    else spans.push({ month, span: 1 });
  }

  const total = cells.reduce((s, c) => s + c.tasks + c.nodes, 0);

  // 连续天数：今天还没完成任何事时从昨天算起（今天还没过完）
  const todayIdx = cells.findIndex((c) => c.today);
  const count = (c: HeatCell) => c.tasks + c.nodes;
  let i = count(cells[todayIdx]) > 0 ? todayIdx : todayIdx - 1;
  let streak = 0;
  while (i >= 0 && count(cells[i]) > 0) {
    streak++;
    i--;
  }

  return {
    cells,
    months: spans.map((m) => ({ label: m.span >= 3 ? `${m.month + 1}月` : "", span: m.span })),
    total,
    streak,
  };
}
