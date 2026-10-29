import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { Check } from "../../components/Check";
import { Icon, type IconName } from "../../components/Icon";
import { onDataChanged } from "../../lib/commands";
import {
  closeSticky,
  getStickyBoard,
  getStickyState,
  onStickyChanged,
  openMain,
  setStickyTaskDone,
  startStickyDrag,
  updateSticky,
  type StickyBoard,
  type StickyColor,
  type StickyState,
} from "../../lib/desktop";
import { monthDay, timeOf } from "../../lib/dates";
import { canAdoptExample, isModeShortcut } from "../../lib/keys";
import { useInputNavigation } from "../../lib/useInputNavigation";
import { MarkdownInline, markdownToPlainText } from "../../lib/markdown";
import type { ProjectNode, TaskView } from "../../lib/types";
import { useCapture } from "../../lib/useCapture";
import { useDesktopAppearance } from "../../lib/useDesktopAppearance";
import { useUiCopy } from "../../lib/useUiCopy";
import { useInputExample } from "../../lib/useInputExample";
import { nodeDeadline, itemDescription } from "../../lib/nodePresentation";
import "./sticky.css";

const COLORS: [StickyColor, string][] = [
  ["amber", "黄"],
  ["pink", "粉"],
  ["blue", "蓝"],
  ["green", "绿"],
  ["theme", "跟随主题"],
];
function ProjectTree({ node }: { node: ProjectNode }) {
  return (
    <>
      <div className={`st-node l${node.depth}${node.done ? " done" : ""}`}>
        <span className="dot" />
        <span className="t">
          <span data-item-details={itemDescription(node)}>
            <MarkdownInline text={node.title} />
          </span>
        </span>
        {node.dueAt && <span className="st-node-due">{nodeDeadline(node.dueAt)}</span>}
        {node.progress && (
          <span className="p">
            {node.progress.done}/{node.progress.total}
          </span>
        )}
      </div>
      {node.children.map((child) => (
        <ProjectTree key={child.id} node={child} />
      ))}
    </>
  );
}
function Task({
  task,
  onToggle,
  flash,
}: {
  task: TaskView;
  onToggle: (task: TaskView) => void;
  flash: boolean;
}) {
  return (
    <div className={`st-task${task.done ? " is-done" : ""}${flash ? " flash" : ""}`}>
      <Check
        on={task.done}
        label={`${task.done ? "取消完成" : "完成"}「${markdownToPlainText(task.title)}」`}
        onToggle={() => onToggle(task)}
      />
      <div>
        <span className="t">
          <span data-item-details={itemDescription(task)}>
            <MarkdownInline text={task.title} />
          </span>
        </span>
        {!task.done && task.delayDays > 0 ? (
          <span className="meta warn">已延期 {task.delayDays} 天</span>
        ) : !task.done && task.remindAt ? (
          <span className="meta">{timeOf(task.remindAt)} 提醒</span>
        ) : null}
      </div>
    </div>
  );
}
export function StickyWindow() {
  useInputNavigation();
  const appearanceError = useDesktopAppearance();
  const [state, setState] = useState<StickyState | null>(null);
  const [board, setBoard] = useState<StickyBoard | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [menu, setMenu] = useState<"color" | "alpha" | null>(null);
  const [saving, setSaving] = useState(false);
  const [alpha, setAlpha] = useState(0.9);
  const [flash, setFlash] = useState<ReadonlySet<number>>(new Set());
  const flashTimer = useRef<number | undefined>(undefined);
  const capture = useCapture("sticky");
  const copy = useUiCopy();
  const example = useInputExample(
    copy,
    capture.mode,
    capture.busy || !!capture.text.length || !!capture.images.length,
  );
  const setCaptureMode = capture.setMode;
  const defaultModeApplied = useRef(false);
  const input = useRef<HTMLInputElement>(null);
  const reload = useCallback(async () => {
    try {
      const [next, data] = await Promise.all([getStickyState(), getStickyBoard()]);
      setState(next);
      setAlpha(next.opacity);
      setBoard(data);
      if (!defaultModeApplied.current) {
        defaultModeApplied.current = true;
        if (next.kind === "project") setCaptureMode("project");
      }
      setError(null);
    } catch (error) {
      setError(String(error));
    }
  }, [setCaptureMode]);
  useEffect(() => {
    const initialLoad = window.setTimeout(() => void reload(), 0);
    const off = onStickyChanged(() => void reload());
    const dataOff = onDataChanged((changes) => {
      void reload();
      setFlash(new Set(changes.tasks));
      window.clearTimeout(flashTimer.current);
      flashTimer.current = window.setTimeout(() => setFlash(new Set()), 1200);
    });
    const closeMenu = (event: PointerEvent) => {
      if (!(event.target as HTMLElement).closest(".st-pop,.st-tools")) setMenu(null);
    };
    document.addEventListener("pointerdown", closeMenu);
    return () => {
      void off.then((f) => f());
      void dataOff.then((f) => f());
      document.removeEventListener("pointerdown", closeMenu);
      window.clearTimeout(initialLoad);
      window.clearTimeout(flashTimer.current);
    };
  }, [reload]);
  const act = async (action: () => Promise<unknown>) => {
    try {
      setError(null);
      await action();
    } catch (error) {
      setError(String(error));
    }
  };
  const save = async (patch: Parameters<typeof updateSticky>[0]) => {
    setSaving(true);
    try {
      const next = await updateSticky(patch);
      setState(next);
      setAlpha(next.opacity);
      if (next.locked) setMenu(null);
    } catch (error) {
      setError(String(error));
      if (state) setAlpha(state.opacity);
    } finally {
      setSaving(false);
    }
  };
  const toggle = (task: TaskView) =>
    void act(async () => {
      const changes = await setStickyTaskDone(task.id, task.version, !task.done);
      await reload();
      if (changes.parentQuestions?.length) setError("项目已达到里程碑，打开主窗口继续安排。");
    });
  const tool = (icon: IconName, label: string, action: () => void, on = false) => (
    <button
      type="button"
      title={label}
      aria-label={label}
      className={on ? "on" : undefined}
      disabled={saving || !state}
      onClick={action}
    >
      <Icon name={icon} />
    </button>
  );
  const openTasks = board ? [...board.tasks.today, ...board.tasks.legacy] : [];
  return (
    <main
      className={`sticky c-${state?.color ?? "theme"}${menu ? " menu-open" : ""}${state?.locked ? " locked" : ""}`}
      style={{ "--alpha": alpha } as CSSProperties}
    >
      {state?.locked && (
        <span className="st-lock" title="当前便利贴已锁定，右键托盘可解锁">
          <Icon name="lock" />
        </span>
      )}
      <header
        className="st-head"
        onPointerDown={(event) => {
          if (event.button === 0 && !(event.target as HTMLElement).closest("button"))
            void act(startStickyDrag);
        }}
        onDoubleClick={(event) => {
          if (!(event.target as HTMLElement).closest("button")) void act(openMain);
        }}
      >
        <span className="st-title">
          {state?.kind === "project" ? (
            <span data-item-details={board?.project ? itemDescription(board.project) : undefined}>
              项目 · <MarkdownInline text={board?.project?.title ?? "项目"} />
            </span>
          ) : board ? (
            `今日 · ${monthDay(board.businessDate)} · 剩 ${openTasks.length} 件`
          ) : (
            "今日"
          )}
        </span>
        <div className="st-tools">
          {tool("pin", "置顶", () => void save({ pinned: !state?.pinned }), state?.pinned)}
          {tool("droplet", "颜色", () => setMenu(menu === "color" ? null : "color"))}
          {tool("sun", "透明度与贴边隐藏", () => setMenu(menu === "alpha" ? null : "alpha"))}
          {tool(
            "lock",
            "锁定当前便利贴（鼠标穿透，右键托盘可解锁）",
            () => void save({ locked: true }),
          )}
          {tool("window", "打开主窗口", () => void act(openMain))}
          {tool("x", "关闭便利贴", () => void act(closeSticky))}
        </div>
      </header>
      {menu === "color" && (
        <div className="st-pop">
          <div className="lbl">便利贴颜色</div>
          <div className="swatches">
            {COLORS.map(([color, label]) => (
              <button
                key={color}
                title={label}
                aria-label={label}
                className={`swatch-${color}${state?.color === color ? " on" : ""}`}
                disabled={saving}
                onClick={() => void save({ color })}
              />
            ))}
          </div>
        </div>
      )}
      {menu === "alpha" && (
        <div className="st-pop">
          <label className="lbl" htmlFor="sticky-opacity">
            透明度
          </label>
          <input
            id="sticky-opacity"
            type="range"
            min="55"
            max="100"
            value={Math.round(alpha * 100)}
            disabled={saving}
            onChange={(event) => setAlpha(Number(event.target.value) / 100)}
            onPointerUp={() => void save({ opacity: alpha })}
            onKeyUp={(event) => {
              if (
                ["ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"].includes(event.key)
              )
                void save({ opacity: alpha });
            }}
          />
          <label className="edge-option">
            <input
              type="checkbox"
              checked={state?.autoHide ?? false}
              disabled={saving}
              onChange={(event) => void save({ autoHide: event.target.checked })}
            />
            贴边隐藏
          </label>
        </div>
      )}
      <div className="st-body">
        {state?.kind === "project" ? (
          <div className="st-tree">
            {board?.project ? (
              board.project.children.map((node) => <ProjectTree key={node.id} node={node} />)
            ) : (
              <p className="st-empty">项目已不存在。</p>
            )}
          </div>
        ) : (
          <>
            {openTasks.map((task) => (
              <Task key={task.id} task={task} onToggle={toggle} flash={flash.has(task.id)} />
            ))}
            {!!board?.tasks.done.length && (
              <>
                <div className="st-sec">已完成 {board.tasks.done.length}</div>
                {board.tasks.done.map((task) => (
                  <Task key={task.id} task={task} onToggle={toggle} flash={flash.has(task.id)} />
                ))}
              </>
            )}
            {board && !openTasks.length && !board.tasks.done.length && (
              <p className="st-empty">{copy.taskEmpty}</p>
            )}
          </>
        )}
      </div>
      {state && (
        <>
          <div className="st-input">
            <Icon name="plus" />
            <button
              type="button"
              className="st-mode"
              title="Ctrl+Tab 切换任务/项目"
              aria-label={`当前${capture.mode === "task" ? "任务" : "项目"}模式，点击切换`}
              disabled={capture.busy}
              onClick={() => {
                capture.setMode(capture.mode === "task" ? "project" : "task");
                input.current?.focus();
              }}
            >
              {capture.mode === "task" ? "任务" : "项目"}
            </button>
            <input
              ref={input}
              aria-label={capture.mode === "task" ? "记录任务" : "记录项目"}
              title="Ctrl+Tab 切换任务/项目"
              placeholder={example.placeholder}
              value={capture.text}
              disabled={capture.busy}
              maxLength={20_000}
              onChange={(event) => capture.setText(event.target.value)}
              onPaste={capture.paste}
              onKeyDown={(event) => {
                if (event.nativeEvent.isComposing || capture.historyKeyDown(event)) return;
                if (isModeShortcut(event)) {
                  event.preventDefault();
                  capture.setMode(capture.mode === "task" ? "project" : "task");
                } else if (
                  canAdoptExample(
                    event,
                    capture.text,
                    capture.images.length,
                    capture.busy,
                    example.example,
                  )
                ) {
                  event.preventDefault();
                  capture.setText(example.example);
                } else if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                  event.preventDefault();
                  void capture.send();
                }
              }}
            />
          </div>
          {!!capture.images.length && (
            <button
              className="st-attachments"
              disabled={capture.busy}
              onClick={capture.clearImages}
            >
              已附 {capture.images.length} 张图片 · 移除
            </button>
          )}
        </>
      )}
      {(error || capture.error || appearanceError) && (
        <p className="st-status" role="status">
          {error || capture.error || appearanceError}
        </p>
      )}
    </main>
  );
}
