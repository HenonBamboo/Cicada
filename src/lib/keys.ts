interface InputKey {
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  key: string;
}

/** Ctrl+Tab 切换模式；普通 Tab 采用示例，Shift+Tab 正向导航焦点。 */
export function isModeShortcut(e: InputKey): boolean {
  return e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey && e.key === "Tab";
}

export function isExampleShortcut(e: InputKey): boolean {
  return e.key === "Tab" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey;
}

export function isForwardFocusShortcut(e: InputKey): boolean {
  return e.key === "Tab" && e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey;
}

/** 输入框内的 Shift+Tab 按当前页面可用控件顺序向前移动。 */
export function moveFocusForward(current: Element): void {
  const doc = current.ownerDocument;
  const controls = [
    ...doc.querySelectorAll<HTMLElement>("button, a[href], input, select, textarea, [tabindex]"),
  ]
    .filter(
      (element) =>
        element.tabIndex >= 0 &&
        !element.matches(":disabled") &&
        !element.closest('[hidden], [inert], [aria-hidden="true"]') &&
        element.getClientRects().length > 0 &&
        doc.defaultView?.getComputedStyle(element).visibility === "visible",
    )
    .sort(
      (a, b) => (a.tabIndex || Number.MAX_SAFE_INTEGER) - (b.tabIndex || Number.MAX_SAFE_INTEGER),
    );
  const index = controls.findIndex((element) => element === current);
  controls[(index + 1) % controls.length]?.focus();
}

export function canAdoptExample(
  e: InputKey,
  text: string,
  imageCount: number,
  busy: boolean,
  example: string,
): boolean {
  return (
    isExampleShortcut(e) &&
    text.length === 0 &&
    imageCount === 0 &&
    !busy &&
    example.trim().length > 0
  );
}

export function exampleBody(value: string): string {
  return value.replace(/^\s*(?:比如|例如|示例)\s*[：:]\s*/, "").trim();
}
