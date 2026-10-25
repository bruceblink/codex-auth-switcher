# egui 与 gpui-kit 维护成本评估

日期：2026-10-25。

## 结论

按用户提出的两项切换条件，egui/eframe 都满足：维护量代理指标更低，Windows x64 便携包也更小。当前 Rust 桌面程序已切换到 egui/eframe。配置核心、CLI、加密存档和恢复流程保持不变。

GPUI Kit 的界面视觉更完整。egui 候选版以较少依赖和更小包体换取维护成本下降，但需要应用自行维护间距、颜色、卡片和状态层级。用户指出的视觉差距是真实取舍，不能由包体数据抵消。

## 同条件测量

基线是切换前的 `switcher-desktop` + gpui-kit 0.7.3。候选版使用相同 `switcher-core`、CLI、`rfd` 文件对话框、功能和便携包清单，只替换图形界面。两边均使用 Rust/Cargo 1.98.1、Windows x64 MSVC、独立干净的构建目录、Release 参数 `opt-level = "s"`、strip、thin LTO，以及静态 MSVC 运行库。

| 测量项 | gpui-kit 0.7.3 | egui/eframe 0.37.0 | egui 变化 |
| --- | ---: | ---: | ---: |
| Release EXE | 19,164,160 字节 | 7,826,944 字节 | 少 11,337,216 字节，59.16% |
| 相同文件组成的 ZIP | 7,159,606 字节 | 3,368,452 字节 | 少 3,791,154 字节，52.95% |
| Windows x64 普通依赖图中的唯一包 | 428 | 209 | 少 219，51.17% |
| 普通依赖边 | 1,047 | 423 | 少 624，59.60% |
| 全新 Release 构建时间 | 159.22 秒 | 69.33 秒 | 少 89.89 秒，56.46% |
| 桌面 UI 与测试 Rust 行数 | 1,479 行 | 1,049 行 | 少 430 行，29.07% |

两个 ZIP 均只包含 EXE 和同一份 `docs/06-v2-user-guide.md`。项目没有 Windows 安装程序，所以这里比较的是便携 ZIP 和 EXE，不称为安装包体积。压缩包以 PowerShell `Compress-Archive -CompressionLevel Optimal` 生成。

依赖数量来自 Windows x64 的 `cargo tree --edges normal --target x86_64-pc-windows-msvc`。行数来自 GPUI 的 `app.rs`、`view.rs`、`tests.rs`，以及 egui 的 `app.rs`、`app/ui.rs`、`app/format.rs`、`app/tests.rs`。egui 当前共 1,049 行，其中应用与布局代码 826 行、测试 223 行；GPUI 有 1,039 行应用代码和 440 行 UI 测试。这些是代码规模代理，不等同于后续维护工时。

| 对照产物 | SHA-256 |
| --- | --- |
| GPUI EXE | `D4886132997FD13199F3F41FCC8BF87F7F9CF2457BF8E4C92281E689B2F382BE` |
| GPUI ZIP | `D03F00BC985BF69446CDFDDF7E67D3463282A5D521E20769A4C509863570B7B7` |
| egui EXE | `63A7A951DAD91F8A576CFDB7F9774D5909B04DA56C2856D15A39ED6D65BEB2D8` |
| egui ZIP | `E6F8C4566931B2FA822821E55B4A04EB821A27FB1E5C6E59F317777C6CC64F9D` |

## 维护成本比较

### egui/eframe

- 当前界面分为应用状态与后台操作、控件布局、错误文字和交互测试四个模块。模式、输入、预览和文件对话框都使用 Rust 类型。
- AccessKit 语义保留。`egui_kittest` 可按按钮角色和名称驱动界面，候选版有 4 个 UI 测试。
- 本项目只启用 glow 渲染，不使用 WGPU 作为桌面渲染后端。系统字体、默认字体、剪贴板、链接、Wayland 和 X11 特性保持开启。
- eframe 0.37.0 的声明 MSRV 是 Rust 1.95。本仓库的 MSRV 因此从 1.91 提高到 1.95。当前 CI 使用 stable，但需要确保未来依赖升级仍满足该要求。
- eframe/egui 版本固定为 0.37.0。直接升级时需要重新运行 Rust 检查、Headless 测试、Windows 打包和真实窗口检查。
- egui 控件更基础。若要达到 gpui-kit 现有视觉风格，需要本项目持续维护颜色、留白、层级和交互状态。

