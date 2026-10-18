import { invoke } from "@tauri-apps/api/core";
import type { TimeSettings } from "./types";
export interface Preferences {
  time: TimeSettings;
  automation: {
    weeklyReport: boolean;
    monthlyReport: boolean;
    yearlyReport: boolean;
    reviewEnabled: boolean;
    reviewTime: string;
  };
  desktop: {
    quickShortcut: string;
    mainShortcut: string;
    stickyShortcut: string;
    autostart: boolean;
    showStickiesOnStartup: boolean;
    closeToTray: boolean;
  };
  sticky: { color: string; opacity: number; autoHide: boolean };
}
export const getPreferences = () => invoke<Preferences>("get_preferences");
export const savePreferences = (preferences: Preferences) =>
  invoke<Preferences>("save_preferences", { preferences });
export interface PromptView {
  id: string;
  title: string;
  content: string;
  defaultContent: string;
  modified: boolean;
  defaultUpdated: boolean;
  unknownVariables: string[];
  variables: { name: string; description: string }[];
}
export const getPrompts = () => invoke<PromptView[]>("get_prompts");
export const savePrompt = (id: string, content: string) =>
  invoke<PromptView>("save_prompt", { id, content });
export const resetPrompt = (id: string) => invoke<PromptView>("reset_prompt", { id });
export const previewPrompt = (id: string) =>
  invoke<{ content: string; tools: string }>("preview_prompt", { id });
export const exportData = (format: "json" | "markdown") =>
  invoke<{ fileName: string; content: string; mimeType: string }>("export_data", { format });
export const importData = (content: string) =>
  invoke<{ tasks: number; nodes: number; completions: number; recurRules: number }>("import_data", {
    content,
  });
export interface OperationEntry {
  id: number;
  source: string;
  operation: string;
  entity: string;
  summary: string;
  createdAt: string;
  undoneAt: string | null;
}
export const listOperationLog = () => invoke<OperationEntry[]>("list_operation_log");
