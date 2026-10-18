// 日期工具：业务日期一律是 "YYYY-MM-DD" 字符串，按 UTC 计算避免时区偏移。
// "今天"只能来自 Rust 返回的业务日期，这里不读取系统日期。

const DAY = 86_400_000;
const WEEK = "日一二三四五六";

export function parseDate(s: string): Date {
  const [y, m, d] = s.slice(0, 10).split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, d));
}

export function formatDate(d: Date): string {
  return d.toISOString().slice(0, 10);
}

export function addDays(s: string, n: number): string {
  return formatDate(new Date(parseDate(s).getTime() + n * DAY));
}

export function diffDays(from: string, to: string): number {
  return Math.round((parseDate(to).getTime() - parseDate(from).getTime()) / DAY);
}

/** 周一 = 0 */
export function weekdayIndex(s: string): number {
  return (parseDate(s).getUTCDay() + 6) % 7;
}

/** 10 月 17 日 */
export function monthDay(s: string): string {
  const d = parseDate(s);
  return `${d.getUTCMonth() + 1} 月 ${d.getUTCDate()} 日`;
}

/** 周五 */
export function weekdayName(s: string): string {
  return "周" + WEEK[parseDate(s).getUTCDay()];
}

/** 时刻字符串 "YYYY-MM-DDTHH:MM:SS" 中的 HH:MM */
export function timeOf(at: string): string {
  return at.slice(11, 16);
}

/** 相对今天的说法：今天 / 明天 / 后天 / 周三（一周内）/ 10 月 24 日 */
export function relativeDay(today: string, date: string): string {
  const n = diffDays(today, date);
  if (n === 0) return "今天";
  if (n === 1) return "明天";
  if (n === 2) return "后天";
  if (n > 2 && n < 7) return weekdayName(date);
  return monthDay(date);
}

/** 提醒时间：与计划日期或今天同一天时只显示时刻，否则带上日期 */
export function remindLabel(today: string, plannedDate: string, at: string): string {
  const date = at.slice(0, 10);
  return date === plannedDate || date === today
    ? timeOf(at)
    : `${relativeDay(today, date)} ${timeOf(at)}`;
}

/** 问候语的时间段（按本地钟点，只影响称呼，不参与业务判断） */
export function period(hour: number): string {
  if (hour < 5) return "夜深了";
  if (hour < 9) return "早上好";
  if (hour < 12) return "上午好";
  if (hour < 14) return "中午好";
  if (hour < 18) return "下午好";
  if (hour < 23) return "晚上好";
  return "夜深了";
}
