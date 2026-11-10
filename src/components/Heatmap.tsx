import { useMemo, useState } from "react";
import { monthDay } from "../lib/dates";
import { buildHeatGrid, type HeatCell } from "../lib/heatmap";
import type { HeatDay } from "../lib/types";
import { Icon } from "./Icon";
import { HeatmapTip } from "./HeatmapTip";

interface Tip {
  cell: HeatCell;
  anchor: HTMLElement;
}

/** 完成热力图（docs/产品设计.md 第 6 节）：悬停看数字，点击在回复区看当天清单 */
export function Heatmap({
  today,
  days,
  onPickDay,
  onReports,
}: {
  today: string;
  days: HeatDay[];
  onPickDay: (date: string) => void;
  onReports: () => void;
}) {
  const grid = useMemo(() => buildHeatGrid(today, days), [today, days]);
  const [tip, setTip] = useState<Tip | null>(null);

  return (
    <div className="card heat">
      <div className="heat-side">
        <div>
          <div className="big">{grid.total}</div>
          <div className="cap">
            近一年完成的事 · 连续 <b>{grid.streak}</b> 天
          </div>
        </div>
        <div className="heat-foot">
          少
          {[0, 1, 2, 3, 4].map((l) => (
            <span key={l} className="hc" data-l={l} />
          ))}
          多
          <button type="button" className="btn" onClick={onReports}>
            <Icon name="file" /> 报告
          </button>
        </div>
      </div>
      <div className="heat-main">
        <div className="heat-months" aria-hidden="true">
          {grid.months.map((m, i) => (
            <span key={i} style={{ gridColumn: `span ${m.span}` }}>
              {m.label}
            </span>
          ))}
        </div>
        <div className="heat-grid" onMouseLeave={() => setTip(null)}>
          {grid.cells.map((c) =>
            c.outside ? (
              <span key={c.date} className="hc future" />
            ) : (
              <button
                key={c.date}
                type="button"
                tabIndex={c.today ? 0 : -1}
                className={`hc${c.today ? " today" : ""}`}
                data-l={c.level}
                aria-label={`${monthDay(c.date)}：任务 ${c.tasks}，项目 ${c.nodes}`}
                onMouseEnter={(e) => {
                  setTip({ cell: c, anchor: e.currentTarget });
                }}
                onFocus={(e) => {
                  setTip({ cell: c, anchor: e.currentTarget });
                }}
                onBlur={() => setTip(null)}
                onClick={() => onPickDay(c.date)}
              />
            ),
          )}
        </div>
      </div>
      {tip && (
        <HeatmapTip anchor={tip.anchor} onDismiss={() => setTip(null)}>
          {monthDay(tip.cell.date)} · 任务 <span className="n">{tip.cell.tasks}</span> · 项目{" "}
          <span className="n">{tip.cell.nodes}</span>
        </HeatmapTip>
      )}
    </div>
  );
}
