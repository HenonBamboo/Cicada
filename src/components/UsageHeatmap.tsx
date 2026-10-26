import { useMemo, useState } from "react";
import type { UsageDay } from "../lib/commands";
import { monthDay } from "../lib/dates";
import { heatCalendar } from "../lib/heatmap";

const number = new Intl.NumberFormat("zh-CN");

export function UsageHeatmap({ today, days }: { today: string; days: UsageDay[] }) {
  const [tip, setTip] = useState<{ day: UsageDay; x: number; y: number } | null>(null);
  const calendar = useMemo(() => heatCalendar(today), [today]);
  const byDate = useMemo(() => new Map(days.map((day) => [day.date, day])), [days]);
  const max = Math.max(
    1,
    ...days
      .filter((day) => day.date >= calendar.from && day.date <= today)
      .map((day) => day.totalTokens),
  );

  return (
    <div className="usage-heat">
      <div className="usage-heat-heading">每日用量 · 近一年</div>
      <div className="usage-heat-scroll" onMouseLeave={() => setTip(null)}>
        <div className="heat-months" aria-hidden="true">
          {calendar.months.map((month, i) => (
            <span key={i} style={{ gridColumn: `span ${month.span}` }}>
              {month.label}
            </span>
          ))}
        </div>
        <div className="heat-grid">
          {calendar.dates.map((date) => {
            if (date < calendar.from || date > today)
              return <span key={date} className="hc future" />;
            const day = byDate.get(date) ?? {
              date,
              inputTokens: 0,
              outputTokens: 0,
              totalTokens: 0,
              requests: 0,
              unknownRequests: 0,
            };
            const level =
              day.totalTokens > 0 ? Math.max(1, Math.ceil((day.totalTokens / max) * 4)) : 0;
            const show = (element: HTMLElement) => {
              const rect = element.getBoundingClientRect();
              setTip({ day, x: rect.left + rect.width / 2, y: rect.top });
            };
            return (
              <button
                key={date}
                type="button"
                className={`hc${day.unknownRequests ? " unknown" : ""}${date === today ? " today" : ""}`}
                data-l={level}
                tabIndex={date === today ? 0 : -1}
                aria-label={`${date}：已知 ${number.format(day.totalTokens)} Token，${day.requests} 次请求，${day.unknownRequests} 次用量未知`}
                onMouseEnter={(event) => show(event.currentTarget)}
                onFocus={(event) => show(event.currentTarget)}
                onBlur={() => setTip(null)}
                onClick={(event) => show(event.currentTarget)}
              />
            );
          })}
        </div>
      </div>
      <div className="heat-foot">
        少
        {[0, 1, 2, 3, 4].map((level) => (
          <span key={level} className="hc" data-l={level} />
        ))}
        多<span className="hc unknown" /> 含未知用量
      </div>
      {tip && (
        <div
          className="heat-tip popover usage-heat-tip"
          role="tooltip"
          style={{
            left: Math.min(Math.max(tip.x, 125), window.innerWidth - 125),
            top: tip.y - 8,
            transform: "translate(-50%, -100%)",
          }}
        >
          <div>
            {monthDay(tip.day.date)} · {number.format(tip.day.requests)} 次请求
          </div>
          <div>
            输入 {number.format(tip.day.inputTokens)} · 输出 {number.format(tip.day.outputTokens)}
          </div>
          <div>已知合计 {number.format(tip.day.totalTokens)} Token</div>
          {tip.day.unknownRequests > 0 && (
            <div>{number.format(tip.day.unknownRequests)} 次用量未知，未计入合计</div>
          )}
        </div>
      )}
    </div>
  );
}
