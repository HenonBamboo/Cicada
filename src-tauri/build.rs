#[allow(dead_code)]
mod command_manifest;
fn main() {
    println!("cargo:rerun-if-changed=command_manifest.rs");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(command_manifest::COMMANDS)),
    )
    .expect("生成窗口权限失败");
}
