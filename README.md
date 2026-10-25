# Codex Auth Switcher

Windows 桌面 Codex 的订阅与 API key 模式切换工具。本仓库只维护 Rust 版本。程序包含图形界面和命令行界面，不需要安装 Python、Rust 或 Ramag。

工具同时恢复 `config.toml` 的模式参数和 `auth.json` 的认证。工具不依赖、不修改 `autocode.config.toml`。MCP、项目、插件、代理和其他共享设置保留当前值。

## 下载与启动

从 GitHub Actions 的 `Cross-platform checks` 工作流下载 `codex-auth-switcher-windows-x64` 便携包。解压后运行 `codex-auth-switcher.exe`。发布版本、SHA-256 与使用步骤见 [V2 用户指南](docs/06-v2-user-guide.md)。

## 本机构建

需要 Rust 工具链和 Windows MSVC 构建工具。

```powershell
cargo run --locked
cargo run --locked -- --command status
cargo run --locked -- --command initialize
cargo run --locked -- --command preview --target api
cargo run --locked -- --command switch --target api --api-key-stdin
```

标准输入只用于提供 key。不要把 key 写入命令参数。可使用 `--demo --codex-home <新目录> --isolated` 创建合成测试环境；演示目录必须是新目录。

## 项目结构

仓库只保留一个 Cargo workspace。`switcher-core` crate 管理配置、加密存档和恢复；`switcher-desktop` crate 提供一个 egui 桌面程序和 CLI。界面协调代码位于 `app.rs`，控件布局位于 `app/ui.rs`，状态与错误文字位于 `app/format.rs`。UI 测试位于 `app/tests.rs`。这两个 crate 属于同一个 workspace，不是两套桌面实现。

## 配置与安全边界

- 订阅默认 provider：`openai`。
- API 默认 provider：`OpenAI`，默认地址：`https://vip.auto-code.net/v1`。
- `openai` 与 `OpenAI` 是不同的 ID。初始化会优先导入现有配置，不用默认值覆盖已有网关。
- API 模式保留现有 provider 地址和能力字段，并通过 `requires_openai_auth = true` 从 `auth.json` 读取 key。
- UI/CLI 显式输入优先于环境变量，再使用已保存模式和项目默认值。
- API key 输入框遮盖密钥。应用后输入内容会清除。
- 本地模式存档位于 `.codex/auth-switcher-v2/`。`modes.enc` 与 `pending.enc` 使用 AES-256-GCM；`vault.key` 与密文分开保存。
- Windows 状态目录只允许当前用户和 SYSTEM 访问。密钥缺失、密文认证失败或外部文件冲突时，程序停止并保留现有数据。
- Codex 使用的 `auth.json` 继续采用 Codex 格式。程序不会向远程 API 发送请求，也不会验证认证是否有效。
- 文件检查不能锁住 Codex 自身。切换前请关闭 Codex；切换后重启 Codex 并新建会话。

旧 DPAPI 存档和第一版 Python 程序不由当前版本读取。若第一版留有未完成的 `pending.dpapi` 写入，先按历史版本流程恢复，再使用当前版本。

## 验证

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
.\scripts\test-v2-headless.ps1
.\scripts\package-windows.ps1
.\scripts\test-v2-native-ui.ps1
```

GitHub Actions 在 Windows、Linux 和 macOS 运行格式检查、Clippy 和 workspace 测试。Windows runner 还会构建便携包，并运行 Headless 与原生窗口检查；Linux 和 macOS 暂不执行原生窗口验收或打包。自动测试使用临时目录和虚构认证数据。Headless 检查会驱动 GUI 按钮并验证隔离文件状态。Windows 原生窗口检查会启动便携程序，只操作自动生成的演示目录。

## 文档

- [认证模式恢复历史计划](docs/02-auth-mode-recovery-plan.md)
- [技术重评](docs/03-technology-review.md)
- [第一版验证历史](docs/04-v1-validation.md)
- [Rust + egui 开发计划](docs/08-v2-egui-development-plan.md)
- [V2 用户指南](docs/06-v2-user-guide.md)
- [egui 与 gpui-kit 维护成本评估](docs/07-egui-maintenance-evaluation.md)

第一版开发和验证文档只作为历史记录，不代表仓库仍维护 Python 代码。当前项目仅支持 Windows 文件认证存储，不支持系统钥匙串、跨账户迁移或无人值守登录。
