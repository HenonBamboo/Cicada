import { useMemo } from "react";
import { Handle, Position, ReactFlow, type Edge, type Node, type NodeProps } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { layoutTree } from "../../lib/treeLayout";
import type { ProjectNode } from "../../lib/types";
import { MarkdownInline } from "../../lib/markdown";
import { nodeDeadline, nodeDescription } from "../../lib/nodePresentation";
import { Icon } from "../Icon";

type BoxData = { node: ProjectNode; root: boolean; flash: boolean };
type BoxNode = Node<BoxData, "box">;

function Box({ data }: NodeProps<BoxNode>) {
  const { node: n, root, flash } = data;
  const pr = n.progress;
  return (
    <>
      <Handle type="target" position={Position.Left} isConnectable={false} />
      <span
        className={`tbox${root ? " root" : ""}${n.done && !root ? " done" : ""}${flash ? " flash" : ""}`}
        title={nodeDescription(n)}
      >
        {n.done && !root && <Icon name="check" />}
        <MarkdownInline text={n.title} />
        {n.dueAt && <span className="node-due">{nodeDeadline(n.dueAt)}</span>}
        {pr && (
          <span className="p">
            {pr.done}/{pr.total}
          </span>
        )}
      </span>
      <Handle type="source" position={Position.Right} isConnectable={false} />
    </>
  );
}

const nodeTypes = { box: Box };

/** 树状视图：类似脑图的分支视图，只读（不可拖动、连线、选择） */
export function TreeView({
  projects,
  isFlash,
}: {
  projects: ProjectNode[];
  isFlash: (id: number) => boolean;
}) {
  const { nodes, edges } = useMemo(() => {
    const laid = layoutTree(projects);
    const nodes: BoxNode[] = laid.nodes.map((l) => ({
      id: String(l.id),
      type: "box",
      position: { x: l.x, y: l.y },
      data: { node: l.node, root: l.root, flash: isFlash(l.id) },
      draggable: false,
      selectable: false,
      connectable: false,
    }));
    const edges: Edge[] = laid.edges.map((e) => ({
      id: e.id,
      source: String(e.source),
      target: String(e.target),
      type: "smoothstep",
      selectable: false,
      focusable: false,
    }));
    return { nodes, edges };
  }, [projects, isFlash]);

  return (
    <div className="tree-flow">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        // 不缩放到全部可见（多个项目时字太小），从左上角按正常大小显示，滚轮上下浏览
        defaultViewport={{ x: 16, y: 16, zoom: 0.95 }}
        minZoom={0.4}
        maxZoom={1.5}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable={false}
        edgesFocusable={false}
        nodesFocusable={false}
        zoomOnDoubleClick={false}
        panOnScroll
        preventScrolling={false}
        proOptions={{ hideAttribution: true }}
      />
    </div>
  );
}
