import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

interface Detail {
  anchor: HTMLElement;
  content: string;
}

/** 共用悬停详情；使用 portal 避免任务面板和树视图裁剪，浮层不接收鼠标操作。 */
export function ItemDetails() {
  const [detail, setDetail] = useState<Detail | null>(null);
  const active = useRef<HTMLElement | null>(null);
  const popup = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const hide = () => {
      active.current = null;
      setDetail(null);
    };
    const anchorFor = (target: EventTarget | null) =>
      target instanceof Element ? target.closest<HTMLElement>("[data-item-details]") : null;
    const show = (event: Event) => {
      const anchor = anchorFor(event.target);
      if (!anchor) {
        hide();
        return;
      }
      if (active.current === anchor) return;
      const content = anchor.dataset.itemDetails;
      if (!content) return;
      active.current = anchor;
      setDetail({ anchor, content });
    };
    const leave = (event: PointerEvent | FocusEvent) => {
      if (event.relatedTarget instanceof Node && active.current?.contains(event.relatedTarget))
        return;
      hide();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") hide();
    };
    document.addEventListener("pointerover", show);
    document.addEventListener("pointerout", leave);
    document.addEventListener("focusin", show);
    document.addEventListener("focusout", leave);
    document.addEventListener("pointerdown", hide);
    document.addEventListener("keydown", key);
    document.addEventListener("scroll", hide, true);
    window.addEventListener("resize", hide);
    window.addEventListener("blur", hide);
    return () => {
      document.removeEventListener("pointerover", show);
      document.removeEventListener("pointerout", leave);
      document.removeEventListener("focusin", show);
      document.removeEventListener("focusout", leave);
      document.removeEventListener("pointerdown", hide);
      document.removeEventListener("keydown", key);
      document.removeEventListener("scroll", hide, true);
      window.removeEventListener("resize", hide);
      window.removeEventListener("blur", hide);
    };
  }, []);
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
  const [name, ...lines] = detail.content.split("\n");
  return createPortal(
    <div
      ref={popup}
      className="item-details popover"
      role="tooltip"
      style={{ visibility: "hidden" }}
    >
      <strong>{name}</strong>
      <div>{lines.join("\n")}</div>
    </div>,
    document.body,
  );
}
