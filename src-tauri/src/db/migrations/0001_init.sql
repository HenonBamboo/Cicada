-- 知了 · 初始数据结构，见 docs/产品设计.md 11.2
-- 约定：日期 YYYY-MM-DD（业务日期）；时刻 YYYY-MM-DDTHH:MM:SS（本地时间）

-- 项目与节点：parent_id 为空即项目（第 1 层），最多 3 层
CREATE TABLE nodes (
  id             INTEGER PRIMARY KEY,
  parent_id      INTEGER REFERENCES nodes(id) ON DELETE CASCADE,
  depth          INTEGER NOT NULL CHECK (depth BETWEEN 1 AND 3),
  title          TEXT    NOT NULL,
  status         TEXT    NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'done')),
  -- 只有项目（depth = 1）有项目状态
  project_status TEXT CHECK (project_status IN ('active', 'done', 'archived')),
  note           TEXT,
  due_at         TEXT,
  sort_order     INTEGER NOT NULL DEFAULT 0,
  created_at     TEXT    NOT NULL,
  updated_at     TEXT    NOT NULL,
  done_at        TEXT,
  version        INTEGER NOT NULL DEFAULT 1,
  CHECK ((parent_id IS NULL) = (depth = 1)),
  CHECK ((depth = 1) = (project_status IS NOT NULL))
);
CREATE INDEX idx_nodes_parent ON nodes(parent_id, sort_order);

-- 重复任务规则
CREATE TABLE recur_rules (
  id                  INTEGER PRIMARY KEY,
  title               TEXT    NOT NULL,
  kind                TEXT    NOT NULL CHECK (kind IN ('daily', 'weekdays', 'weekly', 'monthly', 'yearly')),
  -- 规则参数 JSON，如 {"weekday":1}、{"day":1}、{"month":3,"day":8}
  params              TEXT    NOT NULL DEFAULT '{}',
  remind_time         TEXT,
  note                TEXT,
  node_id             INTEGER REFERENCES nodes(id) ON DELETE SET NULL,
  start_date          TEXT    NOT NULL,
  end_date            TEXT,
  active              INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
  last_generated_date TEXT,
  created_at          TEXT    NOT NULL,
  updated_at          TEXT    NOT NULL,
  version             INTEGER NOT NULL DEFAULT 1
);

-- 任务
CREATE TABLE tasks (
  id             INTEGER PRIMARY KEY,
  title          TEXT    NOT NULL,
  planned_date   TEXT    NOT NULL,
  status         TEXT    NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'done')),
  postponed_days INTEGER NOT NULL DEFAULT 0,
  due_at         TEXT,
  remind_at      TEXT,
  reminded_at    TEXT,
  note           TEXT,
  recur_rule_id  INTEGER REFERENCES recur_rules(id) ON DELETE SET NULL,
  node_id        INTEGER REFERENCES nodes(id) ON DELETE SET NULL,
  created_at     TEXT    NOT NULL,
  updated_at     TEXT    NOT NULL,
  done_at        TEXT,
  -- 完成时所在的业务日期（已完成分区、日切归档用）
  done_date      TEXT,
  version        INTEGER NOT NULL DEFAULT 1,
  CHECK ((status = 'done') = (done_at IS NOT NULL))
);
CREATE INDEX idx_tasks_planned ON tasks(status, planned_date);
CREATE INDEX idx_tasks_done_date ON tasks(done_date);
CREATE INDEX idx_tasks_remind ON tasks(remind_at) WHERE remind_at IS NOT NULL AND reminded_at IS NULL;
-- 一个节点同时最多一条未完成的关联任务（7.4）
CREATE UNIQUE INDEX ux_tasks_open_node ON tasks(node_id) WHERE node_id IS NOT NULL AND status = 'open';

-- 完成事件：热力图和报告的数据来源。一件事只记一条；不设外键，删除对象后历史仍可读
CREATE TABLE completions (
  id            INTEGER PRIMARY KEY,
  business_date TEXT    NOT NULL,
  kind          TEXT    NOT NULL CHECK (kind IN ('task', 'node')),
  task_id       INTEGER,
  node_id       INTEGER,
  title         TEXT    NOT NULL,
  project_title TEXT,
  completed_at  TEXT    NOT NULL,
  op_group      TEXT,
  CHECK (task_id IS NOT NULL OR node_id IS NOT NULL)
);
CREATE INDEX idx_completions_date ON completions(business_date);
CREATE INDEX idx_completions_task ON completions(task_id);
CREATE INDEX idx_completions_node ON completions(node_id);

