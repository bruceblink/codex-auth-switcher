# 技术路线

最后更新：2026-10-31。

当前程序使用 Rust + GPUI Kit。TOML 与 JSON 处理、AES-256-GCM 存档和事务恢复逻辑都位于 Rust workspace。程序只提供桌面图形界面。Python 第一版实现和 CLI 已从当前分支删除。

此前的框架对比见[egui 与 gpui-kit 维护成本评估](07-egui-maintenance-evaluation.md)。本机测量表明 egui 产物和依赖图更小，但用户反馈 egui 界面存在卡顿且难以美化，因此决定切回 GPUI Kit。迁移范围和验收条件见[GPUI Kit 桌面迁移计划](09-gpui-kit-migration-plan.md)。

GPUI Kit 提供原生窗口、组件和主题。应用不引入 WebView 或 Python 运行时。文件与加密工作在后台执行，任务结束后通知界面更新。

## 运行与数据边界

- 图形界面由 GPUI Kit 创建原生窗口。文件操作和加密操作在后台执行。
- GPUI Kit 版本锁定在 0.8.1，工作区使用 Rust stable 1.99.1 进行构建和验证。
- `switcher-core` 不依赖图形框架。它负责配置解析、模式状态、加密、文件锁、预览过期检查和失败恢复。
- 模式密文、密钥和 Codex 的活动配置分开保存。工具不会联网验证 key，不会读取或修改 `autocode.config.toml`。
- Windows 状态目录限制为当前用户和 SYSTEM。缺失密钥、密文校验失败或外部文件变化时，工具停止并保留已有数据。
- 第一版的 `modes.dpapi` 与 `pending.dpapi` 不由当前程序读取。需要恢复旧版未完成写入时，从 Git 历史提交 `d2bb002` 取回第一版程序，在原 Windows 用户账户下处理。

## 平台与发布

当前 Windows 发布构件为 Windows x64 MSVC 原始可执行文件；CI 同时生成 Linux x64、macOS ARM64 和 macOS x64 的原始 Release 可执行文件。每个可执行文件都带有独立的 SHA-256 校验文件。工作流不生成 ZIP 或 `tar.gz` 归档。构件不安装服务或系统启动项。Windows Release 窗口已在本机 100% 显示缩放下完成视觉和功能验收，并在隔离的临时 `CODEX_HOME` 中通过原生文件选择器导入合成配置。跨平台 CI 对 Windows、Linux 和 macOS 运行格式检查、Clippy 与 workspace 测试；Windows CI 还运行原生窗口验收。Linux 和 macOS 的 GUI 视觉验收、代码签名、安装程序、自动更新和系统钥匙串不属于当前已验证范围。

相关文档：

- [egui 历史开发计划](08-v2-egui-development-plan.md)
- [GPUI Kit 桌面迁移计划](09-gpui-kit-migration-plan.md)
- [egui 与 gpui-kit 维护成本评估](07-egui-maintenance-evaluation.md)
- [V2 用户指南](06-v2-user-guide.md)
- [GPUI Kit 历史计划](05-v2-gpui-kit-development-plan.md)
- [第一版开发计划历史](01-development-plan.md)
- [第一版验证记录历史](04-v1-validation.md)

## 官方资料

- [GPUI Kit 安装文档](https://gpui-kit.com/docs/installation/)
- [GPUI Kit 版本记录](https://github.com/longbridge/gpui-kit/releases)
- [RustCrypto AES-GCM](https://docs.rs/aes-gcm/latest/aes_gcm/)
