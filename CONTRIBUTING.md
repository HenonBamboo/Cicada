# 贡献指南

感谢你参与知了。请先阅读 `docs/产品设计.md` 和 `docs/视觉规范.md`；功能、交互和界面修改应保持现有产品范围，不顺带增加无关能力或视觉效果。

## 开发环境

- Windows 11 x64；
- Node.js 与 npm；
- Rust MSVC 工具链；
- Visual Studio C++ 构建工具、Windows SDK 和 WebView2 Runtime。

```powershell
npm ci
npm run tauri dev
```

## 修改原则

- 前端不直接访问数据库、模型服务或 API Key；业务写入、校验和模型请求统一在 Rust 端完成；
- 用户数据保持向后兼容。数据库结构变更必须增加版本化迁移，并在迁移前创建备份；
- 不提交真实 API Key、用户数据库、配置、日志、备份、构建缓存或发布产物；
- 更新依赖后同步检查 `THIRD_PARTY_NOTICES.txt`；
- 修改用户可见行为时同步更新 README、产品设计或更新记录中的对应说明。

## 提交前检查

根据改动范围运行必要检查，不需要无理由扩大验证：

```powershell
npm run typecheck
npm run lint
cargo test --manifest-path src-tauri/Cargo.toml --offline
```

发布构建使用：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1
```

提交贡献即表示你有权提供这些内容，并同意按本项目的 `AGPL-3.0-only` 许可证发布。
