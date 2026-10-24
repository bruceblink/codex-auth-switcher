# Codex Auth Switcher

Windows 桌面 Codex 的订阅与 API key 模式切换工具。第一版使用 Python、Tkinter、tomlkit 和 Windows DPAPI。

工具同时恢复 `config.toml` 的模式参数和 `auth.json` 的认证。它不依赖、不修改 `autocode.config.toml`。配置中的 MCP、项目、插件、代理和其他共享设置继续使用当前值。

## 安装与启动

需要 Windows 和 Python 3.11+，Python 安装中需要包含 Tcl/Tk。

```powershell
py -3 -m pip install -e .
py -3 .\main.py
```

也可以运行 `run-gui.cmd`。本版从源码启动，尚未提供独立安装包。

## 初始化与切换

1. 打开工具，确认主配置目录。默认使用 `CODEX_HOME` 或 `%USERPROFILE%\.codex`。
2. 点击“初始化 / 补齐模式”。工具读取当前配置和已有 `config.toml.bak-*`、`auth.json.bak*`。已有加密状态不会被自动覆盖。
3. 检查两种模式的 provider、模型、推理级别和认证状态。需要自定义来源时，使用“导入来源”菜单选择对应文件。
4. 在正式切换前关闭桌面 Codex，避免 Codex 同时刷新配置或登录令牌。
5. 点击目标模式。切换完成后重启 Codex，并开始新任务。

状态“已就绪”只表示本机配置和认证结构完整。工具不会联网验证令牌是否过期、余额或网关可用性。缺少目标认证时停止切换。需要登录时，使用 Codex 自身的登录流程，再重新导入认证。

## 配置与输入

- 订阅默认 provider：`openai`。
- API 默认 provider：`OpenAI`，默认地址：`https://vip.auto-code.net/v1`。
- `openai` 与 `OpenAI` 是不同的 ID。初始化优先导入实际配置，不用项目默认值覆盖已有网关。
- API 模式保留现有 provider 的地址和能力字段，使用 `requires_openai_auth = true` 从 `auth.json` 读取 key。旧 `env_key` 配置在提供有效 key 后转换为这个保存方式。
- UI/CLI 显式输入优先于环境变量，再使用已保存模式和项目默认值。可用环境变量包括 `CODEX_MODEL_PROVIDER`、`CODEX_API_PROVIDER`、`AUTOCODE_BASE_URL`、`AUTOCODE_API_KEY`。
- 空地址输入保留目标模式地址。环境变量 key 按当前进程、用户、系统顺序读取，最后兼容读取已有 `.codex/.env`。
- API key 输入只显示首尾字符，中间使用 `·`。最终 key 写入 Codex 的 `auth.json`，模式存档通过 DPAPI 加密。工具不再向 `.env` 新增明文 key。
- “记住 base_url 为默认值”只更新项目 `settings.json` 中的非敏感默认地址。

模式状态保存在 `.codex/auth-switcher/modes.dpapi`。DPAPI 将数据绑定到当前 Windows 用户，不支持直接复制到其他账户使用。该目录不属于项目，不得提交到 Git。

## 命令行

```powershell
py -3 main.py --target status
py -3 main.py --target initialize
py -3 main.py --target subscription
py -3 main.py --target api
```

指定目录与初始化来源：

```powershell
py -3 main.py --codex-home C:\test\codex --target initialize --subscription-config C:\test\subscription.toml --subscription-auth C:\test\subscription-auth.json
```

API 切换支持 `--provider`、`--api-base-url`、`--remember-base-url` 和 `--api-key-stdin`。key 只能通过标准输入或环境变量提供，不能放在命令参数中。`--settings` 可指定独立的默认设置文件。

## 备份与失败恢复

- 配置变化前创建 `config.toml.bak-YYYYMMDD-HHmmss`。同秒操作使用数字后缀，已有备份不会被覆盖。
- 如果当前 TOML 包含可识别的明文凭据，停止备份和切换，避免复制凭据。旧备份只提取模式参数，不恢复无关服务凭据。
- 工具不创建新的明文 `auth.json` 备份。切换过程使用加密的 `pending.dpapi` 保存恢复记录。
- 配置与认证分别原子写入。第二个文件写入失败时恢复先前状态。进程中断后，下次初始化或切换会尝试恢复。
- 如果外部程序修改了文件，恢复会停止，不覆盖外部修改。保留 `pending.dpapi`，关闭 Codex 后处理冲突。
- 文件检查不能锁住 Codex 自身；正式切换前关闭 Codex 是必要前提。

## 旧会话与 401

旧会话可能保存原始 provider。把主配置从 `openai` 改为 `OpenAI`，不能保证旧会话同步改变。若网关 key 被发往 `https://api.openai.com/v1/responses`，先检查实际请求地址和旧会话 provider，不要仅凭这个错误判断网关 key 无效。

本版不修改 Codex 的 SQLite 数据库或历史会话。切换后重启 Codex并使用新任务。旧会话迁移与远程认证成功不在本版完成范围内。

## 开发与验证

```powershell
py -3 -m unittest discover -s tests -v
py -3 -m compileall -q main.py src tests
py -3 tests/gui_acceptance.py
```

自动测试使用临时目录和虚构认证数据。GUI 验收程序使用被 Git 忽略的 `build/ui-acceptance` 目录，不操作真实 Codex。

- [第一版开发范围](docs/01-development-plan.md)
- [认证模式恢复计划](docs/02-auth-mode-recovery-plan.md)

工具不发送远程请求、不自动登录、不自动关闭 Codex。当前仅支持 Windows 的文件认证存储，不支持系统钥匙串认证、跨账户迁移和无人值守登录。
