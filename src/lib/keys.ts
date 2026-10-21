interface InputKey {
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  key: string;
}

/** 输入框内按 Ctrl+Tab 切换模式，其他 Tab 组合保留正常焦点导航。 */
export function isModeShortcut(e: InputKey): boolean {
  return e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey && e.key === "Tab";
}

export function canAdoptExample(
  e: InputKey,
  text: string,
  imageCount: number,
  busy: boolean,
  example: string,
): boolean {
  return (
    e.key === "Tab" &&
    !e.ctrlKey &&
    !e.altKey &&
    !e.shiftKey &&
    !e.metaKey &&
    text.length === 0 &&
    imageCount === 0 &&
    !busy &&
    example.trim().length > 0
  );
}

export function exampleBody(value: string): string {
  return value.replace(/^\s*(?:比如|例如|示例)\s*[：:]\s*/, "").trim();
}
