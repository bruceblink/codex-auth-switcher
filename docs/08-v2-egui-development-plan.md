# Rust + egui 第二版开发计划

日期：2026-10-25。

## 范围

当前分支只维护 Rust 图形界面。桌面界面使用 egui/eframe 0.37.0；业务核心由独立的 `switcher-core` crate 提供。CLI 模块、CLI 专用参数和测试入口已删除。Release 图形程序使用 Windows GUI 子系统，不会打开终端窗口。gpui-kit 的运行依赖、GPUI UI 源码和 GPUI 专用测试已移除。GPUI 的旧验收记录保留在 [历史计划](05-v2-gpui-kit-development-plan.md)。Python 第一版的源码、测试、入口、配置和 CI 已删除。

egui 应用继续提供模式初始化、补齐、来源导入、目录选择、刷新、目标选择、API 参数输入、脱敏预览和应用切换。文件与加密操作在工作线程运行。桌面渲染使用 eframe 的 Wgpu 后端；Windows x64 是当前发布目标。eframe 0.37.0 将 MSRV 从 1.91 提高到 1.95。

## 接受条件

- 订阅和 API 模式可分别保存并恢复 provider、模型、推理参数和认证。
- 切换过程保留 MCP、项目、插件等共享配置。工具不读取或修改 `autocode.config.toml`。
- 导入的 TOML 与 JSON 只更新加密模式存档。导入不会直接替换活动配置或认证。
- 预览不显示 key、令牌或共享配置正文。应用前检查预览涉及的文件是否已变化。
- 配置备份只保存当前模式参数和选中 provider，不复制共享设置或凭据；活动配置继续保留共享设置。
- API key 输入使用密码遮罩。成功或失败结束后清除输入缓冲区。
- 后台操作失败时恢复文件状态；外部文件变化时停止覆盖。
- Headless 测试实际驱动 egui 按钮，并检查合成目录内的文件结果。
- Windows Release 原生功能验收使用真实窗口和隔离的临时 `CODEX_HOME`。DPI、输入法、文件对话框和窗口尺寸证据与 Headless 测试分开记录。
- UI 视觉验收必须确认真实窗口截图中能看到控件和布局。可访问性树或控件自动化通过不能替代视觉验收。
- CI 在 Windows、Linux、macOS 运行格式检查、Clippy 和 workspace 测试。Linux runner 安装 eframe 的 X11、Wayland、字体和图形开发依赖。只有 Windows runner 构建便携包并执行原生窗口检查。
- 发布包不依赖宿主 Python、Rust 或包管理器环境。自动化测试不读取真实 Codex 认证。

## 验证命令

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
.\scripts\package-windows.ps1
.\scripts\test-v2-native-ui.ps1
```

当前 Rust 测试包含 23 项共享核心测试和 6 项 egui UI 测试。UI 测试覆盖初始化、TOML/JSON 导入、预览、API 与订阅往返、取消文件与目录选择器、取消后刷新，以及模型目录的读取、复制、保存和备份。

桌面代码只在一个 `switcher-desktop` crate 中维护。`app.rs` 处理界面状态和后台任务，`app/ui.rs` 绘制控件，`app/format.rs` 显示模式摘要和安全错误，`app/tests.rs` 验证按钮流程。独立的 `switcher-core` crate 不依赖桌面框架。

真实 Windows Release 窗口已在隔离的临时 `CODEX_HOME` 中通过原生文件选择器导入合成 TOML 和认证 JSON，并完成 API 切换、订阅恢复、共享设置检查，以及 `models-codex.json` 读取、复制、编辑、保存和备份。磁盘检查确认当前认证、模式参数和状态提示一致；模型目录写入保留了未编辑的顶层标记和模型能力字段；配置备份不含共享 shell 令牌或 API key。测试未修改真实 Codex 配置，也未验证远程认证服务。

在本机当前 100% 显示缩放下，Glow 后端的 Release 窗口出现白色客户区；同一代码切换到 Wgpu 后，主窗口和模型目录编辑器都在真实窗口中完整显示。Computer Use 直接启动最终 Release 后，看到了真实配置的当前状态和模型目录空状态；这次只读检查未读取模型文件，也没有修改真实配置。Wgpu Release 还使用隔离的临时 `CODEX_HOME` 通过模式往返和模型目录读、复制、编辑、保存、备份；测试未调整分辨率或缩放。Headless 截图缺少中文系统字体适配，也不能用于判断最终字形。

本次布局打磨让模式卡片使用固定字段列和可换行的值，分开显示当前模式、就绪状态、provider、模型、推理级别、配置来源和认证来源。模型目录窗口增加了状态提示、分组表单、滚动区域、较清楚的字段标签和长路径悬停显示。窗口在本机当前 100% 显示缩放下测试；未调整屏幕分辨率或缩放设置。

## 数据边界和回滚

测试只操作仓库 `build/` 下的新合成目录或系统临时目录。不会把真实凭据写入测试、日志、源码或 Git。第一版 DPAPI 存档不迁移、不覆盖、不删除。当前存档格式和 `switcher-core` 的持久化逻辑不因更换 GUI 框架而改变。

egui 是当前唯一维护的桌面实现。未来若要更换框架，应作为一次明确迁移处理，不在当前 workspace 中并行保留第二套实现。回滚不修改 `switcher-core` 或用户数据。此计划记录当前实现范围和验收边界。
