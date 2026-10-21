import { useEffect, useState } from "react";
import { getBootstrap, onConfigChanged, windowReady } from "../lib/commands";
import type { Bootstrap } from "../lib/types";
import { applyAppearance } from "./resolve";

const darkQuery = () => window.matchMedia("(prefers-color-scheme: dark)");

/** 读取启动信息，应用外观，并订阅 Rust 发出的配置变更事件（多窗口同步） */
export function useAppState() {
  const [state, setState] = useState<Bootstrap | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getBootstrap()
      .then(setState)
      .catch((e: unknown) => setError(String(e)));
    const off = onConfigChanged((config) => setState((s) => (s ? { ...s, config } : s)));
    return () => {
      off.then((f) => f());
    };
  }, []);

  // 外观应用后（或启动失败时）再显示窗口
  const bootstrapReady = state !== null;
  const ready = bootstrapReady || error !== null;
  useEffect(() => {
    if (!ready) return;
    const frame = requestAnimationFrame(() => void windowReady(bootstrapReady));
    return () => cancelAnimationFrame(frame);
  }, [ready, bootstrapReady]);

  useEffect(() => {
    if (!state) return;
    const q = darkQuery();
    const apply = () =>
      applyAppearance(
        document.documentElement,
        state.config.appearance,
        q.matches,
        state.glassSupported,
      );
    apply();
    q.addEventListener("change", apply);
    return () => q.removeEventListener("change", apply);
  }, [state]);

  return { state, error };
}
