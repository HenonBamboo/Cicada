use serde::{Serialize, Serializer};

/// 应用统一错误类型。返回给前端时序列化为中文提示文本。
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("数据库错误：{0}")]
    Db(#[from] rusqlite::Error),
    #[error("文件读写失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("配置格式错误：{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
    /// 提交时发现相关数据已变化，需要重新预览
    #[error("{0}")]
    Conflict(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
