# 知了（Cicada）

知了是面向 Windows 11 x64 的个人任务与项目记录工具。你可以用自然语言创建、查询和调整任务、重复任务与三层项目结构，也可以通过桌面便利贴和全局快捷键随时记录。

当前版本：`0.1.0`

许可证：`AGPL-3.0-only`

## 功能

- 通过模型工具调用管理任务、项目、提醒和重复任务；
- 危险操作确认、连续撤销、离线收件箱和失败重试；
- 今日任务、完成热力图以及周报、月报、年报；
- 桌面便利贴、快速记录、托盘和可自定义全局快捷键；
- 主题、提示词、时间、启动、报告、导入导出和备份恢复设置；
- 数据保存在本机，API Key 保存在 Windows 凭据管理器。

产品范围和交互规则以[产品设计](docs/产品设计.md)与[视觉规范](docs/视觉规范.md)为准。

## 运行

从 [GitHub Releases](https://github.com/HenonBamboo/Cicada/releases) 下载 `cicada.exe` 或便携 ZIP，退出旧版本后直接运行或覆盖原 EXE，无需安装。

运行要求：

- Windows 11 x64；
- Microsoft Edge WebView2 Runtime；
- 可访问的 OpenAI 兼容接口或受支持的模型服务。

程序当前未进行代码签名，Windows 首次运行时可能显示来源提示。请从项目 Releases 下载，并核对同一版本的 `SHA256SUMS.txt`。

首次启动后，在“设置 → 模型”中填写服务商、Base URL、模型名和 API Key。模型地址支持 HTTP 和 HTTPS；HTTP 会明文传输请求内容和 API Key，只应连接你信任的服务。

| 操作                | 默认快捷键              |
| ------------------- | ----------------------- |
| 快速记录            | `Ctrl+Alt+Space`        |
| 显示 / 隐藏主窗口   | `Ctrl+Alt+Z`            |
| 显示 / 隐藏便利贴   | `Ctrl+Alt+S`            |
| 切换任务 / 项目模式 | `Tab`                   |
| 发送 / 换行         | `Enter` / `Shift+Enter` |

## 数据与隐私

正式版数据位于 `%APPDATA%\Cicada`，开发版数据位于 `%APPDATA%\Cicada-dev`。任务、项目和完成记录保存在 `cicada.db`，设置保存在 `config.json`，报告、提示词、日志和备份位于对应子目录。

API Key 按“服务商 + 接口来源”保存在当前 Windows 用户的凭据管理器，不写入配置、数据库、日志或备份。多个正式版 EXE 共享同一数据目录；替换 EXE 不会删除业务数据。

完整说明见[隐私说明](PRIVACY.md)。

## 从源码开发

需要 Node.js、Rust MSVC 工具链、Visual Studio C++ 构建工具、Windows SDK 和 WebView2 Runtime。

```powershell
git clone git@github.com:HenonBamboo/Cicada.git
cd Cicada
npm ci
npm run tauri dev
```

常用检查：

```powershell
npm run typecheck
npm run lint
cargo test --manifest-path src-tauri/Cargo.toml --offline
```

生成直接运行 EXE 和便携 ZIP：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1
```

产物写入 `releases/`。维护者发布前应遵循[发布流程](docs/发布流程.md)，确保二进制与对应 Git 标签的源码一致。

## 参与项目

提交代码前请阅读[贡献指南](CONTRIBUTING.md)。安全问题请按[安全政策](SECURITY.md)私下报告，不要在公开 Issue 中包含 API Key、数据库或备份。

## 许可证

除单独标明的第三方组件外，本仓库的源代码、文档和项目自有图形资源依据 [GNU Affero General Public License v3.0 only](LICENSE) 发布。第三方组件仍适用各自许可证，详见 [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt)。

Copyright © 2026 HenonBamboo.
