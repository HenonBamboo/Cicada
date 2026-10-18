/** 圆形勾选框。readonly 时只展示（项目节点），不可点击 */
export function Check({
  on,
  readonly,
  label,
  onToggle,
}: {
  on: boolean;
  readonly?: boolean;
  label?: string;
  onToggle?: () => void;
}) {
  const mark = (
    <svg viewBox="0 0 12 12" aria-hidden="true">
      <path d="M2.5 6.2 5 8.6 9.6 3.6" />
    </svg>
  );
  if (readonly) {
    return <span className={`check readonly${on ? " on" : ""}`}>{mark}</span>;
  }
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={on}
      aria-label={label}
      className={`check${on ? " on" : ""}`}
      onClick={onToggle}
    >
      {mark}
    </button>
  );
}
