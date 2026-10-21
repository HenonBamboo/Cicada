import { useEffect, useState } from "react";
import { monthDay, period, weekdayName } from "../lib/dates";
import type { TaskBoard } from "../lib/types";
import { useUiCopy } from "../lib/useUiCopy";
import { Icon } from "./Icon";

/** 长期延期的天数阈值（docs/产品设计.md 5.3） */
const STALE_DAYS = 7;

/** 每分钟更新一次钟点，让"早上好 / 晚上好"随时间变化 */
function useHour() {
  const [hour, setHour] = useState(() => new Date().getHours());
  useEffect(() => {
    const id = window.setInterval(() => setHour(new Date().getHours()), 60_000);
    return () => window.clearInterval(id);
  }, []);
  return hour;
}

/** 模型问候语沿用原有布局；离线时保留本地模板。 */
export function Greeting({ date, tasks }: { date: string; tasks: TaskBoard }) {
  const hour = useHour();
  const copy = useUiCopy();
  const left = tasks.today.length + tasks.legacy.length;
  const stale = [...tasks.legacy, ...tasks.today].find((t) => t.delayDays > STALE_DAYS);
  const day = `${monthDay(date)} · ${weekdayName(date)}`;
  return (
    <>
      <h1 className="greeting">
        <span className="greeting-icon" aria-hidden="true">
          <Icon name={hour >= 5 && hour < 18 ? "sun" : "moon"} />
        </span>
        {copy.greeting ? (
          copy.greeting
        ) : (
          <>
            {period(hour)}，
            {left ? (
              <>
                还有 <em>{left}</em> 件事，咱们一件件来。
              </>
            ) : (
              "今天的待办清单很清爽。"
            )}
          </>
        )}
      </h1>
      <p className="greeting-sub">
        {day}
        {copy.nudge ? (
          <>
            {" "}
            · <span className="nudge">{copy.nudge}</span>
          </>
        ) : stale ? (
          <>
            {" · "}
            <span className="nudge">「{stale.title}」还在清单里，要继续留着吗？</span>
          </>
        ) : null}
      </p>
    </>
  );
}

/** 静息态下的一行摘要 */
export function Summary({ tasks, activeProjects }: { tasks: TaskBoard; activeProjects: number }) {
  return (
    <div className="summary">
      <span>
        今日待办 <b>{tasks.today.length}</b>
      </span>
      <span>
        遗留 <b>{tasks.legacy.length}</b>
      </span>
      <span>
        已完成 <b>{tasks.done.length}</b>
      </span>
      <span>
        进行中的项目 <b>{activeProjects}</b>
      </span>
      <span className="dim">点击输入框展开</span>
    </div>
  );
}
