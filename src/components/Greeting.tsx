import { useEffect, useState } from "react";
import { monthDay, period, weekdayName } from "../lib/dates";
import type { TaskBoard } from "../lib/types";
import { getGreeting, onGreetingChanged, type GreetingView } from "../lib/commands";

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
  const [greeting, setGreeting] = useState<GreetingView | null>(null);
  useEffect(() => {
    let alive = true;
    const update = (value: GreetingView) => {
      if (alive) setGreeting(value);
    };
    void getGreeting()
      .then(update)
      .catch(() => {});
    const unlisten = onGreetingChanged(update);
    return () => {
      alive = false;
      void unlisten.then((stop) => stop());
    };
  }, [date, hour, tasks]);
  const left = tasks.today.length + tasks.legacy.length;
  const stale = [...tasks.legacy, ...tasks.today].find((t) => t.delayDays > STALE_DAYS);
  const day = `${monthDay(date)} · ${weekdayName(date)}`;
  return (
    <>
      <h1 className="greeting">
        {greeting?.generated ? (
          greeting.text
        ) : (
          <>
            {period(hour)}，
            {left ? (
              <>
                今天还剩 <em>{left}</em> 件事
              </>
            ) : (
              "今天的事都办完了"
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
              「{stale.title}」已经拖了 {stale.delayDays} 天，还要做吗？
            </span>
          </>
        ) : left ? null : (
          " · 好好休息"
        )}
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