### gpui-kit

- Kit 提供更完整的组件和一致的主题。用户反馈其视觉效果明显优于当前 egui 界面。
- 维护依赖图较大。当前基线有 428 个唯一普通依赖包和 1,047 条依赖边。
- gpui-kit 0.7.3 固定配套 GPUI 快照。升级要按 Kit 的版本配套关系处理，不能独立替换 GPUI 快照。
- Kit 的 UI 测试 API 能驱动窗口事件，但 UI 组件、状态和窗口行为需要更多应用代码。

两个框架都支持 AccessKit 与测试窗口。两种方案都需要真实 Windows 验收；Headless 窗口不证明 Windows 文件对话框、DPI、输入法或系统窗口行为正确。

## 功能和窗口验证

共享核心和 CLI 的 20 项 Rust 测试通过。egui 界面的 4 项测试通过，覆盖模式选择、初始化、真实读取合成 TOML/JSON 来源、预览、API 与订阅往返、应用后清除 key、选择器取消及取消后的刷新。打包版 Headless 检查也通过，覆盖导入数据、脱敏预览、认证恢复和共享设置保留。最终 workspace Release 构建生成 7,826,944 字节 EXE 和 3,368,452 字节 ZIP；Headless 检查直接运行 ZIP 解压后的 EXE。

在 100% 显示缩放、96 DPI 的 Windows 真实窗口中，已操作初始化、订阅/API 目标选择、预览、API 应用和订阅恢复。CLI 及合成目录文件确认最终为订阅模式，模型为 `demo-subscription-model`，认证为 `chatgpt`，共享配置保持不变。没有更改显示分辨率或缩放。

真实窗口中的文件导入尚未通过本轮验收。TOML 系统选择器可以打开，但 Computer Use 无法在对话框窗口中继续输入选择；窗口句柄随后失效，候选程序退出。此结果只说明 GUI 自动化未完成，不能据此报告导入功能在原生窗口中通过或失败。导入路径已由 Rust UI 测试和隔离 Headless 检查实际执行。

实际中文界面显示正常。Headless 截图缺少中文系统字体适配，因此不使用 Headless 画面判断最终字形。当前窗口的卡片已改为双列等宽布局，并给目录输入框的选择和刷新按钮留出空间；尚未把 egui 的视觉精细度调整到 GPUI Kit 的水平。

## 边界与后续验收

测量只代表本机、记录的工具链和构建参数。依赖数量、源代码行数、构建时间和包体积都是代理指标，不测量长期维护人力、图形性能或不同硬件的启动表现。egui 的 Windows 文件导入仍需在后续可用的原生窗口验收中完成。

保留 egui 的依据是用户给出的“维护成本更低且便携包更小”的切换条件。视觉偏好作为明确风险记录；若产品优先级改为保持 GPUI Kit 的现有外观，应重新评估是否回切，而不能把 egui 的尺寸和依赖优势当成唯一产品标准。

## 官方资料

- [egui 发布记录](https://github.com/emilk/egui/releases)：0.37.0 发布于 2026-10-15。
- [eframe 0.37.0 文档](https://docs.rs/eframe/0.37.0/eframe/)：窗口集成、渲染器与应用接口。
- [egui AccessKit 说明](https://github.com/emilk/egui/blob/main/docs/accessibility.md)与[egui_kittest 文档](https://docs.rs/egui_kittest/0.37.0/egui_kittest/)：可访问性与界面测试。
- [gpui-kit 发布记录](https://github.com/longbridge/gpui-kit/releases)：0.7.3 与 0.8.0 的版本变更及快照说明。
