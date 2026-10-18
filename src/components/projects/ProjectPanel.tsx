import { lazy, Suspense, useCallback, useState } from "react";
import type { ProjectNode } from "../../lib/types";
import { nodeKey } from "../../lib/useBoard";
import { Icon } from "../Icon";
import { FolderView } from "./FolderView";

const TreeView = lazy(() => import("./TreeView").then((module) => ({ default: module.TreeView })));

const treeFallback = <div className="tree-flow" aria-hidden="true" />;

type View = "folder" | "tree";

/** 项目区：只读。切换视图、展开折叠属于读操作（docs/产品设计.md 7.3） */
export function ProjectPanel({
  projects,
  flash,
}: {
  projects: ProjectNode[];
  flash: ReadonlySet<string>;
}) {
  const [view, setView] = useState<View>("folder");
  const [archivedOpen, setArchivedOpen] = useState(false);
  const active = projects.filter((p) => p.projectStatus !== "archived");
  const archived = projects.filter((p) => p.projectStatus === "archived");
  const isFlash = useCallback((id: number) => flash.has(nodeKey(id)), [flash]);

  return (
    <div className="card panel">
      <div className="panel-head">
        <span className="panel-title">项目</span>
        <span className="panel-meta">
          {active.filter((p) => p.projectStatus === "active").length} 个进行中
        </span>
        <div className="seg" role="radiogroup" aria-label="项目视图">
          <button
            type="button"
            role="radio"
            aria-checked={view === "folder"}
            className={view === "folder" ? "on" : undefined}
            title="文件夹视图"
            aria-label="文件夹视图"
            onClick={() => setView("folder")}
          >
            <Icon name="folder" />
          </button>
          <button
            type="button"
            role="radio"
            aria-checked={view === "tree"}
            className={view === "tree" ? "on" : undefined}
            title="树状视图"
            aria-label="树状视图"
            onClick={() => setView("tree")}
          >
            <Icon name="tree" />
          </button>
        </div>
      </div>
      <div className="panel-body">
        {projects.length === 0 ? (
          <div className="empty">还没有项目。切到项目模式说一句，比如：新建项目「读书计划」</div>
        ) : view === "folder" ? (
          <FolderView active={active} archived={archived} isFlash={isFlash} />
        ) : (
          <div className="project-trees">
            {active.length > 0 && (
              <Suspense fallback={treeFallback}>
                <TreeView projects={active} isFlash={isFlash} />
              </Suspense>
            )}
            {archived.length > 0 && (
              <>
                <button
                  type="button"
                  className={`sec${archivedOpen ? "" : " collapsed"}`}
                  aria-expanded={archivedOpen}
                  onClick={() => setArchivedOpen((value) => !value)}
                >
                  <Icon name="chevron-down" />
                  已归档 <span className="n">{archived.length}</span>
                </button>
                {archivedOpen && (
                  <Suspense fallback={treeFallback}>
                    <TreeView projects={archived} isFlash={isFlash} />
                  </Suspense>
                )}
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
