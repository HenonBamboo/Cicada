import { useState, type ReactNode } from "react";
export function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="settings-row">
      <div className="lab">
        <b>{label}</b>
        {hint && <small>{hint}</small>}
      </div>
      {children}
    </div>
  );
}
export function Switch({
  value,
  onChange,
  label,
}: {
  value: boolean;
  onChange: (value: boolean) => void;
  label: string;
}) {
  return (
    <label className="settings-switch">
      <input
        aria-label={label}
        type="checkbox"
        checked={value}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span />
    </label>
  );
}
export function Seg<T extends string>({
  value,
  choices,
  onChange,
}: {
  value: T;
  choices: [T, string][];
  onChange: (value: T) => void;
}) {
  return (
    <div className="seg">
      {choices.map(([id, label]) => (
        <button key={id} className={id === value ? "on" : ""} onClick={() => onChange(id)}>
          {label}
        </button>
      ))}
    </div>
  );
}
export function Shortcut({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const [recording, setRecording] = useState(false);
  return (
    <button
      className={`kbd-btn ${recording ? "rec" : ""}`}
      title="点击后按下新的组合键"
      onBlur={() => setRecording(false)}
      onClick={() => setRecording(!recording)}
      onKeyDown={(e) => {
        if (!recording) return;
        e.preventDefault();
        e.stopPropagation();
        if (e.key === "Escape") {
          setRecording(false);
          return;
        }
        if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return;
        if (!e.ctrlKey && !e.altKey && !e.metaKey) return;
        const key =
          e.code === "Space"
            ? "Space"
            : e.code.startsWith("Key")
              ? e.code.slice(3)
              : e.code.startsWith("Digit")
                ? e.code.slice(5)
                : e.key;
        onChange(
          [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super", key]
            .filter(Boolean)
            .join("+"),
        );
        setRecording(false);
      }}
    >
      <span className="kbd">
        {recording ? (
          <span>请按下新的组合键…</span>
        ) : (
          value.split("+").map((k, i) => <span key={i}>{k}</span>)
        )}
      </span>
    </button>
  );
}
