import { ThemeMenu } from "../../components/ThemeMenu";
import { useCallback, useEffect, useRef, useState, type MouseEvent } from "react";
import { MotionConfig } from "motion/react";
import { Composer } from "../../components/Composer";
import { Greeting, Summary } from "../../components/Greeting";
import { Heatmap } from "../../components/Heatmap";
import { ProjectPanel } from "../../components/projects/ProjectPanel";
import { ReplyCard } from "../../components/ReplyCard";
import { TaskPanel } from "../../components/tasks/TaskPanel";
import { TitleBar } from "../../components/TitleBar";
import {
  completeOnboarding,
  getDayCompletions,
  getUpdate,
  installUpdate,
  onUpdateAvailable,
  listReports,
  readReport,
  setTaskDone,
  type UpdateView,
} from "../../lib/commands";
import { monthDay } from "../../lib/dates";
import type { Mode, TaskView } from "../../lib/types";
import { useBoard } from "../../lib/useBoard";
import { useAppState } from "../../theme/useAppState";
import { useAgent } from "../../lib/useAgent";
import { InboxList } from "../../components/InboxList";
import { Settings } from "../../components/Settings";
import { Icon } from "../../components/Icon";
import {
  createSticky,
  showQuick,
  onMainAction,
  onDesktopError,
  onDesktopMessage,
} from "../../lib/desktop";
import "./main.css";

