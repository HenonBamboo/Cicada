export interface HistoryKey {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  isComposing: boolean;
}

/** 多行编辑仅在整个文本的两端浏览历史，不抢占光标移动和输入法按键。 */
export function historyDirection(
  event: HistoryKey,
  text: string,
  selectionStart: number | null,
  selectionEnd: number | null,
  multiline: boolean,
): -1 | 1 | null {
  if (event.ctrlKey || event.altKey || event.shiftKey || event.metaKey || event.isComposing)
    return null;
  if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return null;
  if (selectionStart !== selectionEnd) return null;
  if (multiline) {
    if (event.key === "ArrowUp" && selectionStart !== 0) return null;
    if (event.key === "ArrowDown" && selectionEnd !== text.length) return null;
  }
  return event.key === "ArrowUp" ? 1 : -1;
}

export interface HistoryPosition {
  index: number;
  draft: string;
}

export function stepHistory(
  entries: string[],
  position: HistoryPosition,
  direction: -1 | 1,
  text: string,
): { position: HistoryPosition; text: string } | null {
  if (!entries.length || (direction === -1 && position.index < 0)) return null;
  const draft = position.index < 0 ? text : position.draft;
  const index = Math.max(-1, Math.min(entries.length - 1, position.index + direction));
  return { position: { index, draft }, text: index < 0 ? draft : entries[index] };
}

export function consecutiveUnique(entries: string[]): string[] {
  return entries
    .filter((entry, index) => entry.trim() && entry !== entries[index - 1])
    .slice(0, 50);
}
