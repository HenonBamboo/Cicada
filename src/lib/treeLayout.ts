import type { ProjectNode } from "./types";
import { markdownToPlainText } from "./markdown";

export interface LaidNode {
  id: number;
  node: ProjectNode;
  x: number;
  y: number;
  root: boolean;
}

export interface LaidEdge {
  id: string;
  source: number;
  target: number;
}

const ROW = 40;
const PROJECT_GAP = 28;
const COL_GAP = 32;
/** 节点文字字号（px），与 .tbox / .tbox.root 一致 */
const FONT = 12;
const ROOT_FONT = 13;

/** 估算节点宽度：中文约一个字号宽，英文数字约半个；加上内边距、进度数字和完成对勾 */
export function estimateWidth(n: ProjectNode): number {
  const font = n.depth === 1 ? ROOT_FONT : FONT;
  const text = [...markdownToPlainText(n.title)].reduce(
    (w, ch) => w + (ch.charCodeAt(0) > 0x2e80 ? font : font * 0.55),
    0,
  );
  return 22 + text + (n.progress ? 30 : 0) + (n.done && n.depth > 1 ? 19 : 0);
}

/**
 * 横向树布局：每一层一列，列宽取该层最宽节点；叶子依次往下排，父节点居中于子节点。
 * 多个项目上下堆叠。
 */
export function layoutTree(projects: ProjectNode[]): { nodes: LaidNode[]; edges: LaidEdge[] } {
  const nodes: LaidNode[] = [];
  const edges: LaidEdge[] = [];

  const colWidth = [0, 0, 0];
  const measure = (n: ProjectNode) => {
    const d = Math.min(n.depth, 3) - 1;
    colWidth[d] = Math.max(colWidth[d], estimateWidth(n));
    n.children.forEach(measure);
  };
  projects.forEach(measure);
  const colX = [0, colWidth[0] + COL_GAP, colWidth[0] + colWidth[1] + COL_GAP * 2];

  let cursor = 0;
  const place = (n: ProjectNode): number => {
    let y: number;
    if (n.children.length) {
      const ys = n.children.map((c) => {
        edges.push({ id: `${n.id}-${c.id}`, source: n.id, target: c.id });
        return place(c);
      });
      y = (ys[0] + ys[ys.length - 1]) / 2;
    } else {
      y = cursor;
      cursor += ROW;
    }
    nodes.push({ id: n.id, node: n, x: colX[Math.min(n.depth, 3) - 1], y, root: n.depth === 1 });
    return y;
  };
  projects.forEach((p, i) => {
    if (i > 0) cursor += PROJECT_GAP;
    place(p);
  });
  return { nodes, edges };
}
