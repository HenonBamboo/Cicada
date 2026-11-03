import { useEffect, useRef } from "react";
import { hideQuick, onDesktopFocus, onQuickOpened, resizeQuick } from "../../lib/desktop";
import { useCapture } from "../../lib/useCapture";
import { useUiCopy } from "../../lib/useUiCopy";
import { useInputExample } from "../../lib/useInputExample";
import { useDesktopAppearance } from "../../lib/useDesktopAppearance";
import { canAdoptExample, isModeShortcut } from "../../lib/keys";
import { useInputNavigation } from "../../lib/useInputNavigation";
import { useAgent } from "../../lib/useAgent";
import { ReplyCard } from "../../components/ReplyCard";
import { ReplyActions } from "../../components/ReplyActions";
import { InboxEntry } from "../../components/InboxEntry";
import { InboxList } from "../../components/InboxList";
import "./quick.css";

export function QuickCapture() {
  useInputNavigation();
  const input = useRef<HTMLInputElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const appearanceError = useDesktopAppearance();
  const agent = useAgent();
  const capture = useCapture("quick", "task", agent.submit);
  const busy = capture.busy || agent.busy;
  const copy = useUiCopy();
  const example = useInputExample(
    copy,
    capture.mode,
    busy || !!capture.text.length || !!capture.images.length,
  );
  const setCaptureError = capture.setError;
  useEffect(() => {
    const element = content.current;
    if (!element) return;
    let disposed = false;
    let frame = 0;
    let lastHeight = 0;
    let chain = Promise.resolve();
    const measure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const height = Math.max(132, Math.ceil(element.getBoundingClientRect().height) + 16);
        if (height === lastHeight) return;
        lastHeight = height;
        chain = chain.then(async () => {
          if (disposed || height !== lastHeight) return;
          try {
            await resizeQuick(height);
          } catch (error) {
            if (!disposed) setCaptureError(String(error));
          }
        });
      });
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    measure();
    return () => {
      disposed = true;
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, [setCaptureError]);
  useEffect(() => {
    if (!busy) input.current?.focus();
  }, [busy]);
  useEffect(() => {
    const focus = () => input.current?.focus();
    focus();
    const off = onDesktopFocus(focus);
    const openedOff = onQuickOpened(focus);
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        void hideQuick().catch((error) => setCaptureError(String(error)));
      }
    };
    document.addEventListener("keydown", key);
    return () => {
      void off.then((f) => f());
      void openedOff.then((f) => f());
      document.removeEventListener("keydown", key);
    };
  }, [setCaptureError]);
  return (
    <main className="capture" aria-label="快速记录">
      <div ref={content} className="capture-content">
        <div className="cap-box">
          <img src="/logo.svg" alt="" />
          <input
            ref={input}
            aria-label="快速记录内容"
            value={capture.text}
            disabled={busy}
            autoComplete="off"
            maxLength={20_000}
            placeholder={example.placeholder}
            onChange={(event) => capture.setText(event.target.value)}
            onPaste={capture.paste}
            onKeyDown={(event) => {
              if (busy || event.nativeEvent.isComposing || capture.historyKeyDown(event)) return;
              if (isModeShortcut(event)) {
                event.preventDefault();
                capture.setMode(capture.mode === "task" ? "project" : "task");
              } else if (
                canAdoptExample(event, capture.text, capture.images.length, busy, example.example)
              ) {
                event.preventDefault();
                capture.setText(example.example);
              } else if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                event.preventDefault();
                void capture.send();
              }
            }}
          />
          <div className="seg" role="radiogroup" aria-label="记录模式">
            {(["task", "project"] as const).map((mode) => (
              <button
                key={mode}
                role="radio"
                aria-checked={capture.mode === mode}
                className={capture.mode === mode ? "on" : undefined}
                disabled={busy}
                onClick={() => {
                  capture.setMode(mode);
                  input.current?.focus();
                }}
              >
                {mode === "task" ? "任务" : "项目"}
              </button>
            ))}
          </div>
        </div>
        <div className="cap-hint">
          <span>
            <kbd>Enter</kbd> 发送 · <kbd>Ctrl+Tab</kbd> 切换 · <kbd>Ctrl+V</kbd> 粘贴截图
          </span>
          <span>
            <kbd>Esc</kbd> 收起
          </span>
        </div>
        {!!capture.images.length && (
          <button className="capture-attachments" disabled={busy} onClick={capture.clearImages}>
            已附 {capture.images.length} 张图片 · 移除
          </button>
        )}
        {(capture.error || appearanceError) && (
          <p className="capture-error" role="status">
            {capture.error || appearanceError}
          </p>
        )}
        <InboxEntry count={agent.attentionInbox.length} busy={busy} onOpen={agent.openInbox} />
        {agent.reply && (
          <ReplyCard
            view={agent.reply}
            onClose={() => agent.setReply(null)}
            onStop={agent.canStop ? () => void agent.stop() : undefined}
            stopPending={agent.stopPending}
            stopMessage={agent.stopMessage}
          >
            {agent.showInbox && (
              <InboxList
                items={agent.attentionInbox}
                busy={busy}
                onRetry={(item) => {
                  capture.setMode(item.mode);
                  void agent.retry(item);
                }}
                onDiscard={(id) => void agent.discard(id)}
                onStop={(id) => void agent.stopInbox(id)}
              />
            )}
            {agent.result && (
              <ReplyActions
                result={agent.result}
                busy={busy}
                onConfirm={() => void agent.confirm()}
                onCancel={() => void agent.cancel()}
                onSubmit={(text, token) => void agent.submit(text, capture.mode, [], token)}
              />
            )}
          </ReplyCard>
        )}
      </div>
    </main>
  );
}
