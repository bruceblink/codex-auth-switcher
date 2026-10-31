# egui 与 gpui-kit 维护成本评估

日期：2026-10-25。

本评估记录形成于移除 CLI 之前。表中的框架与包体对比包含 CLI 功能，仅作为历史决策依据；这些 EXE 和 ZIP 数值不代表当前仅含 GUI 的 Release 产物。原表的 egui 记录使用 Glow；本机渲染器复测结果见下文。

## 结论

按用户提出的两项切换条件，egui/eframe Wgpu 仍满足：依赖、构建和 Windows x64 便携包指标低于 GPUI Kit 基线，并且 Wgpu 在本机能正确绘制真实窗口。当前 Rust 桌面程序使用 egui/eframe Wgpu。配置核心、加密存档和恢复流程保持不变。

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

## Windows 渲染器复测

Glow 后端在本机 Windows Release 中多次显示白色客户区，但 UI 自动化仍能读取并操作控件。将同一 egui 界面改为 Wgpu 后，真实窗口截图能显示主页面和模型目录表单，原生功能验收也通过。最终 Wgpu Release 对真实配置的检查只读取当前状态和空白模型编辑器，未读取模型文件或写入配置；完整模式与模型写入验收使用隔离的临时 `CODEX_HOME`。该复测未调整屏幕分辨率或显示缩放。

| 当前 GUI 产物 | Glow | Wgpu | Wgpu 变化 |
| --- | ---: | ---: | ---: |
| Windows x64 Release EXE | 7,661,056 字节 | 12,670,464 字节 | 增加 5,009,408 字节，65.4% |
| 相同文件组成的便携 ZIP | 3,295,433 字节 | 5,267,694 字节 | 增加 1,972,261 字节，59.8% |
| Windows x64 普通依赖图唯一包 | 209 | 219 | 增加 10 个 |

Wgpu EXE 比 GPUI 基线小 6,493,696 字节，便携 ZIP 小 1,891,912 字节。Wgpu 的 Windows 依赖数量比 GPUI 基线少 209 个。Glow 与 Wgpu 测量使用当前 GUI 文件清单、相同 Rust 工具链、Windows x64 MSVC、静态运行库和 Release 参数；Wgpu ZIP 使用与正式打包相同的 EXE 和用户指南。首次 Wgpu 构建为独立目标目录的本机冷构建，包含依赖下载；不将它作为严格构建时长对比。

依赖数量来自 Windows x64 的 `cargo tree --edges normal --target x86_64-pc-windows-msvc`。行数来自 GPUI 的 `app.rs`、`view.rs`、`tests.rs`，以及 egui 的 `app.rs`、`app/ui.rs`、`app/format.rs`、`app/tests.rs`。表中的行数和测试数是形成这份框架评估时的代码快照；后续 UI 打磨会改变当前行数。评估快照中 egui 有 1,049 行，其中应用与布局代码 826 行、测试 223 行；GPUI 有 1,039 行应用代码和 440 行 UI 测试。这些是代码规模代理，不等同于后续维护工时。

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
- 当前桌面程序启用 Wgpu 渲染。它比 Glow 增加 10 个 Windows x64 普通依赖和约 1.97 MB ZIP 体积；本机 Release 窗口可见性正常。系统字体、默认字体、剪贴板、链接、Wayland 和 X11 特性保持开启。
- eframe 0.37.0 的声明 MSRV 是 Rust 1.95。本仓库当前使用 Rust stable 1.99.1；`rust-toolchain.toml` 和 CI 都跟随 stable，依赖升级仍需重新运行完整验证。
- eframe/egui 版本固定为 0.37.0。直接升级时需要重新运行 Rust 检查、Headless 测试、Windows 打包和真实窗口检查。
- egui 控件更基础。若要达到 gpui-kit 现有视觉风格，需要本项目持续维护颜色、留白、层级和交互状态。

### gpui-kit

- Kit 提供更完整的组件和一致的主题。用户反馈其视觉效果明显优于当前 egui 界面。
- 维护依赖图较大。当前基线有 428 个唯一普通依赖包和 1,047 条依赖边。
- gpui-kit 0.7.3 固定配套 GPUI 快照。升级要按 Kit 的版本配套关系处理，不能独立替换 GPUI 快照。
- Kit 的 UI 测试 API 能驱动窗口事件，但 UI 组件、状态和窗口行为需要更多应用代码。

两个框架都支持 AccessKit 与测试窗口。两种方案都需要真实 Windows 验收；Headless 窗口不证明 Windows 文件对话框、DPI、输入法或系统窗口行为正确。

## 功能和窗口验证

共享核心的 23 项 Rust 测试和 egui 界面的 6 项测试通过。UI 测试覆盖模式选择、初始化、合成 TOML/JSON 导入、预览、API 与订阅往返、应用后清除 key、选择器取消及取消后的刷新，并覆盖模型目录读取、复制、保存和备份。Release 原生验收脚本还通过隔离临时目录验证了模型目录保存会保留未编辑字段。

在 100% 显示缩放、96 DPI 的 Windows 真实窗口中，程序读取到真实 Codex 目录的当前模式为订阅，模型为 `gpt-6-luna`，推理级别为 `max`。此项只读取状态，没有更改真实配置或显示设置。

隔离目录中的 Release 窗口通过 Computer Use 操作了 TOML 和 JSON 文件选择器，导入合成文件，并完成 API 切换和订阅恢复。窗口状态与磁盘文件一致；共享 shell 设置保留，模式备份不含凭据。测试只写入临时 `CODEX_HOME`。

本次布局打磨把模式卡片的状态和字段分开显示，值在卡片内换行；模型目录编辑窗口使用分组表单、滚动区域和保存状态提示。主窗口默认高度收紧为 640 像素，API key 输入区说明留空时使用已保存认证。Glow 后端的本机 Release 画面为空白；Wgpu 对照 Release 在同一窗口和 100% 显示缩放下显示完整主界面与编辑器。Headless 测试不用于判断最终字形。原生验收只使用临时 `CODEX_HOME`，没有修改分辨率、缩放或真实 Codex 配置。

## 边界与后续验收

测量只代表本机、记录的工具链和构建参数。依赖数量、源代码行数、构建时间和包体积都是代理指标，不测量长期维护人力、图形性能或不同硬件的启动表现。原生文件导入、API/订阅往返和模型目录操作已由 Windows 原生验收脚本覆盖。当前 Wgpu 真实窗口截图显示了客户区，但其他显卡和驱动组合仍需通过 Windows 用户环境验证。

保留 egui 的依据是用户给出的“维护成本更低且便携包更小”的切换条件。视觉偏好作为明确风险记录；若产品优先级改为保持 GPUI Kit 的现有外观，应重新评估是否回切，而不能把 egui 的尺寸和依赖优势当成唯一产品标准。

## 官方资料

- [egui 发布记录](https://github.com/emilk/egui/releases)：0.37.0 发布于 2026-10-15。
- [eframe 0.37.0 文档](https://docs.rs/eframe/0.37.0/eframe/)：窗口集成、渲染器与应用接口。
- [egui AccessKit 说明](https://github.com/emilk/egui/blob/main/docs/accessibility.md)与[egui_kittest 文档](https://docs.rs/egui_kittest/0.37.0/egui_kittest/)：可访问性与界面测试。
- [gpui-kit 发布记录](https://github.com/longbridge/gpui-kit/releases)：0.7.3 与 0.8.0 的版本变更及快照说明。
