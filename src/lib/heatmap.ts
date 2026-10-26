import { addDays, diffDays, formatDate, parseDate, weekdayIndex } from "./dates";
import type { HeatDay } from "./types";

export const WEEKS = 53;

/** 回溯一年并包含今天；闰日回退到上一年二月的最后一天。 */
function heatYearStart(today: string): string {
  const date = parseDate(today);
  const target = new Date(Date.UTC(date.getUTCFullYear() - 1, date.getUTCMonth(), 1));
  const lastDay = new Date(
    Date.UTC(target.getUTCFullYear(), target.getUTCMonth() + 1, 0),
  ).getUTCDate();
  target.setUTCDate(Math.min(date.getUTCDate(), lastDay));
  return addDays(formatDate(target), 1);
}

export function heatCalendar(today: string) {
  const from = heatYearStart(today);
  const start = addDays(from, -weekdayIndex(from));
  const weeks = Math.ceil((diffDays(start, today) + 1) / 7);
  const dates = Array.from({ length: weeks * 7 }, (_, i) => addDays(start, i));
  const spans: { month: number; span: number }[] = [];
  for (let c = 0; c < weeks; c++) {
    const month = parseDate(dates[c * 7]).getUTCMonth();
    const last = spans[spans.length - 1];
    if (last?.month === month) last.span++;
    else spans.push({ month, span: 1 });
  }
  return {
    from,
    dates,
    weeks,
    months: spans.map((m) => ({ label: m.span >= 3 ? `${m.month + 1}月` : "", span: m.span })),
  };
}

export interface HeatCell {
  date: string;
  tasks: number;
  nodes: number;
  level: 0 | 1 | 2 | 3 | 4;
  future: boolean;
  outside: boolean;
  today: boolean;
}

export interface HeatGrid {
  /** 按列排列，每列一周（周一到周日），首尾补齐空格。 */
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
  const calendar = heatCalendar(today);
  const cells: HeatCell[] = [];
  for (const date of calendar.dates) {
    const hit = byDate.get(date);
    const tasks = hit?.tasks ?? 0;
    const nodes = hit?.nodes ?? 0;
    cells.push({
      date,
      tasks,
      nodes,
      level: level(tasks + nodes),
      future: date > today,
      outside: date < calendar.from || date > today,
      today: date === today,
    });
  }

  const total = cells.reduce((s, c) => s + (c.outside ? 0 : c.tasks + c.nodes), 0);

  // 连续天数：今天还没完成任何事时从昨天算起（今天还没过完）
  const count = (date: string) => (byDate.get(date)?.tasks ?? 0) + (byDate.get(date)?.nodes ?? 0);
  let date = count(today) > 0 ? today : addDays(today, -1);
  let streak = 0;
  while (count(date) > 0) {
    streak++;
    date = addDays(date, -1);
  }

  return {
    cells,
    months: calendar.months,
    total,
    streak,
  };
}
