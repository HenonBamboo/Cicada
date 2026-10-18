import { describe, expect, it } from "vitest";
import {
  addDays,
  diffDays,
  monthDay,
  period,
  relativeDay,
  remindLabel,
  weekdayIndex,
  weekdayName,
} from "./dates";

describe("dates", () => {
  it("跨月跨年加减天数", () => {
    expect(addDays("2026-10-31", 1)).toBe("2026-11-01");
    expect(addDays("2027-01-01", -1)).toBe("2026-12-31");
    expect(diffDays("2026-10-14", "2026-10-17")).toBe(3);
  });

  it("星期", () => {
    expect(weekdayName("2026-10-17")).toBe("周六");
    expect(weekdayIndex("2026-10-19")).toBe(0); // 周一
  });

  it("相对日期", () => {
    const t = "2026-10-17";
    expect(relativeDay(t, t)).toBe("今天");
    expect(relativeDay(t, "2026-10-18")).toBe("明天");
    expect(relativeDay(t, "2026-10-19")).toBe("后天");
    expect(relativeDay(t, "2026-10-21")).toBe("周三");
    expect(relativeDay(t, "2026-10-24")).toBe(monthDay("2026-10-24"));
  });

  it("提醒时间", () => {
    expect(remindLabel("2026-10-17", "2026-10-17", "2026-10-17T15:00:00")).toBe("15:00");
    expect(remindLabel("2026-10-17", "2026-10-20", "2026-10-20T09:00:00")).toBe("09:00");
    expect(remindLabel("2026-10-17", "2026-10-17", "2026-10-18T09:00:00")).toBe("明天 09:00");
  });

  it("问候时间段", () => {
    expect(period(3)).toBe("夜深了");
    expect(period(8)).toBe("早上好");
    expect(period(13)).toBe("中午好");
    expect(period(21)).toBe("晚上好");
  });
});
