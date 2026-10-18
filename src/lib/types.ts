// 与 Rust 端 config.rs / commands.rs 中的结构保持一致（serde 使用 camelCase）

export type ThemeChoice = "system" | "light" | "dark" | "paper" | "glass" | "forest";
export type ResolvedTheme = Exclude<ThemeChoice, "system">;
export type Scale = "small" | "medium" | "large";

export interface Appearance {
  theme: ThemeChoice;
  /** 自定义强调色，null 表示使用主题默认值 */
  accent: string | null;
  fontSize: Scale;
  radius: Scale;
  motion: boolean;
}

export interface TimeSettings {
  /** 日切时间，HH:MM */
  dayCutoff: string;
  weekStart: "monday" | "sunday";
}

export interface AppConfig {
  version: number;
  appearance: Appearance;
  time: TimeSettings;
}

export interface Bootstrap {
  onboardingNeeded: boolean;
  modelConfigured: boolean;
  desktopError?: string | null;
  config: AppConfig;
  dataDir: string;
  devBuild: boolean;
  businessDate: string;
  schemaVersion: number;
  tables: string[];
  glassSupported: boolean;
}

// ---- 主界面数据（与 Rust service/model.rs、agent/mod.rs 对应） ----

export type Mode = "task" | "project";

export interface TaskLink {
  nodeId: number;
  path: string;
}

export interface TaskView {
  id: number;
  title: string;
  plannedDate: string;
  done: boolean;
  delayDays: number;
  dueAt: string | null;
  remindAt: string | null;
  note: string | null;
  recur: string | null;
  link: TaskLink | null;
  version: number;
}

export interface TaskBoard {
  today: TaskView[];
  legacy: TaskView[];
  done: TaskView[];
  upcoming: TaskView[];
}

export interface Progress {
  done: number;
  total: number;
}

export interface ProjectNode {
  id: number;
  title: string;
  depth: number;
  done: boolean;
  projectStatus: "active" | "done" | "archived" | null;
  progress: Progress | null;
  children: ProjectNode[];
}

export interface Board {
  businessDate: string;
  tasks: TaskBoard;
  projects: ProjectNode[];
}

export interface HeatDay {
  date: string;
  tasks: number;
  nodes: number;
}

export interface DayCompletion {
  kind: "task" | "node";
  title: string;
  projectTitle: string | null;
}

export interface ChangeSet {
  tasks: number[];
  nodes: number[];
  parentQuestions?: string[];
}

export interface Reply {
  requestId: string;
  kind: "done" | "info" | "confirm" | "clarify" | "unconfigured" | "offline" | "error";
  status: string | null;
  lead: string;
  items: { icon: string; text: string; danger: boolean }[];
  options: string[];
  changes: ChangeSet;
}

export interface InboxItem {
  id: string;
  mode: Mode;
  content: string;
  receivedAt: string;
  status: "queued" | "processing" | "awaiting" | "done" | "failed";
  lastError: string | null;
  resultSummary: string | null;
}
export type Provider = "deepseek" | "qwen" | "kimi" | "zhipu" | "claude" | "custom";
export interface Endpoint {
  provider: Provider;
  baseUrl: string;
  model: string;
}
export interface ModelSettings {
  chat: Endpoint & { hasKey: boolean };
  vision: (Endpoint & { hasKey: boolean }) | null;
}
export interface BackupEntry {
  name: string;
  createdAt: string;
  kind: string;
  sizeBytes: number;
}
