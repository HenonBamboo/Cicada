import { useEffect, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Icon, type IconName } from "./Icon";

function WinButton({
  icon,
  label,
  danger,
  onClick,
}: {
  icon: IconName;
  label: string;
  danger?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={`grid h-full w-11 place-items-center text-text-2 transition-colors duration-150 ${
        danger ? "hover:bg-danger hover:text-on-danger" : "hover:bg-surface-2 hover:text-text"
      }`}
    >
      <Icon name={icon} size={14} />
    </button>
  );
}

/** 自定义标题栏：整条可拖动，双击最大化/还原（由 data-tauri-drag-region 提供） */
export function TitleBar({ children }: { children?: ReactNode }) {
  const win = getCurrentWindow();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let alive = true;
    const sync = () => win.isMaximized().then((m) => alive && setMaximized(m));
    sync();
    const off = win.onResized(sync);
    return () => {
      alive = false;
      off.then((f) => f());
    };
  }, [win]);

  return (
    <header data-tauri-drag-region className="flex h-10 shrink-0 items-stretch">
      <div
        data-tauri-drag-region
        className="flex flex-1 items-center gap-2 pl-4 text-[0.9rem] text-text-2"
      >
        <img src="/logo.svg" alt="" className="pointer-events-none h-[18px] w-[18px]" />
        <span className="pointer-events-none">知了</span>
      </div>
      {children && <div className="flex items-center gap-0.5 pr-1.5">{children}</div>}
      <WinButton icon="minus" label="最小化" onClick={() => win.minimize()} />
      <WinButton
        icon={maximized ? "restore" : "square"}
        label={maximized ? "还原" : "最大化"}
        onClick={() => win.toggleMaximize()}
      />
      <WinButton icon="x" label="关闭" danger onClick={() => win.close()} />
    </header>
  );
}
