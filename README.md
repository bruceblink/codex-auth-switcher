# Codex Auth Switcher

Windows 桌面 Codex 的订阅与 API key 模式切换工具。本仓库只维护 Rust 图形界面版本。

工具恢复 `config.toml` 的模式参数和 `auth.json` 的认证。工具不依赖、不修改 `autocode.config.toml`。MCP、项目、插件、代理和其他共享设置保留当前值。

## 下载与启动

从 GitHub Actions 的 `Cross-platform checks` 工作流下载 `codex-auth-switcher-windows-x64` 便携包。解压后运行 `codex-auth-switcher.exe`。Release 图形界面启动时不会打开终端窗口。使用步骤见 [V2 用户指南](docs/06-v2-user-guide.md)。

## 本机构建

需要 Rust stable 工具链和 Windows MSVC 构建工具。`rust-toolchain.toml` 会为本地开发和 CI 选择最新 stable 的 Rust、Clippy 和 rustfmt 版本。

```powershell
cargo run --locked
cargo build --release --locked
```

## 项目结构

仓库只保留一个 Cargo workspace。`switcher-core` crate 管理配置、加密存档和恢复；`switcher-desktop` crate 提供 egui 桌面程序。界面协调代码位于 `app.rs`，控件布局位于 `app/ui.rs`，状态与错误文字位于 `app/format.rs`。UI 测试位于 `app/tests.rs`。

## 配置与安全边界

- 订阅默认 provider：`openai`。
- API 默认 provider：`OpenAI`，默认地址：`https://vip.auto-code.net/v1`。
- `openai` 与 `OpenAI` 是不同的 ID。初始化会优先导入现有配置，不用默认值覆盖已有网关。
- API 模式保留现有 provider 地址和能力字段，并通过 `requires_openai_auth = true` 从 `auth.json` 读取 key。
- 模式快照会保留 Codex 的模型详细度、自动压缩设置、模型指令文件、审查模型以及 ChatGPT/OSS provider 字段；旧存档缺少这些字段时沿用活动配置。
- API key 只从输入框或已保存认证读取。程序不会读取 `AUTOCODE_API_KEY` 或 provider 的 `env_key` 环境变量。输入框里的密钥成功应用后，程序会写入 Windows 当前用户的持久环境变量 `AUTOCODE_API_KEY`；切换失败时不会写入。预览会显示认证来源，不显示密钥。应用后输入内容会清除。
- “Codex 模型目录”可以维护 `models-codex.json` 的模型 ID、显示名称、可见性、优先级、默认推理级别和支持的推理级别。新增条目会复制所选模型的其他能力字段。保存前会检查字段和重复 ID，并创建不覆盖的 `models-codex.json.bak-*` 备份；检测到文件被其他程序修改时会拒绝保存。
- 本地模式存档位于 `.codex/auth-switcher-v2/`。`modes.enc` 与 `pending.enc` 使用 AES-256-GCM；`vault.key` 与密文分开保存。
- 配置变化时生成的 `config.toml.bak-*` 只保存当前模式参数和选中 provider，不复制共享设置或凭据。活动配置中的共享设置保持不变。
- Windows 状态目录只允许当前用户和 SYSTEM 访问。密钥缺失、密文认证失败或外部文件冲突时，程序停止并保留现有数据。
- Codex 使用的 `auth.json` 继续采用 Codex 格式。程序不会向远程 API 发送请求，也不会验证认证是否有效。
- 文件检查不能锁住 Codex 自身。切换前请关闭 Codex；切换后重启 Codex 并新建会话。

旧 DPAPI 存档和第一版 Python 程序不由当前版本读取。若第一版留有未完成的 `pending.dpapi` 写入，先按历史版本流程恢复，再使用当前版本。

## 验证

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
.\scripts\package-windows.ps1
.\scripts\test-v2-native-ui.ps1
```

Linux/macOS 本地打包可运行 `./scripts/package-unix.sh`。GitHub Actions 在 Windows、Linux 和 macOS 运行格式检查、Clippy、workspace 测试，并上传三个 runner 对应的 Release 可执行文件、归档包和 SHA-256 校验文件。Windows runner 还会运行原生窗口验收；原生验收只使用临时目录和合成认证数据。

## 文档

- [认证模式恢复历史计划](docs/02-auth-mode-recovery-plan.md)
- [技术重评](docs/03-technology-review.md)
- [第一版验证历史](docs/04-v1-validation.md)
- [Rust + egui 开发计划](docs/08-v2-egui-development-plan.md)
- [V2 用户指南](docs/06-v2-user-guide.md)
- [egui 与 gpui-kit 维护成本评估](docs/07-egui-maintenance-evaluation.md)

第一版开发和验证文档只作为历史记录，不代表仓库仍维护 Python 代码。当前项目仅支持 Windows 文件认证存储，不支持系统钥匙串、跨账户迁移或无人值守登录。
