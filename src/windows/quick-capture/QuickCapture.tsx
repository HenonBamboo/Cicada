import { useEffect, useRef } from "react";
import { hideQuick, onDesktopFocus, onQuickOpened } from "../../lib/desktop";
import { useCapture } from "../../lib/useCapture";
import { useUiCopy } from "../../lib/useUiCopy";
import { useDesktopAppearance } from "../../lib/useDesktopAppearance";
import { isModeShortcut } from "../../lib/keys";
import "./quick.css";

export function QuickCapture() {
  const input = useRef<HTMLInputElement>(null);
  const appearanceError = useDesktopAppearance();
  const capture = useCapture("quick");
  const copy = useUiCopy();
  const setCaptureError = capture.setError;
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
      <div className="cap-box">
        <img src="/logo.svg" alt="" />
        <input
          ref={input}
          aria-label="快速记录内容"
          value={capture.text}
          disabled={capture.busy}
          autoComplete="off"
          maxLength={20_000}
          placeholder={capture.mode === "task" ? copy.taskPlaceholder : copy.projectPlaceholder}
          onChange={(event) => capture.setText(event.target.value)}
          onPaste={capture.paste}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing || capture.historyKeyDown(event)) return;
            if (isModeShortcut(event)) {
              event.preventDefault();
              capture.setMode(capture.mode === "task" ? "project" : "task");
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
              disabled={capture.busy}
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
          <kbd>Enter</kbd> 记下 · <kbd>Tab</kbd> 切换任务/项目 · <kbd>Ctrl+V</kbd> 粘贴截图
        </span>
        <span>
          <kbd>Esc</kbd> 取消
        </span>
      </div>
      {!!capture.images.length && (
        <button
          className="capture-attachments"
          disabled={capture.busy}
          onClick={capture.clearImages}
        >
          已附 {capture.images.length} 张图片 · 移除
        </button>
      )}
      {(capture.error || appearanceError) && (
        <p className="capture-error" role="status">
          {capture.error || appearanceError}
        </p>
      )}
    </main>
  );
}
