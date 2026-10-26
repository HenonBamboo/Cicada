import { describe, expect, it } from "vitest";
import { buildHeatGrid, heatCalendar, level, WEEKS } from "./heatmap";

describe("buildHeatGrid", () => {
  const today = "2026-10-17"; // 周六

  it("53 列，每列从周一开始，今天之后为空", () => {
    const g = buildHeatGrid(today, []);
    expect(g.cells).toHaveLength(WEEKS * 7);
    expect(g.cells[0].date).toBe("2025-10-13"); // 52 周前的周一
    const t = g.cells.findIndex((c) => c.today);
    expect(g.cells[t].date).toBe(today);
    expect(t % 7).toBe(5); // 周六在列内第 6 格
    expect(g.cells.slice(t + 1).every((c) => c.future)).toBe(true);
  });

  it("汇总、分级与月份", () => {
    const g = buildHeatGrid(today, [
      { date: "2026-10-17", tasks: 2, nodes: 1 },
      { date: "2026-10-16", tasks: 7, nodes: 0 },
    ]);
    expect(g.total).toBe(10);
    expect(g.cells.find((c) => c.date === "2026-10-16")?.level).toBe(4);
    expect(g.months.reduce((s, m) => s + m.span, 0)).toBe(WEEKS);
    expect(g.months.some((m) => m.label === "10月")).toBe(true);
  });

  it("连续天数：今天还没完成时从昨天算起", () => {
    const days = [
      { date: "2026-10-16", tasks: 1, nodes: 0 },
      { date: "2026-10-15", tasks: 1, nodes: 0 },
      { date: "2026-10-13", tasks: 1, nodes: 0 },
    ];
    expect(buildHeatGrid(today, days).streak).toBe(2);
    expect(buildHeatGrid(today, [...days, { date: today, tasks: 0, nodes: 1 }]).streak).toBe(3);
    expect(buildHeatGrid(today, []).streak).toBe(0);
  });

  it("分级阈值", () => {
    expect([0, 1, 2, 3, 4, 5, 6, 7].map(level)).toEqual([0, 1, 1, 2, 2, 3, 3, 4]);
  });

  it("仅统计近一年，补齐格和未来记录不计入合计", () => {
    expect(heatCalendar("2024-02-29").from).toBe("2023-03-01");
    expect(heatCalendar("2025-02-28").from).toBe("2024-02-29");
    const grid = buildHeatGrid(today, [
      { date: "2025-10-17", tasks: 10, nodes: 0 },
      { date: "2025-10-18", tasks: 1, nodes: 1 },
      { date: "2026-10-18", tasks: 10, nodes: 0 },
    ]);
    expect(grid.total).toBe(2);
    expect(grid.cells.find((cell) => cell.date === "2025-10-17")?.outside).toBe(true);
    const calendar = heatCalendar(today);
    expect(calendar.dates.length).toBe(calendar.weeks * 7);
    expect(calendar.months.reduce((sum, month) => sum + month.span, 0)).toBe(calendar.weeks);
  });
});
