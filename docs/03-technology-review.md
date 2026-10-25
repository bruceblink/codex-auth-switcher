# 第一版后的技术重评

评估日期：2026-10-24。本文依据第一版代码、原生窗口验收、Windows CI 和文末官方资料，评估第二版的技术路线。评估不代表已经迁移，也不代表完成了候选框架的性能测试。

## 结论

第一版保留 Python + Tkinter。第二版已按用户后续决定选择 **Rust + gpui-kit**。界面需要显示模式、表单和切换预览。用户希望使用成套组件改善视觉效果，并接受该简单工具的性能需求。

这是本项目当前的技术选择。Rust 原生程序可以避免分发 Python 或 Chromium 运行时，但最终包体积仍取决于渲染后端、字体、资源和构建选项。本项目尚未对 egui、gpui-kit、Electron 或 PySide6 做同场景性能与包体积比较，不提供估算数字或性能排名。

Rust 界面路线只比较 **egui/eframe** 与 **gpui-kit**。用户已明确不直接使用原始 GPUI 构建控件。Electron 作为 Web 技术路线保留完整比较。PySide6 是保留现有 Python 核心、减少迁移工作量的备选。

## 第一版的依据

- 第一版已经完成模式初始化、API 与订阅往返、脱敏输入和失败恢复。22 个隔离测试，以及 Windows Python 3.11 / 3.14 CI 已通过。见 [验证记录](04-v1-validation.md) 和 [第一版 CI](https://github.com/bruceblink/codex-auth-switcher/actions/runs/40086573212)。
- 业务核心已与 Tkinter 分离。当前主要复杂度在 TOML 保留式修改、认证状态、外部写入检测和中断恢复，不在绘制界面。更换框架不能替代这些行为。
- 本机 Tkinter 的 UI Automation 树主要显示窗格，原生验收依赖坐标。该结果只适用于当前环境，不能推断其他框架的实际可访问性表现。
- 旧会话保存 provider、网关地址不匹配、上游认证或容量限制，都不会因更换 GUI 自动解决。

## 同标准比较

下表为本项目的适配判断。框架能力来自官方资料；具体产品质量仍需要原型验收。

| 维度 | egui + eframe | gpui-kit | Electron | PySide6 + Qt Widgets |
| --- | --- | --- | --- | --- |
| 简单配置工具 | 适配度高；布局与状态直接 | 适配；组件体系更完整 | 适配；Web 表单生态成熟 | 适配；标准桌面控件成熟 |
| 界面与主题 | 自绘工具界面；可定制，非系统原生控件 | 成套表单、表格、主题、焦点和覆盖层 | HTML/CSS，设计自由度高 | 标准控件、模型视图和 Designer |
| 跨平台界面 | eframe 支持 Windows、macOS、Linux 等 | 项目提供 Windows、macOS、Linux 支持 | 支持 Windows、macOS、Linux | 支持 Windows、macOS、Linux |
| 本项目核心复用 | 需要迁移到 Rust | 需要迁移到 Rust | 改写 Node/Rust 核心，或增加 Python 子进程 | 可直接复用现有 Python 模块 |
| 凭据与文件边界 | Rust 系统模块负责 | Rust 系统模块负责 | 主进程负责，渲染进程使用受限 IPC | Python 系统模块负责 |
| UI 测试 | egui_kittest、AccessKit、截图；补原生验收 | 组件交互测试、焦点/布局/可访问性断言；补原生验收 | Playwright 实验性 Electron 支持；补原生验收 | Qt 测试与原生验收 |
| 运行时分发 | Rust 可执行文件与实际需要的资源 | Rust 可执行文件与实际需要的资源 | Chromium、Node 和应用资源 | Python、Qt 和应用资源，或打包工具产物 |
| 版本维护 | egui 仍有破坏性变更，需锁定版本 | Kit 与底层 GPUI 快照要成组配套 | Electron、Chromium/Node 和前端依赖持续更新 | Python、Qt 与打包工具版本配套 |
| 本项目定位 | 可用的替代方案 | 第二版已选方案 | 强调 Web 生态和快速扩展时的备选 | 追求最小迁移成本时的备选 |

## egui 与 eframe

egui 是即时模式 GUI（Immediate Mode GUI）库。eframe 提供窗口、输入和渲染集成。应用在界面更新函数中读取状态、生成控件、处理用户动作。窗口刷新不等于每帧写文件；配置切换应由明确动作提交。官方资料说明 egui 可以按交互或动画请求重绘，因此不能把即时模式等同于持续满负载刷新。

对本项目的收益：

- 模式列表、输入框、状态区和操作记录可以用紧凑的 Rust 状态模型实现。无需增加 Web 渲染进程或前后端通信层。
- Rust 核心与界面可使用一套构建和测试工具。TOML、JSON、应用层加密与文件恢复逻辑可以集中维护，不依赖 Windows DPAPI。
- eframe 提供 AccessKit 集成；egui_kittest 可按控件角色和名称驱动测试，也支持截图。不能据此省略 Windows 输入法、键盘焦点和读屏验收。
- 官方 eframe 文档提供 wgpu 和 glow 后端，并说明 glow 可减少二进制体积。选择后端前仍需测试 Windows 图形驱动、远程桌面和实际包体积。

需要付出的工作：

- 现有 Python 代码不能直接复用为 Rust 实现。必须迁移配置编辑、认证检测、环境变量优先级、加密状态和中断恢复，并复用测试样例进行对照。
- 托盘、系统文件对话框、安装包、签名和自动更新需要额外集成。eframe 不是完整的桌面发布系统。
- 中文字体需要选择可分发资源或可靠的系统字体加载方式。字体会影响包体积。首尾可见密钥输入也需要独立检查复制、粘贴、选区和日志行为。
- API 仍可能变化，需要固定依赖并控制升级范围。

egui/eframe 仍可作为未来比较对象。第二版当前使用 gpui-kit；核心逻辑保持独立，不在渲染过程中执行磁盘或加密操作。

## gpui-kit

本项目使用 Longbridge 的 **gpui-kit**。该项目将底层基础能力、成套组件和资源组合成应用入口，并固定配套的 GPUI 版本。它与直接使用原始 GPUI 的开发工作量不同。项目资料提供表单、数据表格、主题、焦点、AccessKit 和 UI 交互测试支持。第二版已完成当前 Windows 主机构建和真实窗口检查。

对本项目的收益：

- 模式管理、变更预览、恢复对话框和较复杂的设置页，可以使用同一套组件和交互约定。
- Rust 核心与界面统一。若工具以后扩展成多账户、多 provider 的桌面管理器，组件体系的收益会增加。
- Kit 的示例和测试覆盖组件状态、订阅保留、焦点与布局，有助于避免完全自行建设控件层。

主要成本：

- Kit 固定底层 GPUI 快照，升级必须遵循其配套关系。本项目不能独立替换底层版本并假定兼容。
- 实体、状态、事件订阅和异步任务生命周期需要明确管理。当前实现将文件操作放在后台任务中，并将窗口和输入状态留在 UI 线程。
- 标准组件可以缩短界面开发，但认证核心的 Rust 迁移、Windows 系统集成、打包与签名仍需要完成。
- Kit 的测试窗口不证明真实操作系统窗口的输入、绘制或可访问性已通过；其示例文档也明确要求原生检查。

当前版本已按用户选择采用 gpui-kit。后续需完成更多窗口操作验收，并在需要比较包体积或资源占用时使用相同业务和打包条件测量。

## Electron

Electron 使用 Chromium 多进程模型，主进程运行于 Node.js。它提供成熟的窗口、托盘、菜单、对话框及分发生态；官方文档介绍了 Playwright 的实验性 Electron 测试接口和 Electron Forge 打包。对复杂桌面产品，它能减少界面与发布工具的自建工作。

对本项目的收益：

- 表单、列表、路由、主题和变更预览可用常见 Web 技术完成，前端调试与自动化工具完整。
- 窗口与托盘等能力有统一 API。需要复杂多页面管理界面时，开发效率较高。
- 可以在主进程实现与 Rust 路线相同的应用层加密格式，不要求使用 Electron safeStorage 或操作系统凭据存储。

主要成本与实现边界：

- 需要随应用分发 Chromium/Node。这个成本来自架构，但不能据此给出未经测量的内存或体积数字。对当前简单界面，其收益小于已有 Web 前端可复用的项目。
- 配置文件与解密放在主进程或专用工作进程。渲染进程只接收脱敏状态，通过有限 IPC 请求检查、预览和应用。需要验证消息来源和参数，启用上下文隔离与沙箱，不给页面任意文件或 Node 权限。
- Electron 路线也需要明确密钥、密文格式和旧数据导入，不能因使用框架存储 API 就假定兼容第一版存档。
- 保留 Python 子进程可以少重写核心，但会增加运行时、进程协议、取消和分发成本。这个项目不建议采用双运行时作为长期结构。
- 主进程不能被同步文件操作和长任务阻塞。更新 Electron 与前端依赖也是持续维护工作。

若未来明确优先 Web UI 开发速度、组件生态和自动化流程，可再评估 Electron。当前已选择 Rust 原生桌面界面。

## 其他方案

- **Python + PySide6 + Qt Widgets**：复用现有业务模块的成本最低。适合希望尽快改善标准桌面控件、保留 Python 的路线。需要验证 Qt 部署、DPI 和许可要求；它是备选，不再作为默认第二版方向。
- **PyQt6**：控件能力相近，但分发许可与 PySide6 不同。当前没有既有 PyQt 资产，不增加这个迁移理由。
- **Qt/C++**：桌面能力完整，但需要重写核心和维护 C++ 构建。相对于 Rust 候选，本项目收益不足。
- **Tauri 2**：使用系统 WebView，减少内嵌浏览器运行时，但保留 Rust/Web 两套工具链及通信边界。仅在明确需要 Web UI 时再比较。
- **Godot**：场景与动画能力不对应当前配置工具需求，不作为第二版候选。

## 跨平台范围

第二版按用户要求移除 DPAPI 绑定，统一使用应用层加密文件，不要求分别接入 Keychain 或 Secret Service。第一版已有 DPAPI 存档仍属于旧格式；跨平台还需要验证路径、权限、输入与分发。具体范围如下：

| 部分 | 可共享内容 | 需要单独实现和验收的内容 |
| --- | --- | --- |
| 界面 | 模式列表、表单、预览、状态 | 字体、DPI、输入法、焦点与辅助技术 |
| 配置 | TOML/JSON 解析、模式字段、校验 | 各平台 Codex 路径与文件权限 |
| 认证 | 脱敏结果、状态模型、应用层加密格式与密钥管理 | 各平台用户目录权限、随机数来源与旧数据初始化 |
| 文件恢复 | 加密恢复记录、冲突检测与回滚状态 | 文件锁、原子替换、断电与访问权限行为 |
| 分发 | 版本信息和发布流程约定 | 安装包、签名、更新、卸载和干净机器测试 |

只有对应平台的读写、加密和恢复测试通过后，才标记为已支持。

## 应用层加密存储

第二版使用统一的加密格式，不把操作系统账户绑定或系统钥匙串作为必要条件。默认采用 AES-256-GCM 认证加密（Authenticated Encryption），使用成熟库实现，不自行设计加密算法。默认不增加主密码和每次切换解锁的操作。

密钥与数据的保存方式：

- 首次初始化时，使用操作系统安全随机数生成 256 位密钥，单独保存为用户数据目录中的 `vault.key`。密钥不写死在程序中，不进入项目、设置、日志或普通备份。
- 模式状态保存为 `modes.enc`，未完成操作的恢复记录保存为 `pending.enc`。文件包含格式版本、随机 nonce、密文和认证标签；密钥不嵌入密文。每次加密生成新的 96 位随机 nonce，不能在同一密钥下复用。
- 格式版本和记录类型作为附加认证数据，防止模式文件与恢复记录被混用。原子写入、外部修改检测和失败恢复继续保留。
- 已有密文但密钥缺失、解密失败或认证标签不匹配时停止操作，不自动生成新密钥覆盖旧状态，不回退为明文保存。
- 密钥和密文使用当前用户的数据目录及文件权限。复制存档到其他机器需要同时提供对应密钥；获得两者即可解密，因此这种方案不提供 DPAPI 的账户绑定保护。跨机复制也不保证订阅令牌仍然有效。

加密范围是工具自身的模式存档和恢复记录。Codex 运行时读取的 `auth.json` 继续使用 Codex 要求的格式，不能替换为工具私有密文。该文件仍不进入项目或新建的明文认证备份。

第一版 `modes.dpapi` 和 `pending.dpapi` 不直接作为第二版文件读取。迁移时优先从当前 Codex 配置、认证和有效备份重新初始化；需要保留仅存在于旧存档中的数据时，先在原 Windows 用户下完成一次性转换。新运行时不要求保留 DPAPI 兼容模块。旧版恢复记录必须先由第一版处理，不能在未完成写入时直接切换存储格式。

## 当前实现与剩余验收

第二版已实现基础模式切换。下表记录截至 2026-10-25 的 Windows 本机验收。命令、文件状态和资源测量见[第二版开发计划的验收记录](05-v2-gpui-kit-development-plan.md#当前验收记录)。

1. **已完成（本机）**：Rust 核心测试、Windows ACL 检查和 CLI 状态、预览、切换验收通过。测试覆盖模式往返、故障恢复、过期预览、缺失密钥和损坏密文。
2. **已完成（本机）**：真实窗口验收覆盖导入、预览、应用、目录选择、键盘焦点、密钥掩码、中文输入和 720×600 最小尺寸滚动。最终包完成 API 与订阅配置导入、API 应用和订阅回切；上一包完成认证 JSON 选择器、无效 URL 和存储错误提示检查。
3. **部分完成**：文件操作由 GPUI 后台执行器调度。文件哈希确认刷新和预览没有改活动文件，初始化和应用在用户触发后写入。没有注入长耗时文件操作来量化界面负载期间的响应时间。
4. **已完成（100% 基线）**：第一版 Python 22 项测试和第二版 Rust 23 项测试均通过。Windows 显示设置为 2560×1440、100% 缩放；原生窗口为 96 DPI，功能和窗口尺寸验收通过。本次不更改分辨率或缩放，150% 与 200% 不属于验收范围。
5. **部分完成**：Windows Release 可执行文件、ZIP、单次启动时间、12 秒 CPU/GPU 采样、进程内存和缺失密钥/损坏密文行为已有本机记录。项目没有安装包，因此安装体积不适用。没有进行框架间性能比较。
6. **已完成（Windows runner）**：Release GUI 在新建的 GitHub `windows-latest` runner 上通过 UI Automation 原生窗口测试。测试覆盖初始化、目标选择、预览、API 应用和订阅恢复。隔离 CLI 测试将 PATH 限制为 `System32`。当前电脑没有 Windows Sandbox 或 Hyper-V 虚拟机。

托盘、代码签名和自动更新仍属于独立范围，不阻塞基础切换功能。GitHub Actions run `40146530089` 已通过，覆盖 Rust 打包、隔离 CLI、原生窗口 UI 与 Python 3.11/3.14 检查。

如果后续测量发现打包体积、系统兼容或辅助技术支持不符合要求，再使用同一核心比较 egui/eframe 或 PySide6。

## 资料与限制

以下为本次读取的项目或官方资料。在线主分支会变化；升级时需要重新核对并固定具体依赖版本。没有开展同条件框架性能比较，不把上游示例或性能宣传当作本项目实测结果。

- [egui 官方说明](https://github.com/emilk/egui)：即时模式、平台、重绘与版本变化。
- [eframe 文档](https://docs.rs/eframe/latest/eframe/)：窗口集成、AccessKit 与渲染后端。
- [egui 可访问性](https://github.com/emilk/egui/blob/main/docs/accessibility.md)、[egui_kittest](https://docs.rs/egui_kittest/latest/egui_kittest/)：控件语义、交互与截图测试。
- [GPUI Kit](https://github.com/longbridge/gpui-kit)、[配套依赖](https://github.com/longbridge/gpui-kit/blob/main/Cargo.toml)、[应用示例与验收边界](https://github.com/longbridge/gpui-kit/blob/main/examples/ai_recipes/README.md)：组件、版本固定和测试能力。
- [Electron 进程模型](https://www.electronjs.org/docs/latest/tutorial/process-model)、[安全配置](https://www.electronjs.org/docs/latest/tutorial/security)、[性能](https://www.electronjs.org/docs/latest/tutorial/performance)：系统边界与维护成本。
- [Electron 自动化测试](https://www.electronjs.org/docs/latest/tutorial/automated-testing)、[打包](https://www.electronjs.org/docs/latest/tutorial/tutorial-packaging)、[safeStorage](https://www.electronjs.org/docs/latest/api/safe-storage)：测试、分发及 Windows 加密。
- [Qt Widgets](https://doc.qt.io/qtforpython-6/PySide6/QtWidgets/index.html)、[PySide6 部署](https://doc.qt.io/qtforpython-6/deployment/deployment-pyside6-deploy.html)、[Qt for Python 许可](https://doc.qt.io/qtforpython-6/licenses.html)、[PyQt](https://riverbankcomputing.com/software/pyqt/)：Python 桌面备选。
- [Tauri 2](https://v2.tauri.app/start/)：系统 WebView 路线。
- [RustCrypto AES-GCM](https://docs.rs/aes-gcm/latest/aes_gcm/)：认证加密接口、密钥与 nonce 要求。
