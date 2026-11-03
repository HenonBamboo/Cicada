import { useCallback, useEffect, useRef, useState } from "react";
import {
  cancelPending,
  confirmPending,
  discardInbox,
  listInbox,
  onAgentDelta,
  onCaptureReply,
  onInboxChanged,
  retryInbox,
  stopInput,
  submitInput,
  undoLast,
} from "./commands";
import type { InboxItem, Mode, Reply } from "./types";
import type { ReplyView } from "../components/ReplyCard";
import { isInboxRequest, needsInboxAttention } from "./inbox";

export function useAgent() {
  const [reply, setReply] = useState<ReplyView | null>(null);
  const [result, setResult] = useState<Reply | null>(null);
  const [inbox, setInbox] = useState<InboxItem[]>([]);
  const [showInbox, setShowInbox] = useState(false);
  const [busy, setBusy] = useState(false);
  const [stopPending, setStopPending] = useState(false);
  const [stopMessage, setStopMessage] = useState<string | null>(null);
  const [activeIds, setActiveIds] = useState<string[]>([]);
  const active = useRef<string[]>([]);
  const running = useRef(false);
  const generation = useRef(0);
  const visibleRequest = useRef<string | null>(null);
  const displayResult = useCallback((next: Reply) => {
    visibleRequest.current = next.requestId;
    setResult(next);
    setReply({
      kind: "message",
      status: next.status ?? undefined,
      lead: next.lead,
      tone: next.kind === "error" ? "danger" : undefined,
      items: next.items.map((item) => ({
        icon: item.danger ? "minus" : "check",
        text: item.text,
        danger: item.danger,
      })),
    });
  }, []);
  const showError = useCallback((error: unknown) => {
    visibleRequest.current = null;
    setShowInbox(false);
    setResult(null);
    setReply({ kind: "message", status: "没有完成", lead: String(error), tone: "danger" });
  }, []);
  const refreshInbox = useCallback(() => listInbox().then(setInbox).catch(showError), [showError]);
  useEffect(() => {
    void refreshInbox();
    const off = onInboxChanged(() => void refreshInbox());
    const replyOff = onCaptureReply((next) => {
      if (!running.current && visibleRequest.current === next.requestId) displayResult(next);
    });
    const deltaOff = onAgentDelta((delta) => {
      if (!active.current.includes(delta.requestId)) return;
      setReply((previous) => ({
        kind: "message",
        status: "正在整理",
        streaming: true,
        lead:
          (previous?.kind === "message" && previous.streaming ? previous.lead : "") + delta.text,
      }));
    });
    return () => {
      void off.then((f) => f());
      void replyOff.then((f) => f());
      void deltaOff.then((f) => f());
    };
  }, [refreshInbox, displayResult]);
  const run = async (action: () => Promise<Reply>, requestId?: string, pendingId?: string) => {
    if (running.current) return false;
    running.current = true;
    generation.current++;
    active.current = [requestId, pendingId].filter((id): id is string => !!id);
    setActiveIds(active.current);
    setBusy(true);
    setStopPending(false);
    setStopMessage(null);
    setShowInbox(false);
    visibleRequest.current = null;
    setResult(null);
    setReply({ kind: "thinking" });
    try {
      const next = await action();
      displayResult(next);
      return true;
    } catch (error) {
      showError(error);
      return false;
    } finally {
      active.current = [];
      setActiveIds([]);
      running.current = false;
      setBusy(false);
      setStopPending(false);
      setStopMessage(null);
      void refreshInbox();
    }
  };
  const attentionInbox = inbox.filter(needsInboxAttention);
  const openInbox = () => {
    if (running.current) return;
    visibleRequest.current = null;
    setResult(null);
    setShowInbox(true);
    setReply({
      kind: "message",
      status: "收件箱",
      lead: attentionInbox.length ? "这些输入需要你处理。" : "暂时没有需要处理的输入。",
    });
    void refreshInbox();
  };
  return {
    reply: showInbox
      ? ({
          kind: "message",
          status: "收件箱",
          lead: attentionInbox.length ? "这些输入需要你处理。" : "暂时没有需要处理的输入。",
        } satisfies ReplyView)
      : reply,
    result,
    inbox,
    attentionInbox,
    showInbox,
    busy,
    // 等后台确认输入已登记再提供停止，避免停止先于登记而漏掉这次输入。
    canStop:
      busy &&
      inbox.some(
        (item) =>
          activeIds.includes(item.id) && (item.status === "processing" || item.status === "queued"),
      ),
    stopPending,
    stopMessage,
    stop: async () => {
      if (!running.current || stopPending || !active.current.length) return;
      const current = generation.current;
      setStopPending(true);
      setStopMessage("正在停止…");
      try {
        const state = await stopInput([...active.current]);
        if (current !== generation.current || !running.current) return;
        if (state === "committing") setStopMessage("正在保存结果，请稍候。");
        else if (state === "finished") {
          setStopPending(false);
          setStopMessage("请求尚未开始或已经结束，请稍候查看结果。");
        }
      } catch (error) {
        if (current !== generation.current || !running.current) return;
        setStopPending(false);
        setStopMessage(String(error));
      }
    },
    stopInbox: async (id: string) => {
      try {
        const state = await stopInput([id]);
        if (state === "committing") {
          setReply({ kind: "message", status: "收件箱", lead: "正在保存结果，请稍候。" });
        }
        await refreshInbox();
      } catch (error) {
        showError(error);
      }
    },
    showError,
    setReply: (view: ReplyView | null) => {
      if (running.current) return;
      visibleRequest.current = null;
      setResult(null);
      setShowInbox(false);
      setReply(view);
    },
    openInbox,
    submit: (text: string, mode: Mode, images: string[], selectionToken?: string) => {
      if (
        !running.current &&
        !images.length &&
        !selectionToken &&
        result?.kind !== "clarify" &&
        result?.kind !== "confirm" &&
        isInboxRequest(text)
      ) {
        openInbox();
        return Promise.resolve(true);
      }
      const id = crypto.randomUUID();
      return run(
        () =>
          submitInput(
            text,
            mode,
            images,
            id,
            result?.kind === "clarify" || result?.kind === "confirm" ? result.requestId : undefined,
            selectionToken,
          ),
        id,
        result?.kind === "clarify" || result?.kind === "confirm" ? result.requestId : undefined,
      );
    },
    retry: (item: InboxItem) => run(() => retryInbox(item.id), item.id),
    discard: async (id: string) => {
      try {
        await discardInbox(id);
        await refreshInbox();
      } catch (error) {
        showError(error);
      }
    },
    confirm: () => result && run(() => confirmPending(result.requestId)),
    cancel: () => result && run(() => cancelPending(result.requestId)),
    undo: () => run(undoLast),
  };
}
