import { useEffect, useRef, type KeyboardEvent } from "react";
import { listInputHistory, onInboxChanged, type InputSource } from "./commands";
import {
  consecutiveUnique,
  historyDirection,
  stepHistory,
  type HistoryPosition,
} from "./inputHistory";
import type { Mode } from "./types";

export function useInputHistory(mode: Mode, source: InputSource) {
  const entries = useRef<string[]>([]);
  const position = useRef<HistoryPosition>({ index: -1, draft: "" });
  useEffect(() => {
    let alive = true;
    entries.current = [];
    position.current = { index: -1, draft: "" };
    const reload = () => {
      void listInputHistory(mode, source)
        .then((value) => {
          if (alive) entries.current = consecutiveUnique(value);
        })
        .catch(() => {});
    };
    reload();
    const off = onInboxChanged(reload);
    return () => {
      alive = false;
      void off.then((stop) => stop());
    };
  }, [mode, source]);

  const reset = () => {
    position.current = { index: -1, draft: "" };
  };
  const onKeyDown = (
    event: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>,
    text: string,
    setText: (text: string) => void,
  ) => {
    const input = event.currentTarget;
    const direction = historyDirection(
      { ...event, isComposing: event.nativeEvent.isComposing },
      text,
      input.selectionStart,
      input.selectionEnd,
      input.tagName === "TEXTAREA",
    );
    if (direction === null) return false;
    const next = stepHistory(entries.current, position.current, direction, text);
    if (!next) return false;
    event.preventDefault();
    position.current = next.position;
    setText(next.text);
    // React 提交受控值后将光标放在可继续浏览历史的一端。
    window.requestAnimationFrame(() => {
      const at = direction === 1 ? 0 : next.text.length;
      input.setSelectionRange(at, at);
    });
    return true;
  };
  return { onKeyDown, reset };
}
