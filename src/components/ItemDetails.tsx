import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { MarkdownBlock } from "../lib/markdown";
import type { ItemDescription } from "../lib/nodePresentation";

interface Detail {
  anchor: HTMLElement;
  content: ItemDescription;
}

/** 共用悬停详情；使用 portal 避免列表裁剪，允许移入查看长备注。 */
export function ItemDetails() {
  const [detail, setDetail] = useState<Detail | null>(null);
  const active = useRef<HTMLElement | null>(null);
  const popup = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let hideTimer: number | undefined;
    const keep = () => window.clearTimeout(hideTimer);
    const hide = () => {
      keep();
      active.current = null;
      setDetail(null);
    };
    const scheduleHide = () => {
      keep();
      hideTimer = window.setTimeout(hide, 160);
    };
    const inPopup = (target: EventTarget | null) =>
      target instanceof Node && !!popup.current?.contains(target);
    const anchorFor = (target: EventTarget | null) =>
      target instanceof Element ? target.closest<HTMLElement>("[data-item-details]") : null;
    const show = (event: Event) => {
      if (inPopup(event.target)) {
        keep();
        return;
      }
      const anchor = anchorFor(event.target);
      if (!anchor || anchor.closest('[inert], [aria-hidden="true"]')) {
        scheduleHide();
        return;
      }
      keep();
      if (active.current === anchor) return;
      try {
        const content: ItemDescription = JSON.parse(anchor.dataset.itemDetails || "null");
        if (
          !content ||
          typeof content.title !== "string" ||
          !Array.isArray(content.lines) ||
          !content.lines.every((line) => typeof line === "string") ||
          (content.note !== null && typeof content.note !== "string")
        )
          return;
        active.current = anchor;
        setDetail({ anchor, content });
      } catch {
        hide();
      }
    };
    const leave = (event: PointerEvent | FocusEvent) => {
      if (
        event.relatedTarget instanceof Node &&
        (active.current?.contains(event.relatedTarget) || inPopup(event.relatedTarget))
      )
        return;
      scheduleHide();
    };
    const outside = (event: Event) => {
      if (!inPopup(event.target)) hide();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") hide();
    };
    document.addEventListener("pointerover", show);
    document.addEventListener("pointerout", leave);
    document.addEventListener("focusin", show);
    document.addEventListener("focusout", leave);
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", key);
    document.addEventListener("scroll", outside, true);
    window.addEventListener("resize", hide);
    window.addEventListener("blur", hide);
    return () => {
      document.removeEventListener("pointerover", show);
      document.removeEventListener("pointerout", leave);
      document.removeEventListener("focusin", show);
      document.removeEventListener("focusout", leave);
      keep();
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("keydown", key);
      document.removeEventListener("scroll", outside, true);
      window.removeEventListener("resize", hide);
      window.removeEventListener("blur", hide);
    };
  }, []);
  useEffect(() => {
    if (!detail) return;
    // 仅详情打开时观察页面切换和条目移除，不为隐藏页面保留 portal。
    let alive = true;
    const validate = () => {
      if (!alive || active.current !== detail.anchor) return;
      if (!detail.anchor.isConnected || detail.anchor.closest('[inert], [aria-hidden="true"]')) {
        active.current = null;
        setDetail(null);
      }
    };
    const observer = new MutationObserver(validate);
    observer.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["inert", "aria-hidden"],
    });
    // 条目可能在 pointerover 后、此次 effect 安装前已经被移除。
    queueMicrotask(validate);
    return () => {
      alive = false;
      observer.disconnect();
    };
  }, [detail]);
  useLayoutEffect(() => {
    if (!detail || !popup.current) return;
    const element = popup.current;
    const anchor = detail.anchor.getBoundingClientRect();
    const width = element.offsetWidth;
    const height = element.offsetHeight;
    const margin = 8;
    const left = Math.max(margin, Math.min(anchor.left, window.innerWidth - width - margin));
    const below = anchor.bottom + margin;
    const above = anchor.top - height - margin;
    const top = below + height <= window.innerHeight - margin ? below : above;
    element.style.left = `${left}px`;
    element.style.top = `${Math.max(margin, Math.min(top, window.innerHeight - height - margin))}px`;
    element.style.visibility = "visible";
  }, [detail]);
  if (!detail) return null;
  const { title, lines, note } = detail.content;
  return createPortal(
    <div
      ref={popup}
      className="item-details popover"
      role="tooltip"
      style={{ visibility: "hidden" }}
    >
      <strong className="item-details-title">{title}</strong>
      <div>{lines.join("\n")}</div>
      {note && (
        <div className="item-details-note">
          <MarkdownBlock text={note} />
        </div>
      )}
    </div>,
    document.body,
  );
}
