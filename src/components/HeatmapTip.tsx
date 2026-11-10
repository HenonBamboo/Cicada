import { useEffect, useLayoutEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

/** 按视口定位，避免毛玻璃、变换和滚动容器影响热力图提示框。 */
export function HeatmapTip({
  anchor,
  children,
  className = "",
  onDismiss,
}: {
  anchor: HTMLElement;
  children: ReactNode;
  className?: string;
  onDismiss: () => void;
}) {
  const popup = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = popup.current;
    if (!element) return;
    const rect = anchor.getBoundingClientRect();
    const margin = 8;
    const viewportWidth = document.documentElement.clientWidth;
    const viewportHeight = document.documentElement.clientHeight;
    element.style.maxWidth = `${Math.max(0, viewportWidth - margin * 2)}px`;
    const { width, height } = element.getBoundingClientRect();
    const left = Math.max(
      margin,
      Math.min(rect.left + rect.width / 2 - width / 2, viewportWidth - width - margin),
    );
    const above = rect.top - height - margin;
    const top = above >= margin ? above : rect.bottom + margin;
    element.style.left = `${left}px`;
    element.style.top = `${Math.max(margin, Math.min(top, viewportHeight - height - margin))}px`;
    element.style.visibility = "visible";
  }, [anchor, children]);
  useEffect(() => {
    // 页面位置变化后收起旧提示，避免它留在已经移走的格子位置。
    document.addEventListener("scroll", onDismiss, true);
    window.addEventListener("resize", onDismiss);
    window.addEventListener("blur", onDismiss);
    return () => {
      document.removeEventListener("scroll", onDismiss, true);
      window.removeEventListener("resize", onDismiss);
      window.removeEventListener("blur", onDismiss);
    };
  }, [onDismiss]);
  return createPortal(
    <div
      ref={popup}
      className={`heat-tip popover ${className}`}
      role="tooltip"
      style={{ visibility: "hidden" }}
    >
      {children}
    </div>,
    document.body,
  );
}
