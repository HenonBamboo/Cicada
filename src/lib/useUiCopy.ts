import { useEffect, useSyncExternalStore } from "react";
import { getUiCopy, onUiCopyChanged, type UiCopyView } from "./commands";

const fallback: UiCopyView = {
  greeting: "",
  nudge: null,
  taskPlaceholder: "比如：明天下午三点提醒我交周报",
  projectPlaceholder: "比如：新建一个项目：网站改版",
  taskEmpty: "今天还没有待办，记一件事吧。",
  projectEmpty: "还没有项目。切到项目模式说一句，比如：新建项目「读书计划」",
  generated: false,
};
let copy = fallback;
let users = 0;
let subscriptionGeneration = 0;
let stopListening: Promise<() => void> | undefined;
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};
const snapshot = () => copy;
const update = (value: UiCopyView) => {
  copy = { ...fallback, ...value };
  listeners.forEach((notify) => notify());
};

/** 同一窗口共享一份文案和订阅，组件不会各自请求模型。 */
export function useUiCopy() {
  const value = useSyncExternalStore(subscribe, snapshot, () => fallback);
  useEffect(() => {
    users += 1;
    if (users === 1) {
      const generation = ++subscriptionGeneration;
      let eventReceived = false;
      stopListening = onUiCopyChanged((value) => {
        if (generation !== subscriptionGeneration) return;
        eventReceived = true;
        update(value);
      });
      void getUiCopy()
        .then((value) => {
          if (generation === subscriptionGeneration && !eventReceived) update(value);
        })
        .catch(() => {});
    }
    return () => {
      users -= 1;
      if (!users) {
        subscriptionGeneration += 1;
        void stopListening?.then((stop) => stop());
        stopListening = undefined;
      }
    };
  }, []);
  return value;
}
