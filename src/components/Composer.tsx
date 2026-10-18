import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ClipboardEvent,
  type KeyboardEvent,
} from "react";
import { isModeShortcut } from "../lib/keys";
import type { Mode } from "../lib/types";
import { Icon } from "./Icon";

const PLACEHOLDER: Record<Mode, string> = {
  task: "说点什么，比如：明天下午三点提醒我交周报",
  project: "比如：给「知了」加一个阶段：打包发布",
};

const MAX_HEIGHT = 150;
const MAX_TEXT_LENGTH = 20_000;
const MAX_IMAGES = 4;
const MAX_IMAGE_BYTES = 8 * 1024 * 1024;
const MAX_IMAGE_TOTAL_BYTES = 20 * 1024 * 1024;

function dataUrlBytes(url: string) {
  const data = url.split(",", 2)[1] ?? "";
  const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((data.length * 3) / 4) - padding);
}

export function Composer({
  mode,
  onModeChange,
  onSubmit,
  onFocus,
  busy,
  focusRequest = 0,
}: {
  mode: Mode;
  onModeChange: (m: Mode) => void;
  /** 提交文字和图片；图片使用 data URL 交给识图模型。 */
  onSubmit: (text: string, images: string[]) => Promise<boolean>;
  onFocus: () => void;
  busy: boolean;
  focusRequest?: number;
}) {
  const [text, setText] = useState("");
  const [images, setImages] = useState<string[]>([]);
  const [inputError, setInputError] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);
  const file = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (focusRequest > 0) input.current?.focus();
  }, [focusRequest]);

  // 输入框随内容自动增高
  useLayoutEffect(() => {
    const el = input.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
  }, [text]);

  const canSend = !busy && (text.trim().length > 0 || images.length > 0);

  const send = async () => {
    if (!canSend) return;
    if (await onSubmit(text.trim(), images)) {
      setText("");
      setImages([]);
      setInputError("");
    }
  };

  const addFiles = (files: Iterable<File>) => {
    setInputError("");
    const selected = [...files];
    const accepted: File[] = [];
    let total = images.reduce((sum, image) => sum + dataUrlBytes(image), 0);
    for (const f of selected) {
      if (!["image/png", "image/jpeg", "image/webp", "image/gif"].includes(f.type)) {
        setInputError("仅支持 PNG、JPEG、WebP 和 GIF 图片");
        continue;
      }
      if (images.length + accepted.length >= MAX_IMAGES) {
        setInputError(`一次最多添加 ${MAX_IMAGES} 张图片`);
        break;
      }
      if (f.size > MAX_IMAGE_BYTES) {
        setInputError(`单张图片不能超过 ${MAX_IMAGE_BYTES / 1024 / 1024} MiB`);
        continue;
      }
      if (total + f.size > MAX_IMAGE_TOTAL_BYTES) {
        setInputError(`图片总大小不能超过 ${MAX_IMAGE_TOTAL_BYTES / 1024 / 1024} MiB`);
        break;
      }
      accepted.push(f);
      total += f.size;
    }
    for (const f of accepted) {
      const r = new FileReader();
      r.onload = () => setImages((list) => [...list, String(r.result)]);
      r.onerror = () => setInputError("图片读取失败，请重新选择");
      r.readAsDataURL(f);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void send();
    } else if (isModeShortcut(e)) {
      e.preventDefault();
      onModeChange(mode === "task" ? "project" : "task");
    }
  };

  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = [...e.clipboardData.files];
    if (files.length) {
      e.preventDefault();
      addFiles(files);
    }
  };

  return (
    <div className="composer" onMouseDown={onFocus}>
      <div className="composer-top">
        <div className="seg" role="radiogroup" aria-label="输入模式">
          {(["task", "project"] as const).map((m) => (
            <button
              key={m}
              type="button"
              role="radio"
              aria-checked={mode === m}
              className={mode === m ? "on" : undefined}
              onClick={() => {
                onModeChange(m);
                input.current?.focus();
              }}
            >
              {m === "task" ? "任务" : "项目"}
            </button>
          ))}
        </div>
        <span className="hint">Tab 切换</span>
      </div>

      {images.length > 0 && (
        <div className="attachments">
          {images.map((src, i) => (
            <div className="thumb" key={i}>
              <img src={src} alt={`图片 ${i + 1}`} />
              <button
                type="button"
                aria-label="移除图片"
                disabled={busy}
                onClick={() => setImages((list) => list.filter((_, k) => k !== i))}
              >
                <Icon name="x" size={11} />
              </button>
            </div>
          ))}
        </div>
      )}

      <textarea
        ref={input}
        rows={1}
        value={text}
        maxLength={MAX_TEXT_LENGTH}
        disabled={busy}
        placeholder={PLACEHOLDER[mode]}
        aria-label="输入框"
        onChange={(e) => {
          setText(e.target.value);
          if (inputError) setInputError("");
        }}
        onKeyDown={onKeyDown}
        onPaste={onPaste}
        onFocus={onFocus}
      />

      <div className="composer-bottom">
        <button
          type="button"
          className="icon-btn"
          title="添加图片（也可以直接 Ctrl+V 粘贴截图）"
          aria-label="添加图片"
          disabled={busy}
          onClick={() => file.current?.click()}
        >
          <Icon name="image" />
        </button>
        <input
          ref={file}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/gif"
          multiple
          hidden
          onChange={(e) => {
            if (e.target.files) addFiles(e.target.files);
            e.target.value = "";
          }}
        />
        <span className="kbd" role={inputError ? "status" : undefined}>
          {inputError || "Enter 发送 · Shift+Enter 换行"}
        </span>
        <button
          type="button"
          className="send"
          title="发送"
          aria-label="发送"
          disabled={!canSend}
          onClick={() => void send()}
        >
          <Icon name="arrow-up" />
        </button>
      </div>
    </div>
  );
}
