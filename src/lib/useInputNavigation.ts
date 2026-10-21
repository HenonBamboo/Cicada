import { useEffect } from "react";
import { isExampleShortcut, isForwardFocusShortcut, moveFocusForward } from "./keys";

/** Tab 留给输入示例；Shift+Tab 在窗口内持续正向导航。 */
export function useInputNavigation() {
  useEffect(() => {
    const navigate = (event: KeyboardEvent) => {
      if (isExampleShortcut(event)) event.preventDefault();
      if (isForwardFocusShortcut(event)) {
        event.preventDefault();
        if (!event.isComposing && document.activeElement) moveFocusForward(document.activeElement);
      }
    };
    document.addEventListener("keydown", navigate, true);
    return () => document.removeEventListener("keydown", navigate, true);
  }, []);
}
