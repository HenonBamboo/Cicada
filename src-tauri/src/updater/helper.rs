//! 同卷替换与启动握手；只操作经过验证的固定文件名。
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use super::{Manifest, hash_file, invalid, verify_download, verify_manifest};
use crate::error::AppResult;

const PREFIX: &str = ".cicada-update-";
const HELPER_FLAG: &str = "--cicada-update-helper";
const READY_FLAG: &str = "--cicada-update-ready";
const FILES: &[&str] = &[
    "helper.exe",
    "download.exe",
    "plan.json",
    "helper-ready",
    "app-ready",
    "result.txt",
];

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    token: String,
    target: String,
    parent_pid: u32,
    original_sha256: String,
    manifest: String,
    signature: String,
}

pub(super) struct Prepared {
    pub directory: PathBuf,
}

fn plain_filename(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '\0') || c.is_control())
        && !value.ends_with([' ', '.'])
}

fn regular(path: &Path) -> AppResult<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || reparse(&metadata) {
        return Err(invalid("更新路径不是普通文件"));
    }
    Ok(())
}

fn reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn token_directory(parent: &Path, token: &str) -> AppResult<PathBuf> {
    if uuid::Uuid::parse_str(token)
        .map(|id| id.simple().to_string())
        .ok()
        .as_deref()
        != Some(token)
    {
        return Err(invalid("更新会话标识无效"));
    }
    let directory = parent.join(format!("{PREFIX}{token}"));
    let metadata = fs::symlink_metadata(&directory)?;
    if !metadata.is_dir()
        || reparse(&metadata)
        || directory.canonicalize()?.parent() != Some(parent)
    {
        return Err(invalid("更新目录无效"));
    }
    Ok(directory)
}

