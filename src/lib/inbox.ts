import type { InboxItem } from "./types";

/** 普通排队和处理中不打断用户；只有需要介入的输入才提示收件箱。 */
export function needsInboxAttention(item: InboxItem): boolean {
  return (
    item.status === "awaiting" ||
    item.status === "failed" ||
    (item.status === "queued" &&
      (item.statusReason === "offline" || item.statusReason === "unconfigured"))
  );
}

export function isInboxRequest(text: string): boolean {
  return /^(?:(?:打开|查看|看看)(?:一下)?(?:我的)?收件箱|收件箱(?:里|里面)?(?:还有什么|有什么|有哪些内容)?)[？?。！!\s]*$/.test(
    text.trim(),
  );
}
