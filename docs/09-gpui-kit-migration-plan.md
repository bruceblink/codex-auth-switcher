# GPUI Kit 桌面迁移计划

日期：2026-10-31。当前实现基线：`05a317e`。该提交已推送，包含 egui 桌面界面和 Windows 重启功能。

## 范围

用户反馈 egui 界面卡顿且不便美化，要求改用 GPUI Kit。工作区保留 `switcher-core` 和 `switcher-desktop`，桌面层使用 GPUI Kit 0.8.1。完成后移除 egui/eframe 依赖和测试入口，只维护一套 GUI。

保留模式初始化、官方默认配置、认证导入、预览、API 与订阅往返、失败恢复、模型目录管理、密钥清理及 Windows 重启确认。核心存档格式和配置写入逻辑沿用当前实现。

## 实施顺序

1. 重写桌面状态和布局。文件、加密和进程操作放在后台任务；任务完成时更新界面。不使用定时轮询或逐帧读取文件。
2. 运行核心和 GPUI 交互测试，编译本地 Release。在 Windows 100% 缩放下通过 Computer Use 验证真实窗口、输入、滚动、模式切换和模型保存。完成后独立提交并推送。
3. 调整多平台 CI 与打包。Windows x64、Linux x64、macOS ARM64 和 x64 分别检查、构建、校验原始可执行文件和 SHA-256，不生成 ZIP 或 tar.gz。完成后独立提交并推送，等待各平台结果。

## 接受条件

- 配置目录、订阅/API 模式、模型目录、预览和固定操作区在默认和最小窗口下没有重叠或横向溢出。
- 输入框实体在状态更新间复用；后台任务不阻塞输入和滚动；空闲时不持续刷新窗口。
- 模式往返后，共享配置保留；过期预览、无效输入、未完成恢复和未保存模型修改均按原有规则处理。
- API key 不出现在预览或错误中；应用成功或失败后清理输入及其撤销历史。
- 空目录可创建官方默认配置；没有真实认证时不生成占位 auth.json。
- 重启按钮先确认。自动测试只关闭受控测试进程，不关闭当前会话所在的 Codex。
- GPUI Kit 是唯一桌面框架，workspace 不再依赖 egui/eframe。

## 验证

```text
rustup run stable rustc -Vv
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
```

本机窗口测试使用隔离 CODEX_HOME 与合成认证。记录本机可执行文件体积与启动、输入、滚动结果。Linux/macOS 的构建校验和真实窗口验收分别报告；编译成功不代表 GUI 视觉验收通过。

## 回滚

`05a317e` 是迁移前可恢复的代码版本。迁移只替换桌面层，不改写用户配置或存档。通过撤回迁移提交恢复 egui；不复制第二套 GUI 到当前工作区，不覆盖真实用户认证。
