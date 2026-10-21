//! 所有应用命令必须登记；构建生成权限，运行时再次按宿主窗口身份限制。
pub const COMMANDS: &[&str] = &[
    "get_bootstrap",
    "complete_onboarding",
    "get_board",
    "get_heatmap",
    "get_day_completions",
    "set_task_done",
    "undo_last",
    "set_appearance",
    "window_ready",
    "submit_input",
    "confirm_pending",
    "cancel_pending",
    "list_inbox",
    "list_input_history",
    "retry_inbox",
    "discard_inbox",
    "get_model_settings",
    "get_token_usage",
    "get_preferences",
    "save_preferences",
    "get_prompts",
    "save_prompt",
    "reset_prompt",
    "preview_prompt",
    "suggest_prompt",
    "export_data",
    "save_export",
    "import_data",
    "list_operation_log",
    "save_model_settings",
    "clear_api_key",
    "test_model",
    "open_folder",
    "list_backups",
    "create_backup",
    "restore_backup",
    "get_greeting",
    "get_ui_copy",
    "get_update",
    "get_update_status",
    "check_update",
    "install_update",
    "list_reports",
    "read_report",
    "get_desktop_bootstrap",
    "get_sticky_state",
    "get_sticky_board",
    "update_sticky",
    "close_sticky",
    "create_sticky",
    "set_sticky_task_done",
    "open_main",
    "show_quick_capture",
    "hide_quick_capture",
    "submit_capture",
];
pub const STICKY: &[&str] = &[
    "list_input_history",
    "get_ui_copy",
    "get_desktop_bootstrap",
    "get_sticky_state",
    "get_sticky_board",
    "update_sticky",
    "close_sticky",
    "set_sticky_task_done",
    "open_main",
    "submit_capture",
];
pub const QUICK: &[&str] = &[
    "list_input_history",
    "get_ui_copy",
    "get_desktop_bootstrap",
    "submit_capture",
    "hide_quick_capture",
    "open_main",
];
pub fn allowed(label: &str, command: &str) -> bool {
    if label == "main" {
        COMMANDS.contains(&command)
    } else if label == "quick" {
        QUICK.contains(&command)
    } else if label
        .strip_prefix("sticky-")
        .and_then(|s| s.parse::<i64>().ok())
        .is_some_and(|id| id > 0)
    {
        STICKY.contains(&command)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 副窗口拒绝设置恢复与直接业务写命令() {
        for label in ["quick", "sticky-1"] {
            for command in [
                "get_bootstrap",
                "get_model_settings",
                "save_preferences",
                "save_prompt",
                "reset_prompt",
                "import_data",
                "export_data",
                "list_operation_log",
                "save_model_settings",
                "restore_backup",
                "create_backup",
                "confirm_pending",
                "undo_last",
                "submit_input",
                "set_task_done",
                "create_sticky",
            ] {
                assert!(!allowed(label, command), "{label} must reject {command}");
                assert!(allowed("main", command));
            }
            assert!(allowed(label, "submit_capture"));
        }
        assert!(!allowed("quick", "get_sticky_board"));
        assert!(!allowed("quick", "set_sticky_task_done"));
        assert!(!allowed("sticky-invalid", "submit_capture"));
        assert!(allowed("sticky-1", "set_sticky_task_done"));
        // 确认实际构建的子窗口capability没有授予完整core/default或隐含应用权限。
        for raw in [
            include_str!("capabilities/sticky.json"),
            include_str!("capabilities/quick.json"),
        ] {
            let cap: serde_json::Value = serde_json::from_str(raw).unwrap();
            let label = if cap["identifier"] == "quick-window" {
                "quick"
            } else {
                "sticky-1"
            };
            for p in cap["permissions"].as_array().unwrap() {
                let p = p.as_str().unwrap();
                assert!(!p.ends_with(":default"));
                if let Some(c) = p.strip_prefix("allow-") {
                    assert!(allowed(label, &c.replace('-', "_")));
                }
            }
        }
    }
}
