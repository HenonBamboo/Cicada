//! 设置页数据操作；导出路径仅由原生保存框选择，前端不能指定写入位置。
use std::io::Write;
use std::path::Path;

use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use super::{EVENT_DATA_CHANGED, poisoned, with_service};
use crate::{
    backup,
    error::{AppError, AppResult},
    service::data::{ExportFile, ImportResult, OperationLog, Transfer},
    state::AppState,
};

#[tauri::command]
pub async fn export_data(state: State<'_, AppState>, format: String) -> AppResult<ExportFile> {
    let _maintenance = state.maintenance.read().await;
    with_service(&state, |s| s.export_data(&format))
}

/// 导出快照后先释放数据库与维护锁，用户停留在保存框不会阻塞其它业务。
#[tauri::command]
pub async fn save_export(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    format: String,
) -> AppResult<Option<String>> {
    let data = {
        let _maintenance = state.maintenance.read().await;
        with_service(&state, |s| s.export_data(&format))?
    };
    tauri::async_runtime::spawn_blocking(move || {
        let (label, extension) = match format.as_str() {
            "markdown" => ("Markdown 文件", "md"),
            "json" => ("JSON 文件", "json"),
            _ => return Err(AppError::Invalid("仅支持 Markdown 或 JSON 导出".into())),
        };
        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("导出知了数据")
            .set_file_name(&data.file_name)
            .add_filter(label, &[extension])
            .blocking_save_file();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|_| AppError::Invalid("请选择本机文件保存位置".into()))?;
        write_export(&path, &data.content)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|_| AppError::Invalid("导出未能完成，请重试".into()))?
}

/// 同目录临时写入后再替换，写入失败时保留已有导出文件。
fn write_export(path: &Path, content: &str) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Invalid("保存位置无效".into()))?;
    let temporary = parent.join(format!(".cicada-export-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> AppResult<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod export_tests {
    use super::*;

    #[test]
    fn 导出中文原文且替换失败不残留临时文件() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("知了.json");
        std::fs::write(&path, "old").unwrap();
        write_export(&path, "{\"备注\":\"保留 **Markdown**\"}").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"备注\":\"保留 **Markdown**\"}"
        );
        assert!(write_export(directory.path(), "blocked").is_err());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

#[tauri::command]
pub async fn import_data(
    app: AppHandle,
    state: State<'_, AppState>,
    content: String,
) -> AppResult<ImportResult> {
    let data = Transfer::parse(&content)?;
    let _maintenance = state.maintenance.write().await;
    let config = state.config.lock().map_err(poisoned)?;
    let conn = state.db.lock().map_err(poisoned)?;
    backup::create(&state.paths, &conn, state.clock.as_ref(), "pre-import")?;
    let result =
        crate::service::Service::new(&conn, state.clock.as_ref(), &config)?.import_data(data)?;
    drop(conn);
    drop(config);
    drop(_maintenance);
    let _ = app.emit(
        EVENT_DATA_CHANGED,
        serde_json::json!({"tasks":[],"nodes":[]}),
    );
    Ok(result)
}

#[tauri::command]
pub async fn list_operation_log(state: State<'_, AppState>) -> AppResult<Vec<OperationLog>> {
    let _maintenance = state.maintenance.read().await;
    with_service(&state, |s| s.operation_log())
}
