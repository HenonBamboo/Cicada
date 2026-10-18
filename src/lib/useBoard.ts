import { useCallback, useEffect, useRef, useState } from "react";
import { getBoard, getHeatmap, onDataChanged } from "./commands";
import type { Board, ChangeSet, HeatDay } from "./types";

/** 高亮持续时间，与 .flash 动画一致 */
const FLASH_MS = 1300;

export const taskKey = (id: number) => `t:${id}`;
export const nodeKey = (id: number) => `n:${id}`;

/**
 * 主界面业务数据。只在内存中保存最近一次从 Rust 读到的结果，
 * 数据变更由 Rust 发 data-changed 事件后重新读取。
 */
export function useBoard() {
  const [board, setBoard] = useState<Board | null>(null);
  const [heat, setHeat] = useState<HeatDay[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [flash, setFlash] = useState<ReadonlySet<string>>(new Set());
  const timer = useRef<number | undefined>(undefined);

  const reload = useCallback(
    () =>
      Promise.all([getBoard(), getHeatmap()]).then(
        ([b, h]) => {
          setBoard(b);
          setHeat(h);
          setError(null);
        },
        (e: unknown) => setError(String(e)),
      ),
    [],
  );

  const highlight = useCallback((changes: ChangeSet) => {
    const keys = new Set([...changes.tasks.map(taskKey), ...changes.nodes.map(nodeKey)]);
    if (!keys.size) return;
    setFlash(keys);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setFlash(new Set()), FLASH_MS);
  }, []);

  useEffect(() => {
    void reload();
    const off = onDataChanged((changes) => {
      void reload().then(() => highlight(changes));
    });
    // 窗口重新获得焦点时刷新：业务日期可能已经变化
    const onFocus = () => void reload();
    window.addEventListener("focus", onFocus);
    return () => {
      off.then((f) => f());
      window.removeEventListener("focus", onFocus);
      window.clearTimeout(timer.current);
    };
  }, [reload, highlight]);

  return { board, heat, error, flash, reload };
}
