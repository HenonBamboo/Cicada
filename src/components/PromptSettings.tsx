import { useEffect, useState } from "react";
import {
  getPrompts,
  previewPrompt,
  resetPrompt,
  savePrompt,
  type PromptView,
} from "../lib/settings";
import { openFolder } from "../lib/commands";
export function PromptSettings({ report }: { report: (s: string) => void }) {
  const [prompts, setPrompts] = useState<PromptView[]>([]),
    [id, setId] = useState("base"),
    [draft, setDraft] = useState(""),
    [preview, setPreview] = useState<string | null>(null),
    [compare, setCompare] = useState(false),
    [busy, setBusy] = useState(false);
  const current = prompts.find((p) => p.id === id);
  useEffect(() => {
    void getPrompts()
      .then((p) => {
        setPrompts(p);
        setId(p[0]?.id || "base");
        setDraft(p[0]?.content || "");
      })
      .catch((e) => report(String(e)));
  }, [report]);
  const update = (p: PromptView) => {
    setPrompts((all) => all.map((x) => (x.id === p.id ? p : x)));
    setDraft(p.content);
    return p;
  };
  const save = async () =>
    current && draft !== current.content ? update(await savePrompt(id, draft)) : current;
  const act = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
    } catch (e) {
      report(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section id="prompts">
      <h2>提示词</h2>
      <div className="card settings-group">
        <fieldset
          disabled={busy}
          onMouseDown={(e) => {
            if ((e.target as HTMLElement).closest("button")) e.preventDefault();
          }}
        >
          <div className="settings-row col">
            <div className="lab">
              <b>内部提示词</b>
              <small>
                知了怎么理解你、怎么回答，都写在这里，可以按自己的习惯调整。改提示词不会放宽权限：能改哪些数据始终由程序校验。
              </small>
            </div>
            <div className="pr-tabs">
              {prompts.map((p) => (
                <button
                  key={p.id}
                  className={`${p.id === id ? "on" : ""} ${p.modified ? "mod" : ""}`}
                  onClick={() =>
                    void act(async () => {
                      const saved = await save();
                      setId(p.id);
                      setDraft(saved?.id === p.id ? saved.content : p.content);
                      setPreview(null);
                    })
                  }
                >
                  {p.title}
                </button>
              ))}
            </div>
            <textarea
              className="pr-edit"
              spellCheck={false}
              aria-label="提示词内容"
              readOnly={preview !== null}
              value={preview ?? draft}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={() =>
                void act(async () => {
                  await save();
                })
              }
            />
            {compare && current && (
              <div className="prompt-default">
                <small>默认内容</small>
                <textarea
                  className="pr-edit"
                  aria-label="默认提示词"
                  readOnly
                  value={current.defaultContent}
                />
              </div>
            )}
            <div className="pr-meta">
              <span>可用变量</span>
              {current?.variables.map((v) => (
                <code key={v.name} title={v.description}>{`{{${v.name}}}`}</code>
              ))}
              <span>
                {preview !== null
                  ? "预览：变量已按当前时间填充"
                  : current?.modified
                    ? "已修改，下一轮对话起生效"
                    : "默认版本"}
              </span>
            </div>
            {current?.defaultUpdated && (
              <small>默认版本已更新，可与默认内容对比后决定是否恢复。</small>
            )}
            {!!current?.unknownVariables.length && (
              <small>未知变量将保留原文：{current.unknownVariables.join("、")}</small>
            )}
            <div className="reply-actions">
              <button
                className="btn"
                onClick={() =>
                  void act(async () => {
                    if (preview !== null) {
                      setPreview(null);
                      return;
                    }
                    await save();
                    const p = await previewPrompt(id);
                    setPreview(
                      p.content + (p.tools ? `\n\n【程序生成 · 工具定义】\n${p.tools}` : ""),
                    );
                  })
                }
              >
                {preview !== null ? "返回编辑" : "预览完整提示词"}
              </button>
              <button className="btn" onClick={() => setCompare(!compare)}>
                {compare ? "收起默认对比" : "与默认对比"}
              </button>
              <button
                className="btn"
                onClick={() =>
                  void act(async () => {
                    update(await resetPrompt(id));
                    setPreview(null);
                  })
                }
              >
                恢复默认
              </button>
              <button
                className="btn"
                onClick={() => void openFolder("prompts").catch((e) => report(String(e)))}
              >
                打开文件夹
              </button>
            </div>
          </div>
        </fieldset>
      </div>
    </section>
  );
}
