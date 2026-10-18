//! API Key 只存 Windows 凭据管理器，不写入配置、数据库或备份。
//! 凭据按“服务商 + 接口 origin”隔离；开发版与正式版使用不同的服务名。

use std::collections::HashMap;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::llm::{Endpoint, ProviderId};

pub trait SecretStore: Send + Sync {
    fn get(&self, endpoint: &Endpoint) -> AppResult<Option<String>>;
    fn set(&self, endpoint: &Endpoint, key: &str) -> AppResult<()>;
    fn delete(&self, endpoint: &Endpoint) -> AppResult<()>;
}

pub struct KeyringStore {
    service: &'static str,
}

impl KeyringStore {
    pub fn new(dev: bool) -> Self {
        Self {
            service: if dev { "Cicada-dev" } else { "Cicada" },
        }
    }

    fn entry(&self, endpoint: &Endpoint) -> AppResult<keyring::Entry> {
        keyring::Entry::new(self.service, &credential_name(endpoint)?).map_err(cred_err)
    }

    fn legacy_entry(&self, provider: ProviderId) -> AppResult<keyring::Entry> {
        keyring::Entry::new(self.service, &legacy_name(provider)).map_err(cred_err)
    }

    /// 旧版凭据没有记录接口地址。只允许把它迁移到该服务商的官方默认 origin；
    /// 自定义地址必须由用户重新填写，避免恢复配置后把旧 Key 发往其他主机。
    fn migrate_legacy(&self, endpoint: &Endpoint) -> AppResult<Option<String>> {
        if !legacy_origin_allowed(endpoint) {
            return Ok(None);
        }
        let legacy = self.legacy_entry(endpoint.provider)?;
        let key = match legacy.get_password() {
            Ok(key) => key,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(e) => return Err(cred_err(e)),
        };
        self.entry(endpoint)?.set_password(&key).map_err(cred_err)?;
        // 新凭据已安全落盘；清理失败不会阻止使用已迁移的凭据。
        let _ = legacy.delete_credential();
        Ok(Some(key))
    }
}

fn cred_err(e: keyring::Error) -> AppError {
    AppError::Invalid(format!("访问 Windows 凭据管理器失败：{e}"))
}

/// URL 的 origin 是凭据边界：同一主机下的不同 API 路径共用 Key，主机或端口变化则隔离。
pub(crate) fn endpoint_origin(endpoint: &Endpoint) -> AppResult<String> {
    let url = reqwest::Url::parse(endpoint.base_url.trim())
        .map_err(|_| AppError::Invalid("Base URL 格式不正确".into()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::Invalid("Base URL 格式不正确".into()));
    }
    Ok(url.origin().ascii_serialization())
}

pub(crate) fn same_credential_scope(a: &Endpoint, b: &Endpoint) -> bool {
    a.provider == b.provider
        && matches!(
            (endpoint_origin(a), endpoint_origin(b)),
            (Ok(a), Ok(b)) if a == b
        )
}

fn credential_name(endpoint: &Endpoint) -> AppResult<String> {
    let origin = endpoint_origin(endpoint)?;
    let digest = Sha256::digest(origin.as_bytes());
    Ok(format!(
        "api-key-v2:{}:{digest:x}",
        endpoint.provider.as_str()
    ))
}

fn legacy_name(provider: ProviderId) -> String {
    format!("api-key:{}", provider.as_str())
}

fn legacy_origin_allowed(endpoint: &Endpoint) -> bool {
    let default = endpoint.provider.default_base_url();
    if default.is_empty() {
        return false;
    }
    let default_endpoint = Endpoint {
        provider: endpoint.provider,
        base_url: default.into(),
        model: String::new(),
    };
    same_credential_scope(endpoint, &default_endpoint)
}

impl SecretStore for KeyringStore {
    fn get(&self, endpoint: &Endpoint) -> AppResult<Option<String>> {
        match self.entry(endpoint)?.get_password() {
            Ok(k) => Ok(Some(k)),
            Err(keyring::Error::NoEntry) => self.migrate_legacy(endpoint),
            Err(e) => Err(cred_err(e)),
        }
    }

    fn set(&self, endpoint: &Endpoint, key: &str) -> AppResult<()> {
        self.entry(endpoint)?.set_password(key).map_err(cred_err)
    }

    fn delete(&self, endpoint: &Endpoint) -> AppResult<()> {
        match self.entry(endpoint)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(cred_err(e)),
        }
        // 旧版 provider-only 凭据没有安全的接口归属；用户主动清除时一并移除。
        match self.legacy_entry(endpoint.provider)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(cred_err(e)),
        }
        Ok(())
    }
}

/// 测试用：只存在内存里
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, endpoint: &Endpoint) -> AppResult<Option<String>> {
        let name = credential_name(endpoint)?;
        let mut values = self.0.lock().unwrap();
        if let Some(key) = values.get(&name) {
            return Ok(Some(key.clone()));
        }
        if !legacy_origin_allowed(endpoint) {
            return Ok(None);
        }
        let Some(key) = values.remove(&legacy_name(endpoint.provider)) else {
            return Ok(None);
        };
        values.insert(name, key.clone());
        Ok(Some(key))
    }

    fn set(&self, endpoint: &Endpoint, key: &str) -> AppResult<()> {
        self.0
            .lock()
            .unwrap()
            .insert(credential_name(endpoint)?, key.to_string());
        Ok(())
    }

    fn delete(&self, endpoint: &Endpoint) -> AppResult<()> {
        let mut values = self.0.lock().unwrap();
        values.remove(&credential_name(endpoint)?);
        values.remove(&legacy_name(endpoint.provider));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(provider: ProviderId, base_url: &str) -> Endpoint {
        Endpoint {
            provider,
            base_url: base_url.into(),
            model: "model".into(),
        }
    }

    #[test]
    fn 凭据按服务商和_origin_隔离() {
        let store = MemoryStore::default();
        let first = endpoint(ProviderId::Deepseek, "https://API.deepseek.com:443/v1");
        let same_origin = endpoint(ProviderId::Deepseek, "https://api.deepseek.com/other");
        let other_host = endpoint(ProviderId::Deepseek, "https://example.com/v1");
        let other_provider = endpoint(ProviderId::Custom, "https://api.deepseek.com/v1");

        store.set(&first, "secret").unwrap();
        assert_eq!(store.get(&same_origin).unwrap().as_deref(), Some("secret"));
        assert!(store.get(&other_host).unwrap().is_none());
        assert!(store.get(&other_provider).unwrap().is_none());
    }

    #[test]
    fn 旧版凭据只迁移到官方默认_origin() {
        let store = MemoryStore::default();
        store
            .0
            .lock()
            .unwrap()
            .insert(legacy_name(ProviderId::Deepseek), "legacy-secret".into());
        let custom = endpoint(ProviderId::Deepseek, "https://proxy.example.com/v1");
        assert!(store.get(&custom).unwrap().is_none());

        let official = endpoint(ProviderId::Deepseek, "https://api.deepseek.com/v1");
        assert_eq!(
            store.get(&official).unwrap().as_deref(),
            Some("legacy-secret")
        );
        assert!(
            !store
                .0
                .lock()
                .unwrap()
                .contains_key(&legacy_name(ProviderId::Deepseek))
        );

        store.0.lock().unwrap().insert(
            legacy_name(ProviderId::Deepseek),
            "stale-legacy-secret".into(),
        );
        store.delete(&custom).unwrap();
        assert!(
            !store
                .0
                .lock()
                .unwrap()
                .contains_key(&legacy_name(ProviderId::Deepseek))
        );
    }
}