export function MainWindow() {
  const { state: app, error: appError } = useAppState();
  const { board, heat, error, flash, reload } = useBoard();
  const [focused, setFocused] = useState(false);
  const [mode, setMode] = useState<Mode>("task");
  const agent = useAgent();
  const { reply, setReply, busy, showError } = agent;
  const [settings, setSettings] = useState(false);
  const [onboarding, setOnboarding] = useState(false);
  const [focusRequest, setFocusRequest] = useState(0);
  const [update, setUpdate] = useState<UpdateView | null>(null);
  const [updating, setUpdating] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const installing = useRef(false);
  const operationBusy = busy || updating;
  const onboardingStarted = useRef(false);
  const onboardingSaving = useRef(false);
  useEffect(() => {
    let alive = true;
    let eventReceived = false;
    const available = (value: UpdateView) => {
      eventReceived = true;
      if (alive) setUpdate(value);
    };
    const off = onUpdateAvailable(available);
    void getUpdate()
      .then((value) => {
        if (alive && !eventReceived) setUpdate(value);
      })
      .catch(() => {});
    return () => {
      alive = false;
      void off.then((stop) => stop());
    };
  }, []);
  const updateNow = async () => {
    if (installing.current || busy || onboardingSaving.current) return;
    installing.current = true;
    setUpdating(true);
    setUpdateError(null);
    try {
      await installUpdate();
    } catch (error) {
      installing.current = false;
      setUpdateError(String(error));
      setUpdating(false);
    }
  };
  useEffect(() => {
    if (app?.desktopError) showError(app.desktopError);
  }, [app?.desktopError, showError]);
  const inboxAction = useRef(agent.openInbox);
  const tryFirstMessage = useCallback(() => {
    setSettings(false);
    setMode("task");
    setFocused(true);
    setFocusRequest((n) => n + 1);
    setReply({
      kind: "message",
      lead: "试着说第一句话，比如：明天下午三点提醒我交周报。",
    });
  }, [setReply]);
  useEffect(() => {
    if (!app || onboardingStarted.current) return;
    onboardingStarted.current = true;
    if (!app.onboardingNeeded) return;
    const timer = window.setTimeout(() => {
      setOnboarding(true);
      if (app.modelConfigured) tryFirstMessage();
      else setSettings(true);
    }, 0);
    return () => window.clearTimeout(timer);
  }, [app, tryFirstMessage]);
  useEffect(() => {
    if (
      !onboarding ||
      onboardingSaving.current ||
      !agent.result ||
      !["done", "info"].includes(agent.result.kind)
    )
      return;
    onboardingSaving.current = true;
    void completeOnboarding()
      .then(() => setOnboarding(false))
      .catch((e: unknown) => {
        onboardingSaving.current = false;
        showError(e);
      });
  }, [onboarding, agent.result, showError]);
  useEffect(() => {
    inboxAction.current = agent.openInbox;
  }, [agent.openInbox]);
  useEffect(() => {
    const off = onMainAction((action) => {
      if (installing.current) return;
      if (action === "settings") setSettings(true);
      if (action === "inbox") {
        setSettings(false);
        setFocused(true);
        inboxAction.current();
      }
    });
    const errors = onDesktopError(showError);
    const messages = onDesktopMessage((lead) => {
      if (!installing.current) setReply({ kind: "message", lead });
    });
    return () => {
      void off.then((f) => f());
      void errors.then((f) => f());
      void messages.then((f) => f());
    };
  }, [showError, setReply]);
  const submit = (text: string, images: string[]) =>
    installing.current ? Promise.resolve(false) : agent.submit(text, mode, images);

  const toggleTask = useCallback(
    (t: TaskView, done: boolean) => {
      if (installing.current) return;
      setTaskDone(t.id, t.version, done)
        .then((changes) => {
          if (!busy && !installing.current && changes.parentQuestions?.length) {
            setReply({ kind: "message", lead: changes.parentQuestions.join("\n") });
          }
        })
        .catch((e: unknown) => {
          showError(e);
          void reload();
        });
    },
    [reload, showError, busy, setReply],
  );

  const pickDay = async (date: string) => {
    if (installing.current) return;
    try {
      const list = await getDayCompletions(date);
      if (installing.current) return;
      setReply({
        kind: "message",
        status: monthDay(date),
        lead: list.length ? `这一天完成了 ${list.length} 件事。` : "这一天没有完成记录。",
        items: list.map((c) => ({
          icon: "check",
          text: c.title,
          chip: c.kind === "node" ? (c.projectTitle ?? "项目") : undefined,
        })),
      });
    } catch (e) {
      showError(e);
    }
  };

  const showReports = async () => {
    if (operationBusy) return;
    try {
      const reports = await listReports();
      if (installing.current) return;
      const labels = { weekly: "周报", monthly: "月报", yearly: "年报" };
      setReply({
        kind: "message",
        status: "报告",
        lead: reports.length ? "这是最近生成的报告，点开可以查看全文。" : "暂时没有可查看的报告。",
        items: reports.map((r) => ({
          icon: "file",
          text: `${r.period} · ${labels[r.kind]}`,
          chip: `${monthDay(r.generatedAt.slice(0, 10))}生成`,
          onSelect: () => {
            if (installing.current) return;
            void readReport(r.id)
              .then((doc) => {
                if (installing.current) return;
                setReply({
                  kind: "message",
                  status: `${r.period} · ${labels[r.kind]}`,
                  lead: doc.markdown,
                });
              })
              .catch(showError);
          },
        })),
      });
    } catch (e) {
      showError(e);
    }
  };

  // Esc：先关回复，再退出聚焦态
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || installing.current) return;
      if (settings) setSettings(false);
      else if (busy) return;
      else if (reply) setReply(null);
      else if (focused) {
        setFocused(false);
        (document.activeElement as HTMLElement | null)?.blur();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [reply, focused, busy, settings, setReply]);

  // 点击空白处退出聚焦态；回复区有内容时不自动收起（3.2）
  const onStageDown = (e: MouseEvent) => {
    if (!focused || reply) return;
    if ((e.target as HTMLElement).closest(".composer, .card, .ex, button")) return;
    setFocused(false);
  };

  const motionOn = app?.config.appearance.motion ?? true;
  const err = appError ?? error;

  return (
    <MotionConfig reducedMotion={motionOn ? "user" : "always"}>
      <div
        className={`app${focused ? " focused" : ""}`}
        onClickCapture={(event) => {
          if (installing.current) {
            event.preventDefault();
            event.stopPropagation();
          }
        }}
      >
        <TitleBar>
          <button
            className="icon-btn"
            title="快速记录"
            aria-label="快速记录"
            disabled={updating}
            onClick={() => void showQuick().catch(showError)}
          >
            <Icon name="zap" />
          </button>
          <button
            className="icon-btn"
            title="收成便利贴"
            aria-label="收成便利贴"
            disabled={updating}
            onClick={() => void createSticky("today", undefined, true).catch(showError)}
          >
            <Icon name="sticky" />
          </button>
          {app && (
            <ThemeMenu appearance={app.config.appearance} glassSupported={app.glassSupported} />
          )}
          <button className="btn ghost" disabled={updating} onClick={() => setSettings(!settings)}>
            {settings ? "返回" : "设置"}
          </button>
        </TitleBar>
        {settings && (
          <Settings
            dataDir={app?.dataDir ?? ""}
            onBack={() => setSettings(false)}
            onboarding={onboarding}
            onModelSaved={onboarding ? tryFirstMessage : undefined}
          />
        )}
        <main
          className="stage"
          style={settings ? { display: "none" } : undefined}
          onMouseDown={onStageDown}
        >
          <section className="hero">
            {board ? (
              <Greeting date={board.businessDate} tasks={board.tasks} />
            ) : (
              <h1 className="greeting">知了</h1>
            )}

            <Composer
              mode={mode}
              onModeChange={setMode}
              onSubmit={submit}
              onFocus={() => setFocused(true)}
              busy={operationBusy}
              focusRequest={focusRequest}
            />

            {board && (
              <Summary
                tasks={board.tasks}
                activeProjects={board.projects.filter((p) => p.projectStatus === "active").length}
              />
            )}

            {agent.inbox.length > 0 && (
              <button
                className="btn ghost inbox-link"
                disabled={operationBusy}
                onClick={agent.openInbox}
              >
                收件箱 {agent.inbox.length} 条
              </button>
            )}

            {err && !reply && (
              <ReplyCard
                view={{ kind: "message", status: "没有完成", lead: err, tone: "danger" }}
                onClose={() => void reload()}
              />
            )}
            {update && !busy && !reply && (
              <ReplyCard
                view={{
                  kind: "message",
                  status: `发现新版本 ${update.version}`,
                  lead:
                    updateError ||
                    (updating
                      ? "正在下载并验证更新，完成后会重新启动。"
                      : update.notes || "有新版本可更新，确认后将替换程序并重新启动。"),
                  tone: updateError ? "danger" : undefined,
                }}
                onClose={() => {
                  if (!updating) setUpdate(null);
                }}
              >
                <div className="reply-actions">
                  <button
                    className="btn primary"
                    disabled={updating}
                    onClick={() => void updateNow()}
                  >
                    {updating ? "正在更新" : "更新"}
                  </button>
                  <button className="btn" disabled={updating} onClick={() => setUpdate(null)}>
                    稍后
                  </button>
                </div>
              </ReplyCard>
            )}
            {reply && (
              <ReplyCard view={reply} onClose={() => setReply(null)}>
                {agent.showInbox && (
                  <InboxList
                    items={agent.inbox}
                    busy={operationBusy}
                    onRetry={(item) => {
                      setMode(item.mode);
                      void agent.retry(item);
                    }}
                    onDiscard={(id) => void agent.discard(id)}
                  />
                )}
                {agent.result && (
                  <div className="reply-actions">
                    {agent.result.kind === "confirm" && (
                      <>
                        <button
                          className="btn primary"
                          disabled={operationBusy}
                          onClick={() => void agent.confirm()}
                        >
                          确认
                        </button>
                        <button
                          className="btn"
                          disabled={operationBusy}
                          onClick={() => void agent.cancel()}
                        >
                          算了
                        </button>
                      </>
                    )}
                    {agent.result.kind === "done" && (
                      <button
                        className="btn ghost"
                        disabled={operationBusy}
                        onClick={() => void agent.undo()}
                      >
                        撤销
                      </button>
                    )}
                    {agent.result.kind === "unconfigured" && (
                      <button className="btn" disabled={updating} onClick={() => setSettings(true)}>
                        前往设置
                      </button>
                    )}
                    {agent.result.kind === "clarify" &&
                      agent.result.options.map((option) => (
                        <button
                          key={option}
                          className="btn"
                          disabled={operationBusy}
                          onClick={() => void submit(option, [])}
                        >
                          {option}
                        </button>
                      ))}
                  </div>
                )}
              </ReplyCard>
            )}
          </section>

          {board && (
            <section className="board" aria-label="任务与项目" aria-hidden={!focused}>
              <Heatmap
                today={board.businessDate}
                days={heat}
                onPickDay={(d) => void pickDay(d)}
                onReports={() => void showReports()}
              />
              <div className="panels">
                <TaskPanel
                  tasks={board.tasks}
                  today={board.businessDate}
                  flash={flash}
                  onToggle={toggleTask}
                />
                <ProjectPanel projects={board.projects} flash={flash} />
              </div>
            </section>
          )}
        </main>
      </div>
    </MotionConfig>
  );
}
