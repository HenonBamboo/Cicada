import { describe, expect, it } from "vitest";
import { layoutTree } from "./treeLayout";
import type { ProjectNode } from "./types";

const n = (id: number, depth: number, children: ProjectNode[] = []): ProjectNode => ({
  id,
  title: `节点${id}`,
  depth,
  done: false,
  createdAt: "2026-10-20T09:00:00",
  doneAt: null,
  note: null,
  dueAt: null,
  projectStatus: depth === 1 ? "active" : null,
  progress: children.length ? { done: 0, total: children.length } : null,
  children,
});

describe("layoutTree", () => {
  it("每层一列，父节点居中于子节点", () => {
    const tree = [n(1, 1, [n(2, 2, [n(4, 3), n(5, 3)]), n(3, 2)])];
    const { nodes, edges } = layoutTree(tree);
    const at = (id: number) => nodes.find((x) => x.id === id)!;

    expect(nodes).toHaveLength(5);
    expect(edges.map((e) => e.id).sort()).toEqual(["1-2", "1-3", "2-4", "2-5"]);
    expect(at(1).x).toBe(0);
    expect(at(2).x).toBeGreaterThan(at(1).x);
    expect(at(4).x).toBeGreaterThan(at(2).x);
    expect(at(2).y).toBe((at(4).y + at(5).y) / 2);
    expect(at(1).root).toBe(true);
  });

  it("多个项目上下堆叠互不重叠", () => {
    const { nodes } = layoutTree([n(1, 1, [n(2, 2)]), n(3, 1, [n(4, 2)])]);
    const y = (id: number) => nodes.find((x) => x.id === id)!.y;
    expect(y(4)).toBeGreaterThan(y(2));
    expect(y(3)).toBeGreaterThan(y(1));
  });
});
