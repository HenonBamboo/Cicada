import { useState } from "react";
import { exampleBody } from "./keys";
import type { Mode } from "./types";

interface ExampleCopy {
  taskPlaceholder: string;
  projectPlaceholder: string;
  taskExample?: string;
  projectExample?: string;
}

/** 输入、附图及处理期间保留同一组示例，清空后才采用新候选。 */
export function useInputExample(copy: ExampleCopy, mode: Mode, frozen: boolean) {
  const [stable, setStable] = useState(copy);
  if (!frozen && stable !== copy) setStable(copy);
  const current = frozen ? stable : copy;
  const placeholder = mode === "task" ? current.taskPlaceholder : current.projectPlaceholder;
  const example = mode === "task" ? current.taskExample : current.projectExample;
  const body = exampleBody(example || placeholder);
  return { placeholder: !frozen && body ? `「Tab」 ${placeholder}` : placeholder, example: body };
}
