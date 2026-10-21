import { useState } from "react";
import type { ProjectNode } from "../../lib/types";
import { MarkdownInline, markdownToPlainText } from "../../lib/markdown";
import { itemDescription, nodeDeadline } from "../../lib/nodePresentation";
import { Check } from "../Check";
import { Icon } from "../Icon";

function Row({
  n,
  collapsed,
  toggle,
  isFlash,
}: {
  n: ProjectNode;
  collapsed: ReadonlySet<number>;
  toggle: (id: number) => void;
  isFlash: (id: number) => boolean;
}) {
  const hasKids = n.children.length > 0;
  const closed = collapsed.has(n.id);
  const pr = n.progress;
  return (
    <div>
      <div
        className={`prow l${n.depth}${n.done ? " done" : ""}${isFlash(n.id) ? " flash" : ""}`}
        data-item-details={itemDescription(n)}
      >
        {hasKids ? (
          <button
            type="button"
            className={`tg${closed ? " collapsed" : ""}`}
            aria-expanded={!closed}
            aria-label={`${closed ? "展开" : "折叠"}「${markdownToPlainText(n.title)}」`}
            onClick={() => toggle(n.id)}
          >
            <Icon name="chevron-down" />
          </button>
        ) : (
          <span className="spacer" />
        )}
        <Check on={n.done} readonly />
        <span className="t">
          <MarkdownInline text={n.title} />
        </span>
        {pr && (
          <span className="prog">
            {pr.done}/{pr.total}
            <span className="bar">
              <span style={{ width: `${(pr.done / pr.total) * 100}%` }} />
            </span>
          </span>
        )}
      </div>
      {n.dueAt && (
        <div className={`project-detail l${n.depth}`} data-item-details={itemDescription(n)}>
          <span className="chip amber">
            <Icon name="flag" />
            截止 {nodeDeadline(n.dueAt)}
          </span>
        </div>
      )}
      {hasKids && !closed && (
        <div>
          {n.children.map((k) => (
            <Row key={k.id} n={k} collapsed={collapsed} toggle={toggle} isFlash={isFlash} />
          ))}
        </div>
      )}
    </div>
  );
}

/** 文件夹视图：可展开折叠的缩进列表；已归档项目折叠在底部 */
export function FolderView({
  active,
  archived,
  isFlash,
}: {
  active: ProjectNode[];
  archived: ProjectNode[];
  isFlash: (id: number) => boolean;
}) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<number>>(new Set());
  const [archivedOpen, setArchivedOpen] = useState(false);
  const toggle = (id: number) =>
    setCollapsed((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <>
      {active.map((p) => (
        <div className="proj" key={p.id}>
          <Row n={p} collapsed={collapsed} toggle={toggle} isFlash={isFlash} />
        </div>
      ))}
      {archived.length > 0 && (
        <>
          <button
            type="button"
            className={`sec${archivedOpen ? "" : " collapsed"}`}
            aria-expanded={archivedOpen}
            onClick={() => setArchivedOpen((v) => !v)}
          >
            <Icon name="chevron-down" />
            已归档 <span className="n">{archived.length}</span>
          </button>
          {archivedOpen &&
            archived.map((p) => (
              <div className="proj archived" key={p.id}>
                <Row n={p} collapsed={collapsed} toggle={toggle} isFlash={isFlash} />
              </div>
            ))}
        </>
      )}
    </>
  );
}
