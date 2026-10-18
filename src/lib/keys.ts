/** 输入框内按 Tab 切换任务/项目模式；Shift+Tab 保留给焦点导航 */
export function isModeShortcut(e: {
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  key: string;
}): boolean {
  return !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey && e.key === "Tab";
}
