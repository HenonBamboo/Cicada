-- 收件箱重试与识图缓存（docs/产品设计.md 4.6）
ALTER TABLE inbox ADD COLUMN next_attempt_at TEXT;
-- 识图结果：重试时不再重复识别
ALTER TABLE inbox ADD COLUMN vision_text TEXT;
