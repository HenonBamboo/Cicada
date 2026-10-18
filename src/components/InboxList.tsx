import type { InboxItem } from "../lib/types";
const STATUS = {
  queued: "已暂存",
  processing: "处理中",
  awaiting: "待确认",
  failed: "失败",
  done: "已完成",
};
export function InboxList({
  items,
  busy,
  onRetry,
  onDiscard,
}: {
  items: InboxItem[];
  busy: boolean;
  onRetry: (item: InboxItem) => void;
  onDiscard: (id: string) => void;
}) {
  return (
    <div className="inbox-list">
      {items.map((item) => (
        <div className="inbox-item" key={item.id}>
          <div className="inbox-content">
            <span>{item.content || "图片输入"}</span>
            <small>
              {item.receivedAt} · {STATUS[item.status]}
            </small>
            {item.lastError && <small>{item.lastError}</small>}
          </div>
          <button
            className="btn"
            disabled={busy || item.status === "processing"}
            onClick={() => onRetry(item)}
          >
            {item.status === "awaiting" ? "继续" : "重试"}
          </button>
          <button
            className="btn ghost"
            disabled={busy || item.status === "processing"}
            onClick={() => onDiscard(item.id)}
          >
            放弃
          </button>
        </div>
      ))}
    </div>
  );
}
