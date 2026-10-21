import { useEffect, useRef, useState } from "react";
import { saveExport, importData, listOperationLog, type OperationEntry } from "../lib/settings";
import { onDataChanged } from "../lib/commands";
import { Row } from "./SettingsControls";
export function DataExchange({
  report,
  onImported,
}: {
  report: (s: string) => void;
  onImported: () => Promise<unknown>;
}) {
  const file = useRef<HTMLInputElement>(null),
    [pending, setPending] = useState<{ name: string; content: string } | null>(null),
    [busy, setBusy] = useState(false),
    [message, setMessage] = useState(""),
    [logs, setLogs] = useState<OperationEntry[]>([]);
  const refresh = () => listOperationLog().then(setLogs);
  useEffect(() => {
    void refresh().catch((e) => report(String(e)));
    const off = onDataChanged(() => void refresh().catch((e) => report(String(e))));
    return () => {
      void off.then((f) => f());
    };
  }, [report]);
  const act = async (fn: () => Promise<void>) => {
    setBusy(true);
    setMessage("");
    try {
      await fn();
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  };
  const download = (format: "json" | "markdown") =>
    act(async () => {
      const path = await saveExport(format);
      if (path) setMessage(`已导出到 ${path}`);
    });
  return (
    <>
      <Row label="导出与导入">
        <div className="reply-actions">
          <button className="btn" disabled={busy} onClick={() => void download("markdown")}>
            导出 Markdown
          </button>
          <button className="btn" disabled={busy} onClick={() => void download("json")}>
            导出 JSON
          </button>
          <button className="btn" disabled={busy} onClick={() => file.current?.click()}>
            导入
          </button>
          <input
            ref={file}
            hidden
            type="file"
            accept=".json,application/json"
            onChange={(e) => {
              const picked = e.target.files?.[0];
              e.target.value = "";
              if (!picked) return;
              if (picked.size > 10 * 1024 * 1024) {
                setMessage("导入文件不能超过 10 MB。");
                return;
              }
              void picked
                .text()
                .then((content) => setPending({ name: picked.name, content }))
                .catch((e) => setMessage(String(e)));
            }}
          />
        </div>
      </Row>
      {message && (
        <p className="settings-note" role="status">
          {message}
        </p>
      )}
      {pending && (
        <div className="settings-note">
          <p>
            导入「{pending.name}」：仅支持知了 JSON
            导出格式，将追加任务、项目、重复规则和完成历史。不会覆盖现有数据，重复导入会产生副本；导入前自动备份。
          </p>
          <div className="reply-actions">
            <button
              className="btn primary"
              disabled={busy}
              onClick={() =>
                void act(async () => {
                  const result = await importData(pending.content);
                  setPending(null);
                  await refresh();
                  await onImported();
                  setMessage(`已导入 ${result.tasks} 个任务、${result.nodes} 个项目节点。`);
                })
              }
            >
              确认导入
            </button>
            <button className="btn" disabled={busy} onClick={() => setPending(null)}>
              算了
            </button>
          </div>
        </div>
      )}
      <div className="settings-row col">
        <div className="lab">
          <b>操作日志</b>
          <small>只读。用户操作可在主界面说“撤销”回退；数据导入请通过导入前备份恢复。</small>
        </div>
        <div className="settings-log">
          {logs.length === 0 ? (
            <small>还没有操作记录</small>
          ) : (
            logs.map((log) => (
              <div className="li" key={log.id}>
                <span className="m">{log.createdAt}</span>
                <span className="grow">
                  {log.summary || `${log.operation} · ${log.entity}`}
                  {log.undoneAt ? "（已撤销）" : ""}
                </span>
                <span className="chip">
                  {(
                    { chat: "对话", manual: "手动勾选", system: "系统", undo: "撤销" } as Record<
                      string,
                      string
                    >
                  )[log.source] ?? log.source}
                </span>
              </div>
            ))
          )}
        </div>
      </div>
    </>
  );
}
