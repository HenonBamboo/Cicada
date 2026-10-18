import { useEffect, useRef, useState } from "react";
import { setAppearance } from "../lib/commands";
import type { Appearance } from "../lib/types";
import { THEMES } from "../theme/resolve";
import { Icon } from "./Icon";

/** 标题栏上的主题切换入口。 */
export function ThemeMenu({
  appearance,
  glassSupported,
}: {
  appearance: Appearance;
  glassSupported: boolean;
}) {
  const [open, setOpen] = useState(false);
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!host.current?.contains(e.target as Node)) setOpen(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);

  return (
    <div ref={host} className="relative">
      <button
        type="button"
        className="icon-btn"
        title="切换主题"
        aria-label="切换主题"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <Icon name="sun" />
      </button>
      {open && (
        <div
          role="menu"
          className="popover absolute top-[calc(100%+6px)] right-0 z-50 min-w-[168px] p-1.5"
        >
          <div className="px-2 pt-1 pb-1.5 text-[0.79rem] text-text-3">主题</div>
          {THEMES.map((t) => {
            const on = appearance.theme === t.id;
            const degraded = t.id === "glass" && !glassSupported;
            return (
              <button
                key={t.id}
                type="button"
                role="menuitemradio"
                aria-checked={on}
                title={degraded ? "当前系统不支持毛玻璃，将显示为浅色" : undefined}
                onClick={() => {
                  void setAppearance({ ...appearance, theme: t.id });
                  setOpen(false);
                }}
                className="flex h-8 w-full items-center gap-2.5 rounded-sm border-0 bg-transparent px-2 text-left text-[0.93rem] hover:bg-surface-2"
              >
                {t.name}
                {degraded && <span className="text-[0.8rem] text-text-3">降级</span>}
                <span className={`ml-auto text-accent ${on ? "" : "invisible"}`}>
                  <Icon name="check" />
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