fn create_file(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn command(exe: &Path) -> Command {
    let mut command = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
}

impl Prepared {
    pub fn create(manifest: &[u8], signature: &str) -> AppResult<Self> {
        verify_manifest(manifest, signature)?;
        let target = std::env::current_exe()?.canonicalize()?;
        regular(&target)?;
        let parent = target.parent().ok_or_else(|| invalid("程序路径无效"))?;
        let name = target
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| invalid("程序文件名无效"))?;
        if !plain_filename(name) || !name.to_ascii_lowercase().ends_with(".exe") {
            return Err(invalid("程序文件名无效"));
        }
        // 不覆盖上次恢复备份；它可能是用户唯一可用的旧程序。
        if target.with_extension("exe.old").exists() {
            return Err(invalid(
                "程序目录有待恢复的旧版本，请先保留并移走 .old 文件后重试",
            ));
        }
        let token = uuid::Uuid::new_v4().simple().to_string();
        let directory = parent.join(format!("{PREFIX}{token}"));
        fs::create_dir(&directory)
            .map_err(|_| invalid("程序目录不可写，请将 EXE 放到可写文件夹后更新"))?;
        let prepared = Self { directory };
        let result = (|| {
            let plan = Plan {
                token,
                target: name.to_owned(),
                parent_pid: std::process::id(),
                original_sha256: hash_file(&target)?,
                manifest: STANDARD.encode(manifest),
                signature: signature.to_owned(),
            };
            create_file(
                &prepared.directory.join("plan.json"),
                &serde_json::to_vec(&plan)?,
            )?;
            let helper = prepared.directory.join("helper.exe");
            fs::copy(&target, &helper)?;
            if hash_file(&helper)? != plan.original_sha256 {
                return Err(invalid("更新辅助程序校验失败"));
            }
            Ok(())
        })();
        if let Err(error) = result {
            prepared.cleanup();
            return Err(error);
        }
        Ok(prepared)
    }

    pub async fn launch(&self) -> AppResult<()> {
        let mut child = command(&self.directory.join("helper.exe"))
            .arg(HELPER_FLAG)
            .arg(self.directory.join("plan.json"))
            .spawn()
            .map_err(|_| invalid("无法启动更新辅助程序"))?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(8) {
            if self.directory.join("helper-ready").is_file() {
                return Ok(());
            }
            if child.try_wait()?.is_some() {
                return Err(invalid("更新准备失败，当前版本仍可使用"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(invalid("更新准备超时，当前版本仍可使用"))
    }

    pub fn cleanup(&self) {
        let Some(parent) = self.directory.parent() else {
            return;
        };
        let Some(token) = self
            .directory
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix(PREFIX))
        else {
            return;
        };
        if token_directory(parent, token).is_err() {
            return;
        }
        // 失败路径也只删本次创建的已知临时文件；未知文件和备份绝不递归删除。
        for name in FILES {
            let path = self.directory.join(name);
            if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && !reparse(&m)) {
                let _ = fs::remove_file(path);
            }
        }
        let _ = fs::remove_dir(&self.directory);
    }
}

fn read_plan(directory: &Path, parent: &Path) -> AppResult<(Plan, Manifest, PathBuf)> {
    read_plan_with(directory, parent, verify_manifest)
}

fn read_plan_with(
    directory: &Path,
    parent: &Path,
    verify: impl Fn(&[u8], &str) -> AppResult<Manifest>,
) -> AppResult<(Plan, Manifest, PathBuf)> {
    let path = directory.join("plan.json");
    regular(&path)?;
    if fs::metadata(&path)?.len() > 128 * 1024 {
        return Err(invalid("更新计划大小异常"));
    }
    let plan: Plan = serde_json::from_slice(&fs::read(path)?)?;
    if token_directory(parent, &plan.token)? != directory
        || !plain_filename(&plan.target)
        || !plan.target.to_ascii_lowercase().ends_with(".exe")
        || plan.parent_pid == 0
    {
        return Err(invalid("更新目标路径无效"));
    }
    let bytes = STANDARD
        .decode(&plan.manifest)
        .map_err(|_| invalid("更新清单格式无效"))?;
    let manifest = verify(&bytes, &plan.signature)?;
    let target = parent.join(&plan.target);
    Ok((plan, manifest, target))
}

#[cfg(windows)]
struct ParentProcess(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl ParentProcess {
    fn open(pid: u32, expected: &Path) -> AppResult<Self> {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            QueryFullProcessImageNameW,
        };
        // SAFETY: 只读取指定父进程身份和退出信号，句柄由 Drop 关闭。
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            return Err(invalid("原程序已退出，更新已取消"));
        }
        let process = Self(handle);
        let mut image = vec![0_u16; 32768];
        let mut length = image.len() as u32;
        // SAFETY: image 为可写 UTF-16 缓冲区，length 为其容量。
        if unsafe { QueryFullProcessImageNameW(handle, 0, image.as_mut_ptr(), &mut length) } == 0 {
            return Err(invalid("无法确认原程序身份"));
        }
        let actual = PathBuf::from(std::ffi::OsString::from_wide(&image[..length as usize]))
            .canonicalize()?;
        if actual != expected {
            return Err(invalid("更新父进程与程序路径不一致"));
        }
        Ok(process)
    }

    fn wait(&self) -> AppResult<()> {
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        // SAFETY: self 始终持有有效进程句柄，最多等待 30 秒。
        if unsafe { WaitForSingleObject(self.0, 30_000) } != 0 {
            return Err(invalid("原程序未能退出，更新已取消"));
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for ParentProcess {
    fn drop(&mut self) {
        // SAFETY: 句柄只由本对象持有，且只关闭一次。
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

fn wait_ready(child: &mut Child, marker: &Path, token: &str) -> AppResult<()> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        if regular(marker).is_ok() && fs::read_to_string(marker).is_ok_and(|value| value == token) {
            return Ok(());
        }
        if child.try_wait()?.is_some() {
            return Err(invalid("新版本未能启动，正在恢复旧版本"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(invalid("新版本启动超时，正在恢复旧版本"))
}

#[cfg(windows)]
fn run_helper(plan_argument: &Path) -> AppResult<()> {
    run_helper_with(plan_argument, verify_manifest, command)
}

#[cfg(windows)]
fn run_helper_with(
    plan_argument: &Path,
    verify: impl Fn(&[u8], &str) -> AppResult<Manifest>,
    restart: impl Fn(&Path) -> Command,
) -> AppResult<()> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let directory = executable
        .parent()
        .ok_or_else(|| invalid("更新辅助路径无效"))?;
    let parent = directory
        .parent()
        .ok_or_else(|| invalid("更新辅助路径无效"))?;
    if executable.file_name().and_then(|n| n.to_str()) != Some("helper.exe")
        || plan_argument.canonicalize()? != directory.join("plan.json")
    {
        return Err(invalid("更新辅助参数无效"));
    }
    let (plan, manifest, target) = read_plan_with(directory, parent, verify)?;
    regular(&target)?;
    regular(&directory.join("download.exe"))?;
    if hash_file(&target)? != plan.original_sha256
        || hash_file(&executable)? != plan.original_sha256
    {
        return Err(invalid("原程序已变化，更新已取消"));
    }
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|_| invalid("当前版本格式无效"))?;
    let next =
        semver::Version::parse(&manifest.version).map_err(|_| invalid("更新版本格式无效"))?;
    if next <= current {
        return Err(invalid("更新版本必须高于当前版本"));
    }
    verify_download(&directory.join("download.exe"), &manifest)?;
    let original = ParentProcess::open(plan.parent_pid, &target)?;
    let backup = target.with_extension("exe.old");
    if backup.exists() {
        return Err(invalid("旧版本备份已存在，更新已取消"));
    }
    create_file(
        &directory.join("helper-ready"),
        std::process::id().to_string().as_bytes(),
    )?;
    original.wait()?;
    let applied = (|| -> AppResult<()> {
        // 关闭窗口后再核对一次，避免下载期间或退出时目标被更换。
        regular(&target)?;
        verify_download(&directory.join("download.exe"), &manifest)?;
        if hash_file(&target)? != plan.original_sha256 {
            return Err(invalid("原程序已变化，更新已取消"));
        }
        fs::rename(&target, &backup)?;
        if let Err(error) = fs::rename(directory.join("download.exe"), &target) {
            fs::rename(&backup, &target)?;
            return Err(error.into());
        }
        let replacement = command(&target).arg(READY_FLAG).arg(&plan.token).spawn();
        let success = match replacement {
            Ok(mut child) => {
                match wait_ready(&mut child, &directory.join("app-ready"), &plan.token) {
                    Ok(()) => true,
                    Err(_) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        false
                    }
                }
            }
            Err(_) => false,
        };
        if success {
            fs::remove_file(&backup)?;
            create_file(&directory.join("result.txt"), b"updated")?;
            // 新版负责在辅助进程退出后清理 helper.exe。
            Ok(())
        } else {
            fs::remove_file(&target)?;
            fs::rename(&backup, &target)?;
            create_file(&directory.join("result.txt"), b"rolled-back")?;
            restart(&target).spawn()?;
            Err(invalid("新版本未能启动，已恢复旧版本"))
        }
    })();
    if applied.is_err()
        && !directory.join("result.txt").exists()
        && !backup.exists()
        && regular(&target).is_ok()
        && hash_file(&target).is_ok_and(|hash| hash == plan.original_sha256)
    {
        // 退出后的文件占用/权限失败也应回到仍然完好的旧程序。
        let _ = create_file(&directory.join("result.txt"), b"not-updated");
        let _ = restart(&target).spawn();
    }
    applied
}

pub(super) fn handle_cli() -> bool {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().and_then(|s| s.to_str()) != Some(HELPER_FLAG) {
        return false;
    }
    if args.len() == 2 {
        #[cfg(windows)]
        if let Err(error) = run_helper(Path::new(&args[1])) {
            // 不把路径或任意异常写到用户数据目录。
            eprintln!("{error}");
        }
    }
    true
}

/// 回滚重启不会携带 ready 标识；只清理有签名计划、确定结束且目标已恢复的会话。
pub(super) fn cleanup_completed() -> Option<&'static str> {
    let Ok(target) = std::env::current_exe().and_then(|p| p.canonicalize()) else {
        return None;
    };
    let Some(parent) = target.parent() else {
        return None;
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return None;
    };
    let mut warning = None;
    for entry in entries.flatten().take(256) {
        let directory = entry.path();
        let Some(token) = directory
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix(PREFIX))
        else {
            continue;
        };
        if token_directory(parent, token).is_err() {
            continue;
        }
        let Ok((plan, manifest, expected)) = read_plan(&directory, parent) else {
            continue;
        };
        let result = directory.join("result.txt");
        if target != expected
            || regular(&result).is_err()
            || target.with_extension("exe.old").exists()
        {
            continue;
        }
        let outcome = fs::read_to_string(&result);
        let complete = match outcome.as_deref() {
            Ok("rolled-back" | "not-updated") => {
                hash_file(&target).is_ok_and(|hash| hash == plan.original_sha256)
            }
            Ok("updated") => verify_download(&target, &manifest).is_ok(),
            _ => false,
        };
        if complete {
            warning = match outcome.as_deref() {
                Ok("rolled-back") => Some("新版本未能启动，已恢复旧版本，数据仍然保留"),
                Ok("not-updated") => Some("本次更新未能替换程序，已重新打开原版本"),
                _ => warning,
            };
            // 运行中的 helper.exe 仍由 Windows 锁定，失败时留待下次启动，不会终止进程。
            let prepared = Prepared { directory };
            for _ in 0..20 {
                prepared.cleanup();
                if !prepared.directory.exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    warning
}

pub(super) fn mark_ready() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != READY_FLAG {
        return;
    }
    let result = (|| -> AppResult<(Prepared, u32)> {
        let target = std::env::current_exe()?.canonicalize()?;
        let parent = target.parent().ok_or_else(|| invalid("程序路径无效"))?;
        let directory = token_directory(parent, &args[1])?;
        let (plan, manifest, expected) = read_plan(&directory, parent)?;
        if target != expected || manifest.version != env!("CARGO_PKG_VERSION") {
            return Err(invalid("更新确认目标不一致"));
        }
        verify_download(&target, &manifest)?;
        let marker = directory.join("helper-ready");
        regular(&marker)?;
        let pid = fs::read_to_string(marker)?
            .parse::<u32>()
            .map_err(|_| invalid("更新辅助进程无效"))?;
        create_file(&directory.join("app-ready"), plan.token.as_bytes())?;
        Ok((Prepared { directory }, pid))
    })();
    if let Ok((prepared, _pid)) = result {
        std::thread::spawn(move || {
            let mut finished = false;
            for _ in 0..100 {
                finished |= prepared.directory.join("result.txt").is_file();
                if finished {
                    prepared.cleanup();
                    if !prepared.directory.exists() {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_escape_tokens_and_file_names() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(token_directory(tmp.path(), "../outside").is_err());
        for name in [
            "../x.exe",
            "C:\\x.exe",
            "x.exe:stream",
            "x.exe.",
            "x.exe ",
            "",
        ] {
            assert!(!plain_filename(name));
        }
        assert!(plain_filename("知了 portable.exe"));
    }

    #[test]
    fn failed_startup_never_counts_as_ready() {
        let directory = tempfile::tempdir().unwrap();
        let mut child = if cfg!(windows) {
            command(Path::new("cmd.exe"))
                .args(["/C", "exit", "1"])
                .spawn()
                .unwrap()
        } else {
            command(Path::new("sh"))
                .args(["-c", "exit 1"])
                .spawn()
                .unwrap()
        };
        assert!(wait_ready(&mut child, &directory.path().join("app-ready"), "token").is_err());
    }

    // 子进程复用测试可执行文件，只运行此 fixture，不打开 Tauri 或用户数据目录。
    #[test]
    #[cfg(windows)]
    fn helper_process_fixture() {
        let Ok(mode) = std::env::var("CICADA_UPDATER_FIXTURE") else {
            return;
        };
        let directory = PathBuf::from(std::env::var_os("CICADA_UPDATER_DIRECTORY").unwrap());
        match mode.as_str() {
            "parent" => {
                let start = Instant::now();
                while !directory.join("helper-ready").is_file() {
                    assert!(start.elapsed() < Duration::from_secs(15));
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            "helper" => {
                let key = STANDARD
                    .decode(std::env::var("CICADA_UPDATER_TEST_KEY").unwrap())
                    .unwrap();
                let result = run_helper_with(
                    &directory.join("plan.json"),
                    |bytes, signature| {
                        super::super::verify_manifest_with_key(bytes, signature, &key)
                    },
                    |path| {
                        let mut child = command(path);
                        child
                            .args([
                                "--exact",
                                "updater::helper::tests::helper_process_fixture",
                                "--nocapture",
                            ])
                            .env("CICADA_UPDATER_FIXTURE", "rollback");
                        child
                    },
                );
                let failed = std::env::var("CICADA_UPDATER_TEST_FAIL").is_ok();
                assert_eq!(result.is_err(), failed, "{result:?}");
            }
            "rollback" => {
                create_file(&directory.join("rollback-started"), b"ok").unwrap();
            }
            _ => panic!("unknown fixture"),
        }
    }

    #[test]
    #[cfg(windows)]
    fn signed_helper_replaces_and_rolls_back() {
        use ring::{
            rand::SystemRandom,
            signature::{Ed25519KeyPair, KeyPair},
        };
        let root = tempfile::tempdir().unwrap();
        let fixture_source = root.path().join("next.rs");
        let fixture_exe = root.path().join("next.exe");
        fs::write(&fixture_source, r#"
            fn main() {
                if std::env::var("CICADA_UPDATER_TEST_FAIL").is_ok() { std::process::exit(1); }
                let token = std::env::args().nth(2).unwrap();
                let exe = std::env::current_exe().unwrap();
                let marker = exe.parent().unwrap().join(format!(".cicada-update-{token}")).join("app-ready");
                std::fs::write(marker, &token).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        "#).unwrap();
        assert!(
            command(Path::new("rustc.exe"))
                .arg(&fixture_source)
                .arg("-o")
                .arg(&fixture_exe)
                .status()
                .unwrap()
                .success()
        );
        for fail in [false, true] {
            let target_dir = root.path().join(if fail {
                "rollback-case"
            } else {
                "success-case"
            });
            fs::create_dir(&target_dir).unwrap();
            let target_dir = target_dir.canonicalize().unwrap();
            let token = uuid::Uuid::new_v4().simple().to_string();
            let directory = target_dir.join(format!("{PREFIX}{token}"));
            fs::create_dir(&directory).unwrap();
            let target = target_dir.join("cicada.exe");
            fs::copy(std::env::current_exe().unwrap(), &target).unwrap();
            fs::copy(&target, directory.join("helper.exe")).unwrap();
            fs::copy(&fixture_exe, directory.join("download.exe")).unwrap();
            let key = Ed25519KeyPair::from_pkcs8(
                Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
                    .unwrap()
                    .as_ref(),
            )
            .unwrap();
            let manifest = Manifest {
                schema_version: 1,
                version: "999.0.0".into(),
                tag: "v999.0.0".into(),
                asset: "Cicada-999.0.0-windows-x64.exe".into(),
                size: fs::metadata(&fixture_exe).unwrap().len(),
                sha256: hash_file(&fixture_exe).unwrap(),
                published_at: "2026-10-20T00:00:00Z".into(),
                notes: "fixture".into(),
            };
            let bytes = serde_json::to_vec(&manifest).unwrap();
            let mut parent = command(&target)
                .args([
                    "--exact",
                    "updater::helper::tests::helper_process_fixture",
                    "--nocapture",
                ])
                .env("CICADA_UPDATER_FIXTURE", "parent")
                .env("CICADA_UPDATER_DIRECTORY", &directory)
                .spawn()
                .unwrap();
            let original_sha256 = hash_file(&target).unwrap();
            let plan = Plan {
                token,
                target: "cicada.exe".into(),
                parent_pid: parent.id(),
                original_sha256: original_sha256.clone(),
                manifest: STANDARD.encode(&bytes),
                signature: STANDARD.encode(key.sign(&bytes).as_ref()),
            };
            create_file(
                &directory.join("plan.json"),
                &serde_json::to_vec(&plan).unwrap(),
            )
            .unwrap();
            let mut helper = command(&directory.join("helper.exe"));
            helper
                .args([
                    "--exact",
                    "updater::helper::tests::helper_process_fixture",
                    "--nocapture",
                ])
                .env("CICADA_UPDATER_FIXTURE", "helper")
                .env("CICADA_UPDATER_DIRECTORY", &directory)
                .env(
                    "CICADA_UPDATER_TEST_KEY",
                    STANDARD.encode(key.public_key().as_ref()),
                );
            if fail {
                helper.env("CICADA_UPDATER_TEST_FAIL", "1");
            }
            assert!(helper.status().unwrap().success());
            assert!(parent.wait().unwrap().success());
            assert!(!target.with_extension("exe.old").exists());
            if fail {
                assert_eq!(hash_file(&target).unwrap(), original_sha256);
                assert_eq!(
                    fs::read_to_string(directory.join("result.txt")).unwrap(),
                    "rolled-back"
                );
                let start = Instant::now();
                while !directory.join("rollback-started").is_file() {
                    assert!(start.elapsed() < Duration::from_secs(10));
                    std::thread::sleep(Duration::from_millis(50));
                }
                // 仅测试 fixture 文件，生产清理白名单不会删除未知内容。
                fs::remove_file(directory.join("rollback-started")).unwrap();
            } else {
                assert_eq!(hash_file(&target).unwrap(), manifest.sha256);
                assert_eq!(
                    fs::read_to_string(directory.join("result.txt")).unwrap(),
                    "updated"
                );
            }
            std::thread::sleep(Duration::from_millis(600));
            Prepared { directory }.cleanup();
        }
    }
}
