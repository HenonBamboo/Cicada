import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { relativeDay, remindLabel } from "../../lib/dates";
import type { TaskBoard, TaskView } from "../../lib/types";
import { taskKey } from "../../lib/useBoard";
import { MarkdownBlock, MarkdownInline, markdownToPlainText } from "../../lib/markdown";
import { useUiCopy } from "../../lib/useUiCopy";
import { itemDescription } from "../../lib/nodePresentation";
import { Check } from "../Check";
import { Icon } from "../Icon";

const STALE_DAYS = 7;
/** 勾选后先播放对勾动画，再提交并移动到对应分区 */
const CHECK_DELAY_MS = 380;

function Meta({ t, today, legacy }: { t: TaskView; today: string; legacy?: boolean }) {
  const chips = [];
  if (legacy && t.delayDays > 0) {
    chips.push(
      <span key="delay" className={`chip ${t.delayDays >= STALE_DAYS ? "danger" : "amber"}`}>
        已延期 {t.delayDays} 天
      </span>,
    );
  }
  if (t.plannedDate > today) {
    chips.push(
      <span key="date" className="chip">
        <Icon name="calendar" />
        {relativeDay(today, t.plannedDate)}
      </span>,
    );
  }
  if (t.remindAt) {
    chips.push(
      <span key="remind" className="chip">
        <Icon name="bell" />
        {remindLabel(today, t.plannedDate, t.remindAt)}
      </span>,
    );
  }
  if (t.dueAt) {
    chips.push(
      <span key="due" className="chip amber">
        <Icon name="flag" />
        截止 {relativeDay(today, t.dueAt.slice(0, 10))}
      </span>,
    );
  }
  if (t.recur) {
    chips.push(
      <span key="recur" className="chip">
        <Icon name="repeat" />
        {t.recur}
      </span>,
    );
  }
  if (t.link) {
    chips.push(
      <span key="link" className="chip accent">
        <Icon name="link" />
        {t.link.path}
      </span>,
    );
  }
  return chips.length ? <div className="task-meta">{chips}</div> : null;
}

/** 被改动的条目滚动到可见位置，配合高亮让用户看到发生了什么 */
function useRevealOnFlash(flash: boolean) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (flash) ref.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [flash]);
  return ref;
}

const rowMotion = {
  layout: "position" as const,
  initial: { opacity: 0, y: 4 },
  animate: { opacity: 1, y: 0 },
  exit: { opacity: 0, x: -6, transition: { duration: 0.2 } },
  transition: { duration: 0.25, ease: [0.22, 1, 0.36, 1] as const },
};

function TaskRow({
  t,
  today,
  legacy,
  flash,
  onToggle,
}: {
  t: TaskView;
  today: string;
  legacy?: boolean;
  flash: boolean;
  onToggle: (t: TaskView, done: boolean) => void;
}) {
  // 本地先显示勾选状态，动画播完再提交
  const [pending, setPending] = useState<boolean | null>(null);
  const shown = pending ?? t.done;
  const ref = useRevealOnFlash(flash);
  const toggle = () => {
    if (pending !== null) return;
    const next = !t.done;
    setPending(next);
    window.setTimeout(() => {
      onToggle(t, next);
      setPending(null);
    }, CHECK_DELAY_MS);
  };
  return (
    <motion.div
      ref={ref}
      {...rowMotion}
      className={`task${shown ? " is-done" : ""}${flash ? " flash" : ""}`}
      data-item-details={itemDescription(t)}
    >
      <Check
        on={shown}
        label={
          shown
            ? `取消完成：${markdownToPlainText(t.title)}`
            : `标记完成：${markdownToPlainText(t.title)}`
        }
        onToggle={toggle}
      />
      <div className="task-main">
        <div className="task-title">
          <MarkdownInline text={t.title} />
        </div>
        <Meta t={t} today={today} legacy={legacy} />
        {t.note && (
          <div className="task-note">
            <MarkdownBlock text={t.note} />
          </div>
        )}
      </div>
    </motion.div>
  );
}

function UpcomingRow({ t, today, flash }: { t: TaskView; today: string; flash: boolean }) {
  const ref = useRevealOnFlash(flash);
  return (
    <motion.div
      ref={ref}
      {...rowMotion}
      className={`task upcoming${flash ? " flash" : ""}`}
      data-item-details={itemDescription(t)}
    >
      <span className="dot">
        <Icon name="clock" />
      </span>
      <div className="task-main">
        <div className="task-title">
          <MarkdownInline text={t.title} />
        </div>
        <Meta t={t} today={today} />
        {t.note && (
          <div className="task-note">
            <MarkdownBlock text={t.note} />
          </div>
        )}
      </div>
    </motion.div>
  );
}

function SectionHead({ label, n }: { label: string; n: number }) {
  return (
    <div className="sec">
      {label} <span className="n">{n}</span>
    </div>
  );
}

export function TaskPanel({
  tasks,
  today,
  flash,
  onToggle,
}: {
  tasks: TaskBoard;
  today: string;
  flash: ReadonlySet<string>;
  onToggle: (t: TaskView, done: boolean) => void;
}) {
  const [upcomingOpen, setUpcomingOpen] = useState(false);
  const copy = useUiCopy();
  // 新加到"之后"的任务需要被看见：有高亮条目时自动展开（默认折叠）
  if (!upcomingOpen && tasks.upcoming.some((t) => flash.has(taskKey(t.id)))) {
    setUpcomingOpen(true);
  }
  const total = tasks.today.length + tasks.legacy.length + tasks.done.length;
  const row = (t: TaskView, legacy?: boolean) => (
    <TaskRow
      key={t.id}
      t={t}
      today={today}
      legacy={legacy}
      flash={flash.has(taskKey(t.id))}
      onToggle={onToggle}
    />
  );

  return (
    <div className="card panel">
      <div className="panel-head">
        <span className="panel-title">任务</span>
        <span className="panel-meta">
          {tasks.done.length} / {total} 已完成
        </span>
      </div>
      <div className="panel-body">
        <SectionHead label="今日" n={tasks.today.length} />
        <AnimatePresence initial={false}>{tasks.today.map((t) => row(t))}</AnimatePresence>
        {tasks.today.length === 0 && (
          <div className="empty">{tasks.done.length > 0 ? "今天的事都办完了" : copy.taskEmpty}</div>
        )}

        {tasks.legacy.length > 0 && <SectionHead label="遗留" n={tasks.legacy.length} />}
        <AnimatePresence initial={false}>{tasks.legacy.map((t) => row(t, true))}</AnimatePresence>

        {tasks.done.length > 0 && <SectionHead label="已完成" n={tasks.done.length} />}
        <AnimatePresence initial={false}>{tasks.done.map((t) => row(t))}</AnimatePresence>

        {tasks.upcoming.length > 0 && (
          <button
            type="button"
            className={`sec${upcomingOpen ? "" : " collapsed"}`}
            aria-expanded={upcomingOpen}
            onClick={() => setUpcomingOpen((v) => !v)}
          >
            <Icon name="chevron-down" />
            之后 <span className="n">{tasks.upcoming.length}</span>
          </button>
        )}
        <AnimatePresence initial={false}>
          {upcomingOpen &&
            tasks.upcoming.map((t) => (
              <UpcomingRow key={t.id} t={t} today={today} flash={flash.has(taskKey(t.id))} />
            ))}
        </AnimatePresence>
      </div>
    </div>
  );
}
