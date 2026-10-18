//! 便利贴窗口布局。只保存窗口状态，不修改任务或项目；关闭按原型解释为隐藏。
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::Service;
use super::oplog::{self, Entry, Source};
use crate::error::{AppError, AppResult};

/// 位置、尺寸均为逻辑像素；多屏位置允许负数。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StickyView {
    pub id: i64,
    pub kind: String,
    pub node_id: Option<i64>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: u32,
    pub height: u32,
    pub color: String,
    pub opacity: f64,
    pub pinned: bool,
    pub locked: bool,
    pub auto_hide: bool,
    pub visible: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StickyPatch {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub color: Option<String>,
    pub opacity: Option<f64>,
    pub pinned: Option<bool>,
    pub locked: Option<bool>,
    pub auto_hide: Option<bool>,
    pub visible: Option<bool>,
}

const COLS: &str = "id,kind,node_id,x,y,width,height,color,opacity,pinned,locked,auto_hide,visible";

fn read(row: &Row<'_>) -> rusqlite::Result<StickyView> {
    Ok(StickyView {
        id: row.get(0)?,
        kind: row.get(1)?,
        node_id: row.get(2)?,
        x: row.get(3)?,
        y: row.get(4)?,
        width: row.get(5)?,
        height: row.get(6)?,
        color: row.get(7)?,
        opacity: row.get(8)?,
        pinned: row.get(9)?,
        locked: row.get(10)?,
        auto_hide: row.get(11)?,
        visible: row.get(12)?,
    })
}

fn validate(view: &StickyView) -> AppResult<()> {
    if !(200..=4096).contains(&view.width) || !(160..=4096).contains(&view.height) {
        return Err(AppError::Invalid(
            "便利贴宽度须为 200–4096，高度须为 160–4096 逻辑像素".into(),
        ));
    }
    if !matches!(
        view.color.as_str(),
        "theme" | "amber" | "pink" | "blue" | "green"
    ) {
        return Err(AppError::Invalid(
            "便利贴颜色须为跟随主题、黄、粉、蓝或绿".into(),
        ));
    }
    if !view.opacity.is_finite() || !(0.55..=1.0).contains(&view.opacity) {
        return Err(AppError::Invalid("便利贴透明度须为 55% 到 100%".into()));
    }
    Ok(())
}

