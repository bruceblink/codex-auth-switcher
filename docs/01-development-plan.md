# Rust + GPUI Kit 开发计划

更新时间：2026-10-31。

## 目标

维护一套 Rust + GPUI Kit 桌面程序，用于管理 Codex 的订阅与 API 模式。程序保留用户的共享配置，安全保存模式档案，并在写入失败或进程中断时恢复原状态。

当前工作区只维护 `switcher-core` 和 `switcher-desktop` 两个 Rust crate。GPUI Kit 是唯一桌面框架。产品不提供 Python 运行时或命令行界面。

## 架构与平台范围

- `switcher-core` 负责 TOML 与 JSON 校验、模式存档、预览、文件写入、文件锁、备份和恢复。
- `switcher-desktop` 使用 GPUI Kit 0.8.1 提供一个原生窗口。输入实体在界面状态更新时复用。文件、加密和进程操作在后台运行。渲染过程不执行文件访问，也不使用定时全量刷新。
- Rust 工具链使用 stable。`rust-toolchain.toml`、CI 和本地构建均使用 stable、Clippy 和 rustfmt。
- GitHub Actions 检查 Windows x64、Linux x64、macOS ARM64 和 macOS x64，并构建相应的原始可执行文件与 SHA-256 文件。
- Windows 支持原生重启 Codex；Linux 和 macOS 需要用户手动重启。单实例锁按当前 OS 用户生效，不受 `CODEX_HOME` 影响。
- 程序不访问远程 API，不验证 API key 或订阅令牌。它不修改 `autocode.config.toml`，也不替换 Codex 的系统钥匙串认证。

## 当前功能

- 首次使用时创建缺失的 `config.toml`，使用 Codex 内置 `openai` provider 和官方 API 地址 `https://api.openai.com/v1`。工具不会创建占位 `auth.json`。
- 用户可以导入订阅和 API 模式配置与认证，预览变更，再应用模式切换。预览过期、输入无效、活动文件被外部修改或恢复尚未完成时，程序会停止写入。
- 模式切换保留 MCP、项目、插件、代理和其他共享设置。`auth.json` 保持 Codex 格式。模式档案存储在 `.codex/auth-switcher-v2/`。
- 模型目录页可以读取已配置的 JSON 文件，也可以选择现有文件并设置当前活动模式的 `model_catalog_json`。另一模式的路径保持不变。路径可为绝对路径或相对 `config.toml` 所在目录的路径。
- 模型编辑器支持修改 ID、显示名称、可见性、优先级、默认推理级别和支持的推理级别。保存前校验数据，并检测目录文件和配置路径是否已变化。
- Windows 重启操作需要用户确认。程序使用正常关闭请求，不强制结束 Codex 进程。

## 安全与恢复

- `modes.enc` 和 `pending.enc` 使用 AES-256-GCM。`vault.key` 与密文分开存放。Windows 状态目录只允许当前用户和 SYSTEM 访问。
- 配置更新前创建不覆盖的 `config.toml.bak-*`。该备份只包含当前模式参数和选中的 provider，不复制共享设置或凭据。模型目录保存会在 JSON 文件旁创建独立备份。
- 同步配置、认证和加密档案时先写入恢复记录。进程中断后，初始化流程按记录恢复。密钥缺失、密文损坏、外部修改或恢复冲突时，程序停止并保留数据。
- API key 只来自界面输入或已保存认证。只有成功应用后，程序才会写入 Windows 当前用户的 `AUTOCODE_API_KEY` 环境变量。界面不会显示完整密钥，预览和错误不显示密钥。
- 旧版 DPAPI 数据不由当前程序读取。若旧版存在未完成的 `pending.dpapi` 写入，应使用对应历史版本恢复。
- 自动化测试使用临时目录和合成认证。测试不得读取或修改真实 `.codex` 文件，也不得重启用户当前运行的 Codex。

## 未完成事项

1. **键盘导航：** 尚未在真实窗口中验证 Tab、Shift+Tab、Enter 和 Space 的焦点顺序、可见焦点及操作结果。
2. **跨平台真实窗口：** Linux 和 macOS 的原生窗口尚未完成视觉与交互验收。CI 构建或 headless 测试不能替代真实窗口检查。
3. **正式发布：** 当前工作流生成并上传原始可执行文件与 SHA-256 文件，不创建 GitHub Release。代码签名、安装程序和自动更新也未实现。
4. **模型文件选择的完整验收：** Core 隔离测试覆盖路径写入、活动模式往返、无效文件和失败恢复。workspace 检查和 Windows 真实窗口的文件选择已通过；还需在真实窗口中验证取消、未保存编辑和模式切换。

## 验证

使用 Rust stable，从仓库根目录运行：

```powershell
rustup run stable rustc -Vv
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
```

Windows 打包和原生窗口测试：

```powershell
.\scripts\package-windows.ps1
.\scripts\test-v2-native-ui.ps1
```

Linux 与 macOS 打包：

```sh
./scripts/package-unix.sh
```

CI 在四种目标平台运行格式检查、workspace check、Clippy 和测试。打包工作流验证二进制架构与 SHA-256。Windows 工作流另外运行隔离目录下的原生 UI 自动化。UI 验收记录必须区分 headless、Windows 真实窗口、Linux/macOS 真实窗口、发布构件和正式发布结果。

## 回滚

- 每项独立功能完成验证后单独提交。需要撤回功能时，撤回对应提交，不重置其他工作。
- GPUI Kit 迁移前的代码恢复点是 Git 提交 `05a317e`。恢复该版本时使用 Git 历史，不在当前 workspace 并行维护第二套 GUI。
- 回滚代码不会恢复用户数据。配置路径变更使用 `config.toml.bak-*`，模型文件保存使用 JSON 旁的备份。未完成的事务由 `pending.enc` 恢复；不要手动删除密钥、密文、恢复记录或 Codex 认证。

## 相关文档

- [用户指南](06-v2-user-guide.md)
- [第一版验证历史](04-v1-validation.md)

其他历史开发计划和路线评估保留在 Git 提交历史中。文档目录只维护一份现行开发计划。
