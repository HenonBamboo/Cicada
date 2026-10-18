// 桌面辅助窗口通信：窗口身份与权限由 Rust 根据调用窗口判断。
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Appearance, ChangeSet, Mode, ProjectNode, TaskBoard } from "./types";

export type StickyColor = "theme" | "amber" | "pink" | "blue" | "green";
export interface StickyState {
  id: number;
  kind: "today" | "project";
  nodeId: number | null;
  color: StickyColor;
  opacity: number;
  pinned: boolean;
  locked: boolean;
  autoHide: boolean;
}
export interface StickyBoard {
  businessDate: string;
  tasks: TaskBoard;
  project: ProjectNode | null;
}
export const getDesktopBootstrap = () =>
  invoke<{ appearance: Appearance; glassSupported: boolean }>("get_desktop_bootstrap");
export const getStickyState = () => invoke<StickyState>("get_sticky_state");
export const getStickyBoard = () => invoke<StickyBoard>("get_sticky_board");
export const updateSticky = (
  patch: Partial<Pick<StickyState, "color" | "opacity" | "pinned" | "locked" | "autoHide">>,
) => invoke<StickyState>("update_sticky", { patch });
export const onStickyChanged = (handler: () => void) => listen("sticky-changed", handler);
export const closeSticky = () => invoke<void>("close_sticky");
export const openMain = () => invoke<void>("open_main");
export const hideQuick = () => invoke<void>("hide_quick_capture");
export const showQuick = () => invoke<void>("show_quick_capture");
export const createSticky = (kind: "today" | "project", nodeId?: number, hideMain = false) =>
  invoke<StickyState>("create_sticky", { kind, nodeId, hideMain });
export const setStickyTaskDone = (id: number, version: number, done: boolean) =>
  invoke<ChangeSet>("set_sticky_task_done", { id, version, done });
export const submitCapture = (text: string, mode: Mode, images: string[], requestId: string) =>
  invoke<string>("submit_capture", { text, mode, images, requestId });
export const desktopWindowLabel = () => getCurrentWindow().label;
export const onQuickOpened = (handler: () => void) => listen("quick-opened", handler);
export const onMainAction = (handler: (action: string) => void) =>
  listen<string>("main-action", (event) => handler(event.payload));
export const onDesktopError = (handler: (error: string) => void) =>
  listen<string>("desktop-error", (event) => handler(event.payload));
export const onDesktopMessage = (handler: (message: string) => void) =>
  listen<string>("desktop-message", (event) => handler(event.payload));
export const startStickyDrag = () => getCurrentWindow().startDragging();
export const onDesktopFocus = (handler: () => void) =>
  getCurrentWindow().onFocusChanged((event) => {
    if (event.payload) handler();
  });
