-- 可选的便利贴项目语境；原始输入仍完整保存在 content 中。
ALTER TABLE inbox ADD COLUMN target_project_id INTEGER;
