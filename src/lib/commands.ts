// 与 Rust 通信的唯一入口：组件不直接调用 invoke / listen
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppConfig,
  Appearance,
  Board,
  Bootstrap,
  ChangeSet,
  DayCompletion,
  HeatDay,
  Mode,
  Reply,
  InboxItem,
  ModelSettings,
  Endpoint,
  BackupEntry,
} from "./types";

export const EVENT_CONFIG_CHANGED = "config-changed";
export const EVENT_DATA_CHANGED = "data-changed";

export function getBootstrap(): Promise<Bootstrap> {
  return invoke<Bootstrap>("get_bootstrap");
}

export function completeOnboarding(): Promise<void> {
  return invoke("complete_onboarding");
}

export function setAppearance(appearance: Appearance): Promise<AppConfig> {
  return invoke<AppConfig>("set_appearance", { appearance });
}

/** 首帧渲染完成后通知 Rust 显示窗口，避免启动白屏 */
export function windowReady(): Promise<void> {
  return invoke("window_ready");
}

export function getBoard(): Promise<Board> {
  return invoke<Board>("get_board");
}

export function getHeatmap(): Promise<HeatDay[]> {
  return invoke<HeatDay[]>("get_heatmap");
}

export function getDayCompletions(date: string): Promise<DayCompletion[]> {
  return invoke<DayCompletion[]>("get_day_completions", { date });
}

/** 勾选 / 取消勾选今日任务：界面唯一的直接写操作，经 Rust 校验并写入操作日志 */
export function setTaskDone(id: number, version: number, done: boolean): Promise<ChangeSet> {
  return invoke<ChangeSet>("set_task_done", { id, version, done });
}

/** 输入框提交，所有修改由 Rust 端处理 */
export function submitInput(
  text: string,
  mode: Mode,
  images: string[],
  requestId: string,
  replyTo?: string,
): Promise<Reply> {
  return invoke<Reply>("submit_input", { text, mode, images, requestId, replyTo });
}

export const confirmPending = (requestId: string) =>
  invoke<Reply>("confirm_pending", { requestId });
export const cancelPending = (requestId: string) => invoke<Reply>("cancel_pending", { requestId });
export const undoLast = () => invoke<Reply>("undo_last");
export const listInbox = () => invoke<InboxItem[]>("list_inbox");
export type InputSource = "main" | "quick" | "sticky";
export const listInputHistory = (mode: Mode, source: InputSource) =>
  invoke<string[]>("list_input_history", { mode, source });
export const retryInbox = (requestId: string) => invoke<Reply>("retry_inbox", { requestId });
export const discardInbox = (requestId: string) => invoke<void>("discard_inbox", { requestId });
export const onInboxChanged = (handler: () => void) => listen("inbox-changed", handler);
export const onAgentDelta = (handler: (delta: { requestId: string; text: string }) => void) =>
  listen<{ requestId: string; text: string }>("agent-delta", (e) => handler(e.payload));
export const getModelSettings = () => invoke<ModelSettings>("get_model_settings");
export const saveModelSettings = (input: {
  chat: Endpoint;
  vision: Endpoint | null;
  chatKey?: string;
  visionKey?: string;
}) => invoke<ModelSettings>("save_model_settings", { input });
export const clearApiKey = (endpoint: Endpoint) => invoke<void>("clear_api_key", { endpoint });
export const testModel = (target: "chat" | "vision") =>
  invoke<{ ok: boolean; latencyMs: number; toolCalls: boolean; message: string }>("test_model", {
    target,
  });
export const openFolder = (which: string) => invoke<void>("open_folder", { which });
export const listBackups = () => invoke<BackupEntry[]>("list_backups");
export const createBackup = () => invoke<BackupEntry>("create_backup");
export const restoreBackup = (name: string) =>
  invoke<{ needsApiKey: boolean }>("restore_backup", { name });

export interface ReportEntry {
  id: number;
  kind: "weekly" | "monthly" | "yearly";
  period: string;
  startDate: string;
  endDate: string;
  generatedAt: string;
}
export interface GreetingView {
  text: string;
  nudge: string | null;
  generated: boolean;
}
export const listReports = () => invoke<ReportEntry[]>("list_reports");
export const readReport = (id: number) =>
  invoke<{ entry: ReportEntry; markdown: string }>("read_report", { id });
export const getGreeting = () => invoke<GreetingView>("get_greeting");
export const onGreetingChanged = (handler: (greeting: GreetingView) => void) =>
  listen<GreetingView>("greeting-changed", (e) => handler(e.payload));

export interface UiCopyView {
  greeting: string;
  nudge: string | null;
  taskPlaceholder: string;
  projectPlaceholder: string;
  taskEmpty: string;
  projectEmpty: string;
  generated: boolean;
}
export const getUiCopy = () => invoke<UiCopyView>("get_ui_copy");
export const onUiCopyChanged = (handler: (copy: UiCopyView) => void) =>
  listen<UiCopyView>("ui-copy-changed", (e) => handler(e.payload));
export interface UpdateView {
  version: string;
  notes: string;
  publishedAt: string;
}
export const getUpdate = () => invoke<UpdateView | null>("get_update");
export const installUpdate = () => invoke<void>("install_update");
export const onUpdateAvailable = (handler: (update: UpdateView) => void) =>
  listen<UpdateView>("update-available", (e) => handler(e.payload));

export function onConfigChanged(handler: (config: AppConfig) => void): Promise<UnlistenFn> {
  return listen<AppConfig>(EVENT_CONFIG_CHANGED, (e) => handler(e.payload));
}

export function onDataChanged(handler: (changes: ChangeSet) => void): Promise<UnlistenFn> {
  return listen<ChangeSet>(EVENT_DATA_CHANGED, (e) => handler(e.payload));
}
