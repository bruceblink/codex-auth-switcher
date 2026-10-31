# Codex Auth Switcher

Windows 桌面 Codex 的订阅与 API key 模式切换工具。本仓库只维护 Rust 图形界面版本。

工具恢复 `config.toml` 的模式参数和 `auth.json` 的认证。工具不依赖、不修改 `autocode.config.toml`。MCP、项目、插件、代理和其他共享设置保留当前值。

## 下载与启动

从 GitHub Actions 的 `Cross-platform checks` 工作流下载对应平台的原始可执行文件和 SHA-256 文件。Windows 直接运行 `codex-auth-switcher-v<version>-windows-x64.exe`。Release 图形界面启动时不会打开终端窗口。使用步骤见 [V2 用户指南](docs/06-v2-user-guide.md)。

## 本机构建

需要 Rust stable 工具链和 Windows MSVC 构建工具。`rust-toolchain.toml` 会为本地开发和 CI 选择最新 stable 的 Rust、Clippy 和 rustfmt 版本。

```powershell
cargo run --locked
cargo build --release --locked
```

## 项目结构

仓库只保留一个 Cargo workspace。`switcher-core` crate 管理配置、加密存档和恢复；`switcher-desktop` crate 提供 GPUI Kit 桌面程序。界面协调代码位于 `app.rs`，控件布局位于 `app/view.rs`，状态与错误文字位于 `app/format.rs`。UI 测试位于 `app/tests.rs`。

## 配置与安全边界

- 订阅默认 provider：`openai`。
- 首次使用时，工具创建缺失的 `config.toml`，使用 Codex 内置 `openai` provider 和官方 API 地址 `https://api.openai.com/v1`。模型由 Codex 默认选择。
- `openai` 与 `OpenAI` 是不同的 ID。初始化会优先导入现有配置，不用默认值覆盖已有网关。使用其他地址时，请填写自定义 provider ID。
- API 模式保留现有 provider 地址和能力字段，并通过 `requires_openai_auth = true` 从 `auth.json` 读取 key。
- 模式快照会保留 Codex 的模型详细度、自动压缩设置、模型指令文件、审查模型以及 ChatGPT/OSS provider 字段；旧存档缺少这些字段时沿用活动配置。
- API key 只从输入框或已保存认证读取。程序不会读取 `AUTOCODE_API_KEY` 或 provider 的 `env_key` 环境变量。输入框里的密钥成功应用后，程序会写入 Windows 当前用户的持久环境变量 `AUTOCODE_API_KEY`；切换失败时不会写入。预览会显示认证来源，不显示密钥。应用后输入内容会清除。
- 没有 `auth.json` 时，工具不会生成占位认证。订阅认证由用户在 Codex 中登录生成；API key 认证在用户输入有效格式的 key 并应用后生成。
- “Codex 模型目录”可以维护 `config.toml` 中 `model_catalog_json` 指定的本地目录。相对路径以 `config.toml` 所在目录为基准。新增条目会复制所选模型的其他能力字段。保存前会检查字段和重复 ID，并在目录文件旁创建不覆盖的备份；检测到文件或配置路径变化时会拒绝保存。未指定时工具不会把缓存当成可编辑配置；Codex 的配置文件或启动参数可能覆盖主配置。
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

Linux/macOS 本地打包可运行 `./scripts/package-unix.sh`。GitHub Actions 在 Windows、Linux 和 macOS 运行格式检查、Clippy、workspace 测试，并上传各平台的原始 Release 可执行文件和 SHA-256 校验文件。工作流不生成 ZIP 或 tar.gz；Windows runner 还会运行原生窗口验收，原生验收只使用临时目录和合成认证数据。

窗口图标以 `crates/switcher-desktop/assets/codex-auth-switcher.svg` 为源图。使用 `resvg 0.46.0` 渲染 PNG，再用 `Pillow 12.3.0` 从 PNG 生成包含 16、24、32、48、64、128 和 256 像素图像的 ICO。这两个工具只用于资源生成，不是程序运行时依赖。在仓库根目录运行以下命令可重建资源：

```powershell
cargo +stable install resvg --version 0.46.0 --locked
python -m pip install Pillow==12.3.0
resvg --width 256 --height 256 crates/switcher-desktop/assets/codex-auth-switcher.svg crates/switcher-desktop/assets/codex-auth-switcher.png
python -c "from PIL import Image; Image.open('crates/switcher-desktop/assets/codex-auth-switcher.png').save('crates/switcher-desktop/assets/codex-auth-switcher.ico', format='ICO', sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])"
```

PNG 用于 X11 窗口；Windows ICO 通过 `build.rs` 和 `embed-resource` 以资源 ID 1 编入 Release PE，Windows 打包脚本和 CI 的 Cargo 构建会自动包含它。macOS 当前发布原始可执行文件，不包含 `.app` 图标。

## 文档

- [认证模式恢复历史计划](docs/02-auth-mode-recovery-plan.md)
- [技术重评](docs/03-technology-review.md)
- [第一版验证历史](docs/04-v1-validation.md)
- [Rust + egui 开发计划](docs/08-v2-egui-development-plan.md)
- [GPUI Kit 桌面迁移计划](docs/09-gpui-kit-migration-plan.md)
- [V2 用户指南](docs/06-v2-user-guide.md)
- [egui 与 gpui-kit 维护成本评估](docs/07-egui-maintenance-evaluation.md)

第一版开发和验证文档只作为历史记录，不代表仓库仍维护 Python 代码。当前项目仅支持 Windows 文件认证存储，不支持系统钥匙串、跨账户迁移或无人值守登录。
