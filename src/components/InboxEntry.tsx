/** 主界面与快捷记录使用同一入口，只在有待处理内容时提示。 */
export function InboxEntry({
  count,
  busy,
  onOpen,
}: {
  count: number;
  busy: boolean;
  onOpen: () => void;
}) {
  if (!count) return null;
  return (
    <button className="btn ghost inbox-link" disabled={busy} onClick={onOpen}>
      收件箱 {count} 条
    </button>
  );
}
