//! GitCode 便携 EXE 更新。网络元数据只用于定位，签名清单才是信任边界。
mod helper;

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{AppError, AppResult};

const API: &str = "https://api.gitcode.com/api/v5/repos/HenonBamboo_YYJ/Cicada";
const MAX_EXE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Manifest {
    schema_version: u32,
    version: String,
    tag: String,
    asset: String,
    size: u64,
    sha256: String,
    published_at: String,
    notes: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    version: String,
    notes: String,
    published_at: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available,
    Error,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatusView {
    status: UpdateStatus,
    checked_at: Option<String>,
    update: Option<UpdateView>,
    error: Option<String>,
}

impl From<&Manifest> for UpdateView {
    fn from(value: &Manifest) -> Self {
        Self {
            version: value.version.clone(),
            notes: value.notes.clone(),
            published_at: value.published_at.clone(),
        }
    }
}

#[derive(Clone)]
struct Candidate {
    manifest: Manifest,
    bytes: Vec<u8>,
    signature: String,
}

#[derive(Default)]
pub struct UpdateState {
    candidate: Mutex<Option<Candidate>>,
    status: Mutex<UpdateStatusView>,
    operation: tokio::sync::Mutex<()>,
    installing: AtomicBool,
}

fn invalid(message: &str) -> AppError {
    AppError::Invalid(message.into())
}

fn http_error(_: reqwest::Error) -> AppError {
    invalid("更新服务暂时不可用，请稍后再试")
}

fn client() -> AppResult<reqwest::Client> {
    // 附件可能跳转到 GitCode 对象存储；只接受 HTTPS 跳转，不携带任何用户凭据。
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(180))
        .user_agent(concat!("Cicada/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(http_error)
}

fn asset_url(tag: &str, name: &str) -> String {
    // tag 和 name 已由签名清单的严格格式校验；调用者不能传任意地址。
    format!("{API}/releases/{tag}/attach_files/{name}/download")
}

async fn bounded_bytes(client: &reqwest::Client, url: &str, limit: usize) -> AppResult<Vec<u8>> {
    let response = client.get(url).send().await.map_err(http_error)?;
    if !response.status().is_success() {
        return Err(invalid("更新文件暂时不可用"));
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(invalid("更新文件大小异常"));
    }
    let mut stream = response.bytes_stream();
    let mut output = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(http_error)?;
        if output.len().saturating_add(chunk.len()) > limit {
            return Err(invalid("更新文件大小异常"));
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

fn verify_manifest_with_key(bytes: &[u8], signature: &str, key: &[u8]) -> AppResult<Manifest> {
    if bytes.len() > MAX_MANIFEST_BYTES || key.len() != 32 {
        return Err(invalid("更新清单格式异常"));
    }
    let signature = STANDARD
        .decode(signature.trim())
        .map_err(|_| invalid("更新签名格式无效"))?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(bytes, &signature)
        .map_err(|_| invalid("更新签名校验失败，已保留当前版本"))?;
    let manifest: Manifest = serde_json::from_slice(bytes)?;
    let version =
        semver::Version::parse(&manifest.version).map_err(|_| invalid("更新版本格式无效"))?;
    if manifest.schema_version != 1
        || !version.pre.is_empty()
        || !version.build.is_empty()
        || version.to_string() != manifest.version
        || manifest.tag != format!("v{version}")
        || manifest.asset != format!("Cicada-{version}-windows-x64.exe")
        || manifest.size == 0
        || manifest.size > MAX_EXE_BYTES
        || manifest.sha256.len() != 64
        || !manifest
            .sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || manifest.notes.chars().count() > 6000
        || chrono::DateTime::parse_from_rfc3339(&manifest.published_at).is_err()
    {
        return Err(invalid("更新清单内容无效"));
    }
    Ok(manifest)
}

pub(super) fn verify_manifest(bytes: &[u8], signature: &str) -> AppResult<Manifest> {
    let key = STANDARD
        .decode(include_str!("public-key.txt").trim())
        .map_err(|_| invalid("更新公钥配置无效"))?;
    verify_manifest_with_key(bytes, signature, &key)
}

pub(super) fn hash_file(path: &Path) -> AppResult<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        digest.update(&chunk[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn verify_download(path: &Path, manifest: &Manifest) -> AppResult<()> {
    if std::fs::metadata(path)?.len() != manifest.size || hash_file(path)? != manifest.sha256 {
        return Err(invalid("更新文件校验失败，已保留当前版本"));
    }
    let mut header = [0; 2];
    File::open(path)?.read_exact(&mut header)?;
    if &header != b"MZ" {
        return Err(invalid("更新文件不是 Windows 程序"));
    }
    Ok(())
}

async fn check() -> AppResult<Option<Candidate>> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        #[serde(default)]
        prerelease: bool,
    }
    let client = client()?;
    let release = bounded_bytes(
        &client,
        &format!("{API}/releases/latest?type=latest"),
        512 * 1024,
    )
    .await?;
    let release: Release = serde_json::from_slice(&release)?;
    let Some(version) = release
        .tag_name
        .strip_prefix('v')
        .and_then(|v| semver::Version::parse(v).ok())
    else {
        return Err(invalid("更新版本格式无效"));
    };
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|_| invalid("当前版本格式无效"))?;
    if release.prerelease
        || !version.pre.is_empty()
        || !version.build.is_empty()
        || version <= current
    {
        return Ok(None);
    }
    if release.tag_name != format!("v{version}") {
        return Err(invalid("更新标签格式无效"));
    }
    let bytes = bounded_bytes(
        &client,
        &asset_url(&release.tag_name, "latest.json"),
        MAX_MANIFEST_BYTES,
    )
    .await?;
    let signature = bounded_bytes(
        &client,
        &asset_url(&release.tag_name, "latest.json.sig"),
        256,
    )
    .await?;
    let signature = String::from_utf8(signature).map_err(|_| invalid("更新签名格式无效"))?;
    let manifest = verify_manifest(&bytes, &signature)?;
    if manifest.tag != release.tag_name {
        return Err(invalid("更新清单与发布版本不一致"));
    }
    Ok(Some(Candidate {
        manifest,
        bytes,
        signature,
    }))
}

/// setup 中调用；调试版不请求发布服务器。
pub fn startup(app: &AppHandle) {
    app.manage(UpdateState::default());
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(message) = helper::cleanup_completed() {
            if let Ok(mut slot) = app.state::<crate::state::AppState>().desktop_error.lock() {
                *slot = Some(message.into());
            }
            let _ = app.emit_to("main", "desktop-error", message);
        }
        let _ = check_now(&app, &app.state::<UpdateState>()).await;
    });
}

pub fn get(state: &UpdateState) -> AppResult<Option<UpdateView>> {
    Ok(state
        .candidate
        .lock()
        .map_err(|_| invalid("更新状态暂时不可用"))?
        .as_ref()
        .map(|c| UpdateView::from(&c.manifest)))
}

pub fn get_status(state: &UpdateState) -> AppResult<UpdateStatusView> {
    state
        .status
        .lock()
        .map(|view| view.clone())
        .map_err(|_| invalid("更新状态暂时不可用"))
}

fn publish_status(
    app: &AppHandle,
    state: &UpdateState,
    view: UpdateStatusView,
) -> AppResult<UpdateStatusView> {
    *state
        .status
        .lock()
        .map_err(|_| invalid("更新状态暂时不可用"))? = view.clone();
    let _ = app.emit_to("main", "update-status", &view);
    Ok(view)
}

fn completed_status(
    result: &AppResult<Option<Candidate>>,
    previous: Option<UpdateView>,
) -> UpdateStatusView {
    let (status, update, error) = match result {
        Ok(Some(candidate)) => (
            UpdateStatus::Available,
            Some(UpdateView::from(&candidate.manifest)),
            None,
        ),
        Ok(None) => (UpdateStatus::UpToDate, None, None),
        Err(error) => (UpdateStatus::Error, previous, Some(error.to_string())),
    };
    UpdateStatusView {
        status,
        update,
        error,
        checked_at: Some(chrono::Utc::now().to_rfc3339()),
    }
}

/// 只由本次启动和用户主动检查调用；并发点击复用在途检查，不添加定时或聚焦检查。
pub async fn check_now(app: &AppHandle, state: &UpdateState) -> AppResult<UpdateStatusView> {
    let Ok(_operation) = state.operation.try_lock() else {
        if state.installing.load(Ordering::Acquire) {
            return Err(invalid("正在更新，请稍候"));
        }
        return get_status(state);
    };
    let mut checking = get_status(state)?;
    checking.status = UpdateStatus::Checking;
    checking.error = None;
    publish_status(app, state, checking)?;
    let previous = get(state)?;
    let result = check().await;
    let status = completed_status(&result, previous);
    if let Ok(candidate) = result {
        let view = candidate
            .as_ref()
            .map(|value| UpdateView::from(&value.manifest));
        *state
            .candidate
            .lock()
            .map_err(|_| invalid("更新状态暂时不可用"))? = candidate;
        if let Some(view) = view {
            let _ = app.emit_to("main", "update-available", view);
        }
    }
    publish_status(app, state, status)
}

struct InstallGuard<'a>(&'a AtomicBool);
impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub async fn install(app: &AppHandle, state: &UpdateState) -> AppResult<()> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| invalid("正在检查或安装更新，请稍候"))?;
    if state.installing.swap(true, Ordering::AcqRel) {
        return Err(invalid("正在准备更新，请稍候"));
    }
    let _guard = InstallGuard(&state.installing);
    let candidate = state
        .candidate
        .lock()
        .map_err(|_| invalid("更新状态暂时不可用"))?
        .clone()
        .ok_or_else(|| invalid("当前没有可安装的更新"))?;
    // 临时目录与 EXE 同卷。创建失败会直接返回，不要求提权或改写其它目录。
    let prepared = helper::Prepared::create(&candidate.bytes, &candidate.signature)?;
    let download = prepared.directory.join("download.exe");
    let result = async {
        let response = client()?
            .get(asset_url(
                &candidate.manifest.tag,
                &candidate.manifest.asset,
            ))
            .send()
            .await
            .map_err(http_error)?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|n| n != candidate.manifest.size)
        {
            return Err(invalid("更新文件暂时不可用或大小异常"));
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&download)?;
        let mut size = 0_u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(http_error)?;
            size = size.saturating_add(chunk.len() as u64);
            if size > candidate.manifest.size {
                return Err(invalid("更新文件大小异常"));
            }
            file.write_all(&chunk)?;
        }
        file.sync_all()?;
        drop(file);
        verify_download(&download, &candidate.manifest)?;
        prepared.launch().await
    }
    .await;
    if result.is_err() {
        prepared.cleanup();
    }
    result?;
    app.exit(0);
    Ok(())
}

