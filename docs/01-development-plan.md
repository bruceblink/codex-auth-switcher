# Codex Auth Switcher 开发计划

## 目标

为 Windows 桌面 Codex 提供一个本地 GUI 和 CLI。用户可以在订阅 provider 与 AutoCode API key provider 之间切换。

## 范围

1. 读取 `%USERPROFILE%\.codex\config.toml`。
2. 更新顶层 `model_provider`。
3. 在 API key 模式下确保 `model_providers.autocode` 使用 `AUTOCODE_API_KEY`。
4. 写入前生成带时间戳的配置备份。
5. 显示 API key 是否存在及其来源，但不显示 key。
6. 支持用户输入 API `base_url` 和 API key；输入框显示首尾字符，中间字符显示为 `·`。
7. 可把用户输入的 API key 写入 `.codex\.env`，但不创建包含 key 的备份。
8. 保留 `autocode.config.toml`、`auth.json` 和其他 Codex 状态文件。

## 接受条件

- 订阅操作只切换到 `settings.json` 的 `subscription_provider`。
- API 操作只切换到 `settings.json` 的 `api_provider`，并保留现有 TOML 其他内容。
- 备份文件名符合 `config.toml.bak-YYYYMMDD-HHmmss`。
- 配置使用 UTF-8 和 LF 换行。
- GUI 与 CLI 都不输出密钥值。
- GUI 的 key 输入框只显示脱敏文本，配置文件不包含 key 值。
- 没有 API key 时，工具仍可完成配置切换，并给出桌面 `.env` 配置提示。

## 验证命令

```powershell
py -3 -m py_compile .\main.py .\src\codex_auth_switcher\*.py
py -3 .\main.py --target status
```

真实切换前，先复制或确认 `config.toml` 的自动备份。桌面 Codex 重启后再验证当前 provider。

## 回滚边界

只恢复同一目录中最近一次 `config.toml.bak-*` 文件。不要恢复 `auth.json`，也不要把任何 API key 放入备份或 Git。
