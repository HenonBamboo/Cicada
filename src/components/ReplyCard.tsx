import type { ReactNode } from "react";
import { MarkdownBlock, MarkdownInline } from "../lib/markdown";
import { Icon, type IconName } from "./Icon";

export interface ReplyItem {
  icon: IconName;
  text: string;
  chip?: string;
  danger?: boolean;
  onSelect?: () => void;
}
export type ReplyView =
  | { kind: "thinking" }
  | {
      kind: "message";
      status?: string;
      lead: string;
      items?: ReplyItem[];
      tone?: "danger";
      streaming?: boolean;
    };

export function ReplyCard({
  view,
  onClose,
  children,
}: {
  view: ReplyView;
  onClose: () => void;
  children?: ReactNode;
}) {
  return (
    <div
      className={`card reply${view.kind === "message" && view.tone === "danger" ? " danger" : ""}`}
      role="status"
    >
      <div className="reply-head">
        <img src="/logo.svg" alt="" />
        知了{view.kind === "thinking" ? " · 正在整理" : view.status ? ` · ${view.status}` : ""}
        {view.kind === "thinking" ? (
          <span className="typing" aria-hidden="true">
            <i />
            <i />
            <i />
          </span>
        ) : (
          <button
            type="button"
            className="icon-btn"
            title="关闭（Esc）"
            aria-label="关闭回复"
            onClick={onClose}
          >
            <Icon name="x" />
          </button>
        )}
      </div>
      {view.kind === "message" && (
        <div className="reply-body">
          {view.streaming ? (
            <p className="caret reply-stream">{view.lead.replace(/^[\s\u200B\uFEFF]+/, "")}</p>
          ) : (
            <MarkdownBlock text={view.lead.replace(/^[\s\u200B\uFEFF]+/, "")} />
          )}
          {!!view.items?.length && (
            <ul className="reply-list">
              {view.items.map((it, i) => (
                <li key={i} className={it.danger ? "del" : undefined}>
                  <Icon name={it.icon} />
                  <span>
                    {it.onSelect ? (
                      <button type="button" className="report-link" onClick={it.onSelect}>
                        <MarkdownInline text={it.text} />
                      </button>
                    ) : (
                      <MarkdownInline text={it.text} />
                    )}
                    {it.chip && <span className="chip">{it.chip}</span>}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      {children}
    </div>
  );
}
