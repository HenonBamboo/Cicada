import { useEffect, useState } from "react";
import { monthDay, period, weekdayName } from "../lib/dates";
import type { TaskBoard } from "../lib/types";
import { useUiCopy } from "../lib/useUiCopy";
import { greetingParts, longestDelayedTask } from "../lib/greetingPresentation";
import { markdownToPlainText } from "../lib/markdown";
import { Icon } from "./Icon";

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
  const stale = longestDelayedTask(tasks);
  const day = `${monthDay(date)} · ${weekdayName(date)}`;
  return (
    <>
      <h1 className="greeting">
        <span className="greeting-icon" aria-hidden="true">
          <Icon name={hour >= 5 && hour < 18 ? "sun" : "moon"} />
        </span>
        {copy.greeting ? (
          greetingParts(copy.greeting, tasks).map((part, index) =>
            part.emphasis ? <em key={index}>{part.text}</em> : part.text,
          )
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
        {stale ? (
          <>
            {" · "}
            <span className="nudge">
              「{markdownToPlainText(stale.title)}」已延期 {stale.delayDays} 天，还要继续做吗？
            </span>
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
