import { useRef, useState, type ClipboardEvent } from "react";
import { submitCapture } from "./desktop";
import type { Mode } from "./types";

export function useCapture() {
  const [text, setText] = useState("");
  const [mode, setMode] = useState<Mode>("task");
  const [images, setImages] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sending = useRef(false);
  const pendingImages = useRef(0);
  const [loadingImages, setLoadingImages] = useState(false);
  const requestId = useRef<string | null>(null);
  const changeText = (value: string) => {
    requestId.current = null;
    setText(value);
  };
  const changeMode = (value: Mode) => {
    requestId.current = null;
    setMode(value);
  };
  const send = async () => {
    if (sending.current || pendingImages.current || (!text.trim() && !images.length)) return false;
    sending.current = true;
    setBusy(true);
    setError(null);
    requestId.current ??= crypto.randomUUID();
    try {
      await submitCapture(text.trim(), mode, images, requestId.current);
      setText("");
      setImages([]);
      requestId.current = null;
      return true;
    } catch (error) {
      setError(String(error));
      return false;
    } finally {
      sending.current = false;
      setBusy(false);
    }
  };
  const paste = (event: ClipboardEvent<HTMLInputElement>) => {
    const files = [...event.clipboardData.files].filter((file) =>
      ["image/png", "image/jpeg", "image/webp", "image/gif"].includes(file.type),
    );
    if (!files.length) return;
    event.preventDefault();
    requestId.current = null;
    for (const file of files) {
      pendingImages.current++;
      setLoadingImages(true);
      const reader = new FileReader();
      reader.onload = () => setImages((items) => [...items, String(reader.result)]);
      reader.onerror = () => setError("图片读取失败，请重新粘贴。");
      reader.onloadend = () => {
        pendingImages.current--;
        setLoadingImages(pendingImages.current > 0);
      };
      reader.readAsDataURL(file);
    }
  };
  return {
    text,
    setText: changeText,
    mode,
    setMode: changeMode,
    images,
    busy: busy || loadingImages,
    error,
    setError,
    send,
    paste,
    clearImages: () => {
      requestId.current = null;
      setImages([]);
    },
  };
}