-- 操作日志：同一 group_id 为一组，按组撤销（4.5）
CREATE TABLE op_log (
  id         INTEGER PRIMARY KEY,
  group_id   TEXT    NOT NULL,
  source     TEXT    NOT NULL CHECK (source IN ('chat', 'manual', 'system', 'undo')),
  op         TEXT    NOT NULL,
  entity     TEXT    NOT NULL,
  entity_id  INTEGER,
  before     TEXT,
  after      TEXT,
  summary    TEXT    NOT NULL,
  created_at TEXT    NOT NULL,
  undone_at  TEXT
);
CREATE INDEX idx_op_log_group ON op_log(group_id);
CREATE INDEX idx_op_log_source ON op_log(source, created_at);

-- 收件箱：所有输入先进这里（4.6）
CREATE TABLE inbox (
  id             TEXT    PRIMARY KEY,
  source         TEXT    NOT NULL CHECK (source IN ('main', 'sticky', 'quick')),
  mode           TEXT    NOT NULL CHECK (mode IN ('task', 'project')),
  content        TEXT    NOT NULL,
  image_paths    TEXT    NOT NULL DEFAULT '[]',
  received_at    TEXT    NOT NULL,
  business_date  TEXT    NOT NULL,
  status         TEXT    NOT NULL DEFAULT 'queued'
                         CHECK (status IN ('queued', 'processing', 'awaiting', 'done', 'failed')),
  status_reason  TEXT,
  attempts       INTEGER NOT NULL DEFAULT 0,
  last_error     TEXT,
  result_summary TEXT,
  updated_at     TEXT    NOT NULL
);
CREATE INDEX idx_inbox_status ON inbox(status, received_at);

-- 待确认 / 待追问的变更计划（4.4）
CREATE TABLE pending_plans (
  id         INTEGER PRIMARY KEY,
  request_id TEXT    NOT NULL REFERENCES inbox(id) ON DELETE CASCADE,
  kind       TEXT    NOT NULL CHECK (kind IN ('confirm', 'clarify')),
  plan       TEXT    NOT NULL,
  -- 涉及对象及其版本号 JSON，如 [{"entity":"task","id":1,"version":3}]
  versions   TEXT    NOT NULL DEFAULT '[]',
  context    TEXT,
  preview    TEXT,
  created_at TEXT    NOT NULL,
  expires_at TEXT    NOT NULL
);
CREATE INDEX idx_pending_request ON pending_plans(request_id);

-- 报告索引，正文在 reports/ 下的 Markdown
CREATE TABLE reports (
  id           INTEGER PRIMARY KEY,
  kind         TEXT NOT NULL CHECK (kind IN ('weekly', 'monthly', 'yearly')),
  period       TEXT NOT NULL,
  start_date   TEXT NOT NULL,
  end_date     TEXT NOT NULL,
  path         TEXT NOT NULL,
  generated_at TEXT NOT NULL,
  UNIQUE (kind, period)
);

-- 问候语缓存
CREATE TABLE greetings (
  id            INTEGER PRIMARY KEY,
  business_date TEXT NOT NULL,
  slot          TEXT NOT NULL,
  reason        TEXT,
  text          TEXT NOT NULL,
  created_at    TEXT NOT NULL
);
CREATE INDEX idx_greetings_date ON greetings(business_date, slot);

-- 便利贴窗口状态
CREATE TABLE stickies (
  id         INTEGER PRIMARY KEY,
  kind       TEXT    NOT NULL CHECK (kind IN ('today', 'project')),
  node_id    INTEGER REFERENCES nodes(id) ON DELETE CASCADE,
  x          INTEGER,
  y          INTEGER,
  width      INTEGER NOT NULL DEFAULT 280,
  height     INTEGER NOT NULL DEFAULT 360,
  color      TEXT    NOT NULL DEFAULT 'theme',
  opacity    REAL    NOT NULL DEFAULT 1.0,
  pinned     INTEGER NOT NULL DEFAULT 0,
  locked     INTEGER NOT NULL DEFAULT 0,
  auto_hide  INTEGER NOT NULL DEFAULT 0,
  visible    INTEGER NOT NULL DEFAULT 1,
  created_at TEXT    NOT NULL,
  updated_at TEXT    NOT NULL,
  CHECK ((kind = 'project') = (node_id IS NOT NULL))
);

-- 运行状态：上次日切、上次备份等
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
