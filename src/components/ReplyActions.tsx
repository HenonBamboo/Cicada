import type { Reply } from "../lib/types";

export function ReplyActions({
  result,
  busy,
  onConfirm,
  onCancel,
  onSubmit,
  onUndo,
  onSettings,
}: {
  result: Reply;
  busy: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  onSubmit: (text: string, selectionToken?: string) => void;
  onUndo?: () => void;
  onSettings?: () => void;
}) {
  return (
    <div className="reply-actions">
      {result.kind === "confirm" && (
        <>
          <button className="btn primary" disabled={busy} onClick={onConfirm}>
            确认
          </button>
          <button className="btn" disabled={busy} onClick={onCancel}>
            算了
          </button>
        </>
      )}
      {result.kind === "done" && onUndo && (
        <button className="btn ghost" disabled={busy} onClick={onUndo}>
          撤销
        </button>
      )}
      {result.kind === "unconfigured" && onSettings && (
        <button className="btn" disabled={busy} onClick={onSettings}>
          前往设置
        </button>
      )}
      {result.kind === "clarify" &&
        (result.entityOptions?.length
          ? result.entityOptions.map((option) => (
              <button
                key={option.value}
                className="btn"
                disabled={busy}
                onClick={() => onSubmit(option.label, option.value)}
              >
                {option.label}
              </button>
            ))
          : result.options.map((option) => (
              <button key={option} className="btn" disabled={busy} onClick={() => onSubmit(option)}>
                {option}
              </button>
            )))}
    </div>
  );
}
