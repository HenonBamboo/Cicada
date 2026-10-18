import { useEffect, useState } from "react";
import { getDesktopBootstrap } from "./desktop";
import { onConfigChanged } from "./commands";
import { applyAppearance } from "../theme/resolve";
import type { Appearance } from "./types";

/** 辅助窗口只读取外观，不读取主窗口配置或业务数据。 */
export function useDesktopAppearance() {
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    let appearance: Appearance | undefined;
    let glassSupported = false;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      if (alive && appearance)
        applyAppearance(document.documentElement, appearance, media.matches, glassSupported);
    };
    void getDesktopBootstrap()
      .then((value) => {
        appearance = value.appearance;
        glassSupported = value.glassSupported;
        apply();
      })
      .catch((error) => {
        if (alive) setError(String(error));
      });
    const off = onConfigChanged((config) => {
      appearance = config.appearance;
      apply();
    });
    media.addEventListener("change", apply);
    return () => {
      alive = false;
      media.removeEventListener("change", apply);
      void off.then((f) => f());
    };
  }, []);
  return error;
}