impl Service<'_> {
    /// 包含隐藏的便利贴，供启动恢复和托盘显示/隐藏使用。
    pub fn stickies(&self) -> AppResult<Vec<StickyView>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLS} FROM stickies ORDER BY id"))?;
        Ok(stmt.query_map([], read)?.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn sticky_get(&self, id: i64) -> AppResult<StickyView> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM stickies WHERE id=?1"),
                [id],
                read,
            )
            .optional()?
            .ok_or_else(|| AppError::Invalid("这张便利贴已不存在".into()))
    }

    /// 支持多张；调用方需要复用时先按 kind/node_id 查找已有记录。
    pub fn sticky_create(&self, kind: &str, node_id: Option<i64>) -> AppResult<StickyView> {
        match (kind, node_id) {
            ("today", None) => {}
            ("project", Some(id)) if id > 0 => {
                let valid:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?1 AND depth=1 AND parent_id IS NULL)",[id],|r|r.get(0))?;
                if !valid {
                    return Err(AppError::Invalid(
                        "请选择仍存在的项目，不能为普通子节点创建项目便利贴".into(),
                    ));
                }
            }
            _ => {
                return Err(AppError::Invalid(
                    "便利贴类型须为 today 或 project；只有项目便利贴需要项目编号".into(),
                ));
            }
        }
        let tx = self.conn.unchecked_transaction()?;
        let now = self.now_str();
        tx.execute("INSERT INTO stickies(kind,node_id,width,height,color,opacity,pinned,auto_hide,created_at,updated_at) VALUES(?1,?2,280,360,?4,?5,1,?6,?3,?3)",params![kind,node_id,now,self.sticky_defaults.color,self.sticky_defaults.opacity,self.sticky_defaults.auto_hide])?;
        let id = tx.last_insert_rowid();
        oplog::record(
            &tx,
            Entry {
                group_id: &oplog::new_group_id(),
                source: Source::System,
                op: "sticky.create",
                entity: "stickies",
                entity_id: Some(id),
                before: None,
                after: oplog::row_json(&tx, "stickies", id)?,
                summary: "创建便利贴窗口".into(),
                created_at: &now,
            },
        )?;
        let view = tx.query_row(
            &format!("SELECT {COLS} FROM stickies WHERE id=?1"),
            [id],
            read,
        )?;
        tx.commit()?;
        Ok(view)
    }

    /// 仅首次没有任何记录时创建默认今日便利贴，不重新显示用户已关闭的窗口。
    pub fn sticky_ensure_default(&self) -> AppResult<Vec<StickyView>> {
        let all = self.stickies()?;
        if all.is_empty() {
            Ok(vec![self.sticky_create("today", None)?])
        } else {
            Ok(all)
        }
    }

    pub fn sticky_update(&self, id: i64, patch: StickyPatch) -> AppResult<StickyView> {
        let previous = self.sticky_get(id)?;
        let mut view = previous.clone();
        if let Some(v) = patch.x {
            view.x = Some(v);
        }
        if let Some(v) = patch.y {
            view.y = Some(v);
        }
        if let Some(v) = patch.width {
            view.width = v;
        }
        if let Some(v) = patch.height {
            view.height = v;
        }
        if let Some(v) = patch.color {
            view.color = v;
        }
        if let Some(v) = patch.opacity {
            view.opacity = v;
        }
        if let Some(v) = patch.pinned {
            view.pinned = v;
        }
        if let Some(v) = patch.locked {
            view.locked = v;
        }
        if let Some(v) = patch.auto_hide {
            view.auto_hide = v;
        }
        if let Some(v) = patch.visible {
            view.visible = v;
        }
        validate(&view)?;
        if view == previous {
            return Ok(view);
        }
        let tx = self.conn.unchecked_transaction()?;
        let before = oplog::row_json(&tx, "stickies", id)?;
        let now = self.now_str();
        tx.execute("UPDATE stickies SET x=?2,y=?3,width=?4,height=?5,color=?6,opacity=?7,pinned=?8,locked=?9,auto_hide=?10,visible=?11,updated_at=?12 WHERE id=?1",params![id,view.x,view.y,view.width,view.height,view.color,view.opacity,view.pinned,view.locked,view.auto_hide,view.visible,now])?;
        oplog::record(
            &tx,
            Entry {
                group_id: &oplog::new_group_id(),
                source: Source::System,
                op: "sticky.layout",
                entity: "stickies",
                entity_id: Some(id),
                before,
                after: oplog::row_json(&tx, "stickies", id)?,
                summary: "更新便利贴窗口布局".into(),
                created_at: &now,
            },
        )?;
        tx.commit()?;
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::service::testing::{clock, db};

    #[test]
    fn 便利贴恢复隐藏状态多张与参数校验() {
        let db = db();
        let config = AppConfig::default();
        let c = clock("2026-10-18 10:00:00");
        let s = Service::new(&db, &c, &config).unwrap();
        let first = s.sticky_ensure_default().unwrap().remove(0);
        assert_eq!((first.width, first.height), (280, 360));
        let saved = s
            .sticky_update(
                first.id,
                StickyPatch {
                    x: Some(-300),
                    y: Some(80),
                    width: Some(320),
                    height: Some(400),
                    color: Some("pink".into()),
                    opacity: Some(0.55),
                    pinned: Some(false),
                    locked: Some(true),
                    auto_hide: Some(true),
                    visible: Some(false),
                },
            )
            .unwrap();
        let reopened = Service::new(&db, &c, &config).unwrap();
        assert_eq!(
            reopened.sticky_ensure_default().unwrap(),
            vec![saved.clone()]
        );
        assert_ne!(reopened.sticky_create("today", None).unwrap().id, first.id);
        db.execute("INSERT INTO nodes(depth,title,project_status,created_at,updated_at) VALUES(1,'项目','active','2026-10-18T10:00:00','2026-10-18T10:00:00')",[]).unwrap();
        let project = db.last_insert_rowid();
        assert_eq!(
            reopened
                .sticky_create("project", Some(project))
                .unwrap()
                .node_id,
            Some(project)
        );
        db.execute("INSERT INTO nodes(parent_id,depth,title,created_at,updated_at) VALUES(?1,2,'子项','2026-10-18T10:00:00','2026-10-18T10:00:00')",[project]).unwrap();
        let child = db.last_insert_rowid();
        assert!(reopened.sticky_create("project", Some(child)).is_err());
        assert!(reopened.sticky_create("today", Some(project)).is_err());
        assert!(reopened.sticky_create("project", None).is_err());
        for patch in [
            StickyPatch {
                opacity: Some(f64::NAN),
                ..Default::default()
            },
            StickyPatch {
                opacity: Some(0.5),
                ..Default::default()
            },
            StickyPatch {
                width: Some(1),
                ..Default::default()
            },
            StickyPatch {
                color: Some("red".into()),
                ..Default::default()
            },
        ] {
            assert!(reopened.sticky_update(first.id, patch).is_err());
        }
        assert_eq!(reopened.sticky_get(first.id).unwrap(), saved);
        assert!(reopened.undo_preview().unwrap().is_none());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM op_log WHERE entity='stickies' AND source!='system'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
