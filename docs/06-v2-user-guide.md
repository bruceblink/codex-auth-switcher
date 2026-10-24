# Codex Auth Switcher V2 用户指南

本指南适用于 Windows x64 便携包。运行程序不需要 Python、Rust 或 Ramag。

## 开始使用

1. 解压 ZIP 文件，并运行 `codex-auth-switcher.exe`。
2. 检查“配置目录”。默认值来自 `CODEX_HOME` 或 `%USERPROFILE%\.codex`。
3. 点击“初始化模式”或“补齐模式”。工具读取当前 `config.toml`、`auth.json` 和可识别的备份文件。
4. 检查订阅和 API key 模式卡片。卡片会显示 provider、模型、推理级别和配置来源。
5. 选择目标模式。需要时编辑 provider、Base URL 或 API key。
6. 点击“预览变更”，检查变更字段。预览不显示认证正文。
7. 关闭 Codex，再点击“应用切换”。切换完成后重新启动 Codex，并新建会话。

工具只在初始化、导入或用户确认“应用切换”时写入配置。文件操作在后台运行。工具不请求远程 API，因此“已就绪”不代表 key 有效、订阅未过期或服务可用。

## 导入模式来源

在目标模式区域，使用配置按钮导入 TOML，使用用户按钮导入认证 JSON。导入只更新加密模式存档，不会立即切换当前认证。

切换时，工具会保存当前活动配置和认证。显式导入的配置与认证会保留在目标模式中。即使目标模式当前正在使用，切换到另一模式再切回时仍会应用导入的数据。导入不会立即切换当前认证。

API key 输入框会遮盖输入内容，并显示首尾提示。应用切换后，输入框会清空。API key 不能放在命令行参数中。

## 本地数据和备份

第二版将模式存档保存在 `%USERPROFILE%\.codex\auth-switcher-v2`，或对应 `CODEX_HOME` 下的 `auth-switcher-v2` 目录。`modes.enc` 和 `pending.enc` 使用 AES-256-GCM；`vault.key` 与密文分开保存。Windows 目录 ACL 仅允许当前用户和 SYSTEM 访问。

此加密只保护工具自己的模式存档。复制 `vault.key` 和密文后可以解密存档，所以请按敏感数据管理备份。Codex 使用的 `auth.json` 仍保持 Codex 格式。工具不会修改 `autocode.config.toml`，也不会改变 MCP、项目或插件设置。

第一版 DPAPI 存档不会由第二版读取或覆盖。遇到旧版未完成的恢复记录时，先用第一版完成恢复。

## 命令行

先在一个不存在的临时目录创建虚构数据：

```powershell
$demoHome = Join-Path $env:TEMP "codex-auth-switcher-demo"
.\codex-auth-switcher.exe --demo --isolated --codex-home $demoHome --command initialize
```

后续 CLI 操作继续指定这个演示目录。预览和切换只会修改虚构配置：

```powershell
.\codex-auth-switcher.exe --isolated --codex-home $demoHome --command status
.\codex-auth-switcher.exe --isolated --codex-home $demoHome --command preview --target api
"synthetic-test-only-key" | .\codex-auth-switcher.exe --isolated --codex-home $demoHome --command switch --target api --api-key-stdin
```

不要将真实 key 放进命令行参数、脚本或日志。CLI 输出只包含配置摘要和切换结果。不要对真实 Codex 目录执行 `--command switch`，除非你确实要立即切换该目录。

## 隔离演示

`--demo` 只会创建虚构配置。目标目录必须不存在；`--isolated` 会忽略宿主环境中的 key：

```powershell
.\codex-auth-switcher.exe --demo --isolated --codex-home $demoHome
```

关闭演示程序后，演示目录仍保留，避免程序替用户删除数据。需要新演示时，指定另一个不存在的目录。

## 卸载

本便携包不安装服务、启动项或系统级组件。卸载时删除解压目录即可。加密模式存档保留在 Codex 配置目录中，避免误删认证状态。