/// 必须在 Tauri 单实例插件之前调用。更新辅助进程不创建窗口、不打开数据库。
pub fn handle_cli() -> bool {
    helper::handle_cli()
}

/// 仅在主窗口前端报告就绪后确认新版本启动成功。
pub fn mark_ready() {
    helper::mark_ready();
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::{
        rand::SystemRandom,
        signature::{Ed25519KeyPair, KeyPair},
    };

    fn signed() -> (Ed25519KeyPair, Vec<u8>, String) {
        let key = Ed25519KeyPair::from_pkcs8(
            Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        let bytes = serde_json::to_vec(&Manifest {
            schema_version: 1,
            version: "0.3.0".into(),
            tag: "v0.3.0".into(),
            asset: "Cicada-0.3.0-windows-x64.exe".into(),
            size: 4,
            sha256: format!("{:x}", Sha256::digest(b"MZok")),
            published_at: "2026-10-20T00:00:00Z".into(),
            notes: "更新".into(),
        })
        .unwrap();
        let signature = STANDARD.encode(key.sign(&bytes).as_ref());
        (key, bytes, signature)
    }

    #[test]
    fn signed_manifest_rejects_tampering_and_path_escape() {
        let (key, bytes, signature) = signed();
        assert!(verify_manifest_with_key(&bytes, &signature, key.public_key().as_ref()).is_ok());
        let mut tampered = bytes.clone();
        tampered.push(b' ');
        assert!(
            verify_manifest_with_key(&tampered, &signature, key.public_key().as_ref()).is_err()
        );
        let mut manifest: Manifest = serde_json::from_slice(&bytes).unwrap();
        manifest.asset = "../other.exe".into();
        let escaped = serde_json::to_vec(&manifest).unwrap();
        let signature = STANDARD.encode(key.sign(&escaped).as_ref());
        assert!(verify_manifest_with_key(&escaped, &signature, key.public_key().as_ref()).is_err());
    }

    #[test]
    fn download_checks_size_hash_and_windows_header() {
        let (_, bytes, _) = signed();
        let manifest: Manifest = serde_json::from_slice(&bytes).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("update.exe");
        std::fs::write(&path, b"MZok").unwrap();
        assert!(verify_download(&path, &manifest).is_ok());
        std::fs::write(&path, b"MZno").unwrap();
        assert!(verify_download(&path, &manifest).is_err());
        std::fs::write(&path, b"MZok-extra").unwrap();
        assert!(verify_download(&path, &manifest).is_err());
    }

    #[test]
    fn 检查结果区分最新可用和失败且保留签名候选() {
        let (_, bytes, signature) = signed();
        let candidate = Candidate {
            manifest: serde_json::from_slice(&bytes).unwrap(),
            bytes,
            signature,
        };
        let available = completed_status(&Ok(Some(candidate)), None);
        assert_eq!(available.status, UpdateStatus::Available);
        assert_eq!(available.update.as_ref().unwrap().version, "0.3.0");
        assert!(available.checked_at.is_some());
        let failed = completed_status(&Err(invalid("网络不可用")), available.update);
        assert_eq!(failed.status, UpdateStatus::Error);
        assert_eq!(failed.error.as_deref(), Some("网络不可用"));
        assert!(failed.update.is_some());
        let latest = completed_status(&Ok(None), failed.update);
        assert_eq!(latest.status, UpdateStatus::UpToDate);
        assert!(latest.update.is_none());
        assert!(latest.error.is_none());
    }
}
