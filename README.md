# Codex Auth Switcher

Codex 的订阅与 API key 模式切换工具。本仓库只维护 Rust + GPUI Kit 图形界面版本，提供 Windows x64、Linux x64、macOS ARM64 和 macOS x64 安装包。

工具恢复 `config.toml` 的模式参数和 `auth.json` 的认证。工具不依赖、不修改 `autocode.config.toml`。MCP、项目、插件、代理和其他共享设置保留当前值。

## 下载与启动

从 [GitHub Releases](https://github.com/bruceblink/codex-auth-switcher/releases) 下载对应平台的安装包，并用同一版本的 `SHA256SUMS.txt` 核对文件。当前开发版本使用以下格式；已发布的 `v0.3.0` 继续保留原始可执行文件。

| 平台 | 安装文件 | 启动方式 |
| --- | --- | --- |
| Windows x64 | `Codex-Auth-Switcher-<version>-windows-x64-setup.exe` | 按向导安装，从开始菜单启动 |
| Linux x64，Debian/Ubuntu | `Codex-Auth-Switcher-<version>-linux-amd64.deb` | 用系统包管理器安装，从应用菜单启动 |
| Linux x64，AppImage | `Codex-Auth-Switcher-<version>-linux-x86_64.AppImage` | 赋予执行权限后运行 |
| macOS Apple Silicon | `Codex-Auth-Switcher-<version>-macos-arm64.dmg` | 挂载后把应用拖入 Applications |
| macOS Intel | `Codex-Auth-Switcher-<version>-macos-x86_64.dmg` | 挂载后把应用拖入 Applications |

安装包包含 GPLv3 文本、第三方许可通知和源码说明。使用步骤见[用户指南](docs/06-v2-user-guide.md)。安装和卸载不删除 Codex 配置、认证或模式存档。

Windows 主流程已完成真实窗口验证。Linux 和 macOS 的真实桌面交互尚待验收，构建和包结构测试不能代替此检查。Windows 安装包未使用证书签名；macOS 使用临时签名，未进行 Apple 公证。应用内更新功能正在接入。自动重启 Codex 和用户环境变量持久化目前只支持 Windows；其他平台请手动重启，认证仍保存到所选目录的 `auth.json`。

## 本机构建

需要 Rust stable 工具链。`rust-toolchain.toml` 会为本地开发和 CI 选择最新 stable 的 Rust、Clippy 和 rustfmt 版本。

- Windows：安装 x64 MSVC、Windows SDK 和 CMake。打包脚本会检查构建环境并准备固定版本的 Inno Setup。
- Linux：使用 Ubuntu 24.04 的编译工具、Clang、CMake、Ninja、pkg-config，以及 GTK 3、WebKitGTK 4.1、OpenSSL、ALSA、X11 和 Wayland 开发库。打包还需要 `dpkg-dev`、`patchelf`、`squashfs-tools` 和 `desktop-file-utils`。完整安装命令位于 CI 工作流。
- macOS：安装 Xcode 命令行工具和当前架构的 Rust stable target。打包使用系统的 `iconutil`、`codesign` 和 `hdiutil`。

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
- “Codex 模型目录”可以读取已配置的本地目录，也可以选择已有 JSON 文件。选择文件后，工具只更新当前活动模式的 `model_catalog_json`，保留另一模式的路径。相对路径以 `config.toml` 所在目录为基准。新增条目会复制所选模型的其他能力字段。保存前会检查字段和重复 ID，并在目录文件旁创建不覆盖的备份；检测到文件或配置路径变化时会拒绝保存。工具不会把缓存当成可编辑配置；Codex 的配置文件或启动参数可能覆盖主配置。
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

Linux/macOS 本地打包可运行 `./scripts/package-unix.sh`，或对应的 `./scripts/package-linux.sh`、`./scripts/package-macos.sh`。Windows 安装与卸载测试使用 `.\scripts\package-windows.ps1 -SmokeTest`，测试会拒绝覆盖现有安装。

CI 参考 [Ramag Desktop CI](https://github.com/bruceblink/ramag-platform/blob/59399356bc88f215c8eacca632dcb3e13953632a/.github/workflows/desktop-ci.yml) 和同一提交的发布流程，按本项目的包名、图标、许可证和源码说明适配。质量检查与发布打包分开。Actions 固定提交，Cargo 下载缓存按系统和架构隔离。四种平台运行格式、全目标 check、严格 Clippy 和 workspace 测试；对应平台另外检查实际安装包。

正式标签必须与 Cargo 版本一致。发布任务核对五个安装包，生成总校验文件和 `update-manifest.json`，再发布 GitHub Release。安装产物不使用 ZIP 或 tar.gz；GitHub 自动生成的源码归档不属于应用安装包。Windows 原生 UI 验收只使用临时目录和合成认证数据。

窗口图标以 `crates/switcher-desktop/assets/codex-auth-switcher.svg` 为源图。使用 `resvg 0.46.0` 渲染 PNG，再用 `Pillow 12.3.0` 从 PNG 生成包含 16、24、32、48、64、128 和 256 像素图像的 ICO。这两个工具只用于资源生成，不是程序运行时依赖。在仓库根目录运行以下命令可重建资源：

```powershell
cargo +stable install resvg --version 0.46.0 --locked
python -m pip install Pillow==12.3.0
resvg --width 256 --height 256 crates/switcher-desktop/assets/codex-auth-switcher.svg crates/switcher-desktop/assets/codex-auth-switcher.png
python -c "from PIL import Image; Image.open('crates/switcher-desktop/assets/codex-auth-switcher.png').save('crates/switcher-desktop/assets/codex-auth-switcher.ico', format='ICO', sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])"
```

PNG 用于 X11 窗口；Windows ICO 通过 `build.rs` 和 `embed-resource` 以资源 ID 1 编入 Release PE。macOS 打包时由 PNG 生成 ICNS，写入 DMG 内的 `.app`。打包脚本使用同一套图标资源。

## 许可证

本项目采用 GPL-3.0-only，完整文本见 [LICENSE](LICENSE)。发布目标所含依赖的许可文本和署名见 [第三方许可通知](THIRD-PARTY-NOTICES.html)。第三方组件继续采用各自的许可条款，通知文件不会将其改许可为 GPL-3.0-only。

在仓库根目录运行以下命令可重建通知文件：

```powershell
cargo install --locked --version 0.9.3 --features cli cargo-about
.\scripts\licenses\generate.ps1
```

## 文档

- [Rust + GPUI Kit 开发计划](docs/01-development-plan.md)
- [用户指南](docs/06-v2-user-guide.md)
- [第一版验证历史](docs/04-v1-validation.md)

第一版验证记录仅保留历史证据。当前项目只维护 Rust + GPUI Kit 桌面程序，不包含 Python 第一版或命令行界面。当前认证存储使用 Codex 文件认证；不支持系统钥匙串、跨账户迁移或无人值守登录。
