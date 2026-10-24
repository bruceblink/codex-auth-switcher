# Codex Auth Switcher

Windows 桌面 Codex 的订阅 provider 与 AutoCode API key provider 切换工具。

工具只修改 `%USERPROFILE%\.codex\config.toml` 的顶层 `model_provider` 和 API provider 的 `base_url`。每次发生配置修改前，它会在同一目录创建 `config.toml.bak-YYYYMMDD-HHmmss` 备份。它不会修改或重写 `autocode.config.toml`、`auth.json`，也不会把 API key 内容写入 `config.toml`、日志或项目设置。

## 选型结论

本项目选择 **Python + Tkinter** 完成第一版。这个工具只有状态读取、TOML 文本保留式修改、备份和两个按钮。Python 在当前 Windows 环境已经可用，Tkinter 是 Python 的标准 GUI 接口，Python 官方文档明确支持 Windows；因此第一版不需要下载 Rust crate、Node 依赖或游戏引擎资源。

| 方案 | 适合点 | 本项目的主要成本 | 结论 |
| --- | --- | --- | --- |
| Python + Tkinter | 标准库、启动快、文件和注册表操作直接 | 默认控件视觉较朴素，后续打包需要处理 Python 运行时 | 第一版采用 |
| Python + PySide6 | 官方 Qt for Python 绑定，Qt Widgets/QML 和设计工具完整 | 需要 Qt 运行时、虚拟环境和打包配置 | 需要更强视觉时优先迁移 |
| Python + PyQt6 | Qt 控件完整，Python 开发速度高，生态成熟 | GPLv3 或商业授权；不适合未确定分发许可的工具 | 有明确 PyQt 许可时使用 |
| Qt/C++ | 原生桌面能力完整，长期维护和性能稳定 | C++ 开发和构建链成本最高 | 规划长期产品时考虑 |
| egui + eframe | Rust 原生程序、单二进制方向清晰、跨平台 | 首次编译依赖较多；egui 是即时模式 UI，原生外观需要自行设计 | 适合 Rust 长期维护 |
| GPUI | 布局和视觉表达能力强，适合编辑器类产品 | 官方 GPUI README 仍标记为 pre-1.0，API 可能变化，Windows 集成和组件准备成本较高 | 不用于快速首版 |
| Electron | Web UI 开发速度高，跨平台能力成熟 | 内含 Chromium 和 Node.js，安装包和运行时开销较大 | 这个小工具不需要 |
| Godot | 场景编辑、动画和视觉交互很快，Windows 导出方便 | 它是游戏引擎；配置文件、Windows 环境变量和重启流程需要额外桥接 | 只有做可视化启动器或动画面板时考虑 |

依据： [Python Tkinter 文档](https://docs.python.org/3/library/tkinter.html)、[Qt Widgets 文档](https://doc.qt.io/qt-6/qtwidgets-index.html)、[Qt for Python 文档](https://doc.qt.io/qtforpython-6/)、[PyQt 官方说明](https://riverbankcomputing.com/software/pyqt/)、[egui README](https://github.com/emilk/egui/blob/main/README.md)、[GPUI README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)、[Electron 官方文档](https://www.electronjs.org/docs/latest/)、[Godot Windows 导出文档](https://docs.godotengine.org/en/stable/tutorials/export/exporting_for_windows.html)。

## 当前配置约定

`settings.json` 默认使用以下 provider：

- 订阅模式：`OpenAI`。这与现有 `autocode.config.toml` 中的订阅平台配置保持一致。
- API key 模式：`autocode`，读取 `AUTOCODE_API_KEY`，请求地址为 `https://vip.auto-code.net`。

如果你的订阅模式实际使用内置 OpenAI provider，把 `settings.json` 的 `subscription_provider` 改为 `openai`。工具不会替你修改 `autocode.config.toml`。

GUI 的 API key 输入框只显示首字符、末字符，中间字符使用 `·`。默认会把用户输入的 key 写入 `%USERPROFILE%\.codex\.env`，这样桌面 Codex 重启后可以读取；可以取消复选框来避免保存。工具不会创建包含 key 的备份。

桌面 Codex 可能不会继承从终端设置的环境变量。也可以手动在 `%USERPROFILE%\.codex\.env` 中配置：

```dotenv
AUTOCODE_API_KEY=<your-api-key>
```

不要把真实 key 写进这个项目或提交到 Git。Base URL 只写入当前 Codex provider；勾选“记住 base_url 为默认值”后，工具才会更新项目的 `settings.json` 默认值。

## 使用

在 PowerShell 中从项目根目录运行：

```powershell
py -3 .\main.py
```

命令行状态检查不会输出 key：

```powershell
py -3 .\main.py --target status
```

命令行切换也可用：

```powershell
py -3 .\main.py --target subscription
py -3 .\main.py --target api
```

切换完成后，完全退出并重新打开桌面 Codex，再开始新任务。

## 项目边界

- 只处理本机 Windows Codex 配置。
- 不调用远程 API，不验证 key 余额，不发送用户代码。
- 不自动注销或重新登录 ChatGPT。
- 不自动关闭桌面 Codex，避免丢失未保存的任务状态。
