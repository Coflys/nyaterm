# PR #1006 修订实施记录

日期：2026-10-30。工作区：`E:\Projects\Rust\nyaterm-pr-1006`，分支：`review/pr-1006`，原始基点：`0111867f0`。修复按主题提交，并合入 PR 源分支截至 `deeb32ae2` 的更新；本记录随修复推送到 PR #1006，不发布 GitHub review。原 `main` 工作区保持不变。

## 后端与配置

- 运行后端仅有 `Builtin` 和 `Windows`，默认内置 IronRDP。删除 Auto 界面、平台自动选择及创建失败自动回退。
- 保存名称仍为 `builtin`、`windows`。缺失值及旧 `auto`、`internal` 读取为内置，旧 `active-x` 读取为 Windows；再次保存写明确名称。未知字符串在保存模型中保留，编辑其他字段不覆盖；连接解析返回 `Unsupported`。
- 普通启动在加载密码及创建 Relay 前检查平台、后端和路由；恢复、重试及运行入口继续检查能力。ActiveX 不支持非 Windows 平台或 Relay。IronRDP manager/helper 同样拒绝 Windows 配置，防止误路由。
- 非 Windows 编辑器保留并显示已有 ActiveX 值，但禁止新选择。切换后端不会改写原指纹策略或剪贴板设置。不兼容的系统认证、剪贴板、代理设置在保存和连接时报告错误。六种语言说明同步更新。
- `RdpSecuritySettings.windows_authentication` 是可选的后端专用配置。ActiveX 必须显式选 `system`，不会将 TOFU 自动转换为 Windows 系统验证，也不读取或修改 NyaTerm 指纹记录。内置继续原有证书流程。
- ActiveX 设置 `AuthenticationLevel = 1`，Windows 验证失败时拒绝；剪贴板仅允许关闭或系统完整重定向，拒绝仅文本。文件与格式能力仍受 Windows 和服务端策略限制。自动重连开关、次数及显示尺寸/DPI 显式映射。

## ActiveX 进程与标签嵌入

新增 `nyaterm-activex-helper`。原 desktop 的 ActiveX 实现已删除，COM/OLE、原生子窗口、网络控件及事件订阅全部迁入 helper。desktop 只保留布局、显隐、焦点、状态展示和非阻塞 IPC 代理。

helper 主线程拥有 STA，初始化与清理配对，创建、调用和销毁控件均在此线程执行。stdin 线程只向有界队列投递类型化命令。IPC 协议升级到版本 12，RDP/VNC 生命周期测试同步更新。代理复用有界写入、helper 路径解析及进程回收逻辑；凭据仅经管道传递，不进入参数或日志。

窗口以跨进程 `WS_CHILD` 嵌入应用父窗口，创建前验证父 PID/HWND 并匹配 DPI awareness。几何更新保留最新值，GPUI 不等待 COM 或进程结束。FitWindow 继续使用面板测量及 600ms 回退；后台恢复标签可用保存尺寸启动隐藏会话。

移除全部屏幕截图、BGRA 复制与 GPUI 图片上传。菜单、确认框、锁屏、隐藏标签及不可见窗口驱动原生窗口隐藏，GPUI 显示占位，会话继续。关闭遮挡后检查标签和窗口可见性，使用不激活窗口的显示方式。UI 包装层通过弹出菜单自身的焦点归属识别标题菜单、下拉菜单、上下文菜单和子菜单，避免仅靠 desktop 的菜单状态清单。

订阅连接、登录、断开、致命错误和自动重连事件；`Connected` 按 0/1/2 数值检查。首次连接有 30 秒截止时间，重连有 120 秒截止时间。心跳 10 秒未应答转为超时；管道关闭转为宿主意外退出；终止状态不残留 Connecting。正常关闭给 750ms 宽限，之后 kill/wait。独立 EOF watcher 可在 STA 卡死时回收孤儿进程。显式重试关闭旧宿主并重新建立会话，不切换后端。

Windows 发布包包含新 helper，其他平台不打包。workspace 可在非 Windows 编译并运行平台无关测试。

## 设置与会话

普通设置、主题草稿及主题提交状态统一参与退出检查，顺序为主题、普通设置、会话、编辑器、持久化。普通设置提示使用现有“放弃更改／取消”；设置 feature 独占待续接的会话数量，重复关闭复用提示。放弃恢复草稿和预览，取消保留草稿、发出 `ShutdownCancelled` 并清理退出批准状态。

跨窗口进程退出逐个访问有草稿的窗口，确认完成前不释放进程资源。托盘隐藏路径保持草稿。新增测试还修复了主题放弃后续接普通设置确认时的根视图重入，以及放弃未保存主题后残留预览的问题。

移除本 PR 新增的按保存连接 ID 强制跳转已有 RDP 会话逻辑，恢复显式创建多个独立实例，保留通用重复连接提示。原有 RDP/VNC 已连接标签跨窗口移动限制继续存在，本轮未扩展该功能。

## 与 PR 最新提交集成

保留源分支在原始基点之后的 4 个提交，包括 Linux 条件编译、macOS 测试兼容、RDP 后端下拉框派发以及远端 DPI 修复。采用合并提交保留历史，不重写源分支。

DPI 修复迁入独立 helper：在 `Connect` 前通过 `IMsRdpExtendedSettings` 设置初始桌面缩放；后续尺寸更新保留桌面 DPI，并统一设备缩放为 100。helper 生命周期测试使用 150% 初始桌面缩放创建真实控件，另有参数测试验证重设尺寸不丢失缩放。桌面布局适配复用内置路径的面板测量逻辑，未恢复旧的进程内 COM 或截图路径。

## 自动验证

最终集成提交：`1c27aec48`。集成后重新运行 Windows workspace check、完整测试、严格 Clippy、fmt 和架构检查，以及 Linux workspace/all-targets 严格 Clippy，均通过。

Windows 使用 Rust 1.99.0 / MSVC，Linux 使用 WSL Ubuntu 与隔离 Rust 1.99.0 工具链。Linux 系统开发库按仓库 `scripts/ci/install-linux-deps.sh` 补齐。日志位于工作区 `target/pr1006-*.log`，不作为源码提交。

| 检查 | 结果 |
| --- | --- |
| Windows `cargo check --workspace --locked` | 通过 |
| Windows `cargo test --workspace -j 2 --locked` | 合入源分支最新提交后复验通过：4,624 项通过、32 项按既有标记忽略、0 项失败 |
| Windows `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| Windows `cargo fmt --all -- --check` | 通过 |
| Linux core / remote-desktop / ActiveX helper 测试与严格 Clippy | 通过 |
| Linux workspace / all-targets 严格 Clippy | 合入源分支最新提交后复验通过 |
| Python 发布打包及产物验证测试 | 49 项通过 |
| `python scripts/ci/check_architecture.py` | 通过 |
| `git diff --check` | 通过 |

新增及调整测试覆盖配置别名、默认值、未知值及编辑保存；系统认证和剪贴板能力限制；Relay 预检查；无 helper/待启动任务的失败与重试；设置取消、放弃、重复关闭、非活动设置页、主题与普通设置先后顺序、跨窗口草稿及取消后重试；同一保存连接的独立会话。既有 FitWindow、600ms 回退、重连分类、错误复制和 RDP/VNC helper 生命周期测试继续通过。

ActiveX helper 的 7 项 Windows 生命周期测试通过，包含真实跨进程 HWND 嵌入和反复显隐、未曾连接成功时的拒绝端口失败、协议不匹配、正常关闭、崩溃 EOF、父管道 EOF，以及挂起 STA 后的孤儿回收。标题菜单 8 项回归测试通过，并增加打开/关闭时的统一遮挡查询断言。

## 剩余人工验收

- 未提供可用 RDP 测试服务器，尚未验证真实登录、Windows 证书拒绝、已有指纹隔离、剪贴板及文件重定向、服务端断开和自动重连次数。
- 尚未完成真实应用的标签切换、分屏、多窗口、菜单/弹窗、锁屏、快捷键、焦点、最小化/托盘恢复，以及 150%/200% DPI 和跨显示器移动验收。自动 HWND 和状态测试不能替代这些检查。
- 尚未在 macOS 实机运行构建和测试；Linux 运行了受影响平台无关包测试和全 workspace 严格 Lint，未运行完整 GPUI 图形交互验收。
- 未运行完整发布安装包构建及安装，已验证 helper 打包选择及产物检查脚本。
- 本轮保留源分支已有的 macOS 测试兼容修复；尚无 macOS 实机验证结果。Windows workspace 和 Linux 受影响包测试未出现此前提及的 `stat -c` 错误。

这些限制意味着本轮完成源码实现及上述自动回归，不能视为 ActiveX 已通过全部平台和真实服务器发布验收。


## 后续修复：恢复队列、显示降级与遮挡

本轮以 `bb076d5de` 为起点，先快进集成作者的 `c6d862de1`（后端下拉框跨平台测试修正），随后实施以下修复。保留两个明确后端、独立 ActiveX helper 和设置标签页，不改变认证、剪贴板、配置保存或退出保护契约。

- 原生会话周期维护后统一执行 `settle_remote_desktop_restore()`。真实连接事件和测试使用同一状态应用函数；Connecting/Reconnecting 保持等待，成功、失败、超时和宿主错误解除阻塞并继续恢复队列。重复维护不会重复续接。
- helper 的 `DisplayUpdates` 分开记录初始/已应用参数与待更新参数，忽略重复参数和仅位置变化，Fixed 连接成功不再无条件调整分辨率。动态调整返回错误时清除待请求、禁用本连接后续调整，保留 SmartSizing 和会话；新宿主重新探测。COM 状态读取、连接错误、崩溃和心跳超时的失败/回收路径保持原样。
- `NativeRdpSessionDetails.dynamic_resize_available` 是带 `serde(default)` 的可选字段。旧消息可读取，协议版本继续为 12；已打开的会话详情在能力变化时收到重绘通知。六种语言同步说明降级到 SmartSizing。
- `nyaterm-ui::nya_popup_is_active()` 按窗口统一查询 PopupMenu 和 Select。Select 注册弱实体并读取现有 `is_menu_focused()`，不维护打开标记，不修改依赖分支。关闭、销毁和重建均使旧状态失效。
- root 自绘层改用一份只读派生快照，渲染和 ActiveX 显隐共用；新增覆盖传输菜单、外部编辑同步提示、粘贴确认、终端操作和模态子窗口。遮挡解除后仍检查窗口、标签和会话状态，不恢复焦点。跨窗口隔离有回归测试。
- 保留作者对 Windows/非 Windows 新选择的不同预期；补充已有 Windows 值在所有平台保留并主动切回 Builtin 的测试，不跳过平台测试。

本轮测试还覆盖初始尺寸不重复调整、Fixed/DPI、拒绝调整后不再调用和新宿主恢复探测、旧/新详情消息、恢复队列续接与重连等待、Select 关闭/重建/销毁、PopupMenu 关闭/销毁、自绘多层遮挡和隐藏标签。恢复续接测试使用后续 SSH/RDP 恢复描述验证队列继续处理（缺失连接按原逻辑跳过），不模拟真实 SSH/RDP 登录。

### 本轮验证

| 检查 | 最终结果 |
| --- | --- |
| Windows `cargo check --workspace --locked` | 通过 |
| Windows `cargo test --workspace -j 2 --locked` | 4,636 项通过、0 失败、32 项沿用既有忽略标记；desktop 2,367 项通过 |
| Windows `cargo fmt --all -- --check` | 通过 |
| Windows `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| Linux `cargo test -p nyaterm-desktop -p nyaterm-ui -p nyaterm-remote-desktop -p nyaterm-activex-helper --locked -j 2` 及 UI 复验 | desktop 2,335 项通过、9 项既有忽略；UI 141 项、remote-desktop 79 项、ActiveX helper 单元与生命周期各 4 项通过 |
| Linux `cargo clippy --workspace --all-targets --locked -j 2 -- -D warnings` | 通过 |
| `python scripts/ci/check_architecture.py` | 通过 |
| `git diff --check` | 通过 |

Windows 的三个 helper 生命周期套件均随 workspace 测试通过；ActiveX 套件为 7 项，包含真实 HWND 创建/显隐和挂起 STA 回收。Linux 确实执行了 desktop 测试，未以 Lint 代替。首次并行跨平台编译触发本机内存不足，改为串行后完成上述验证。新增用例的独立存储目录及 GPUI 延后销毁时序问题已修正并复验。日志保存在未跟踪的 `target/pr1006-followup-*.log`；PR 三平台测试与 Rust gate 以推送后的运行结果为准。

### 本轮实机限制

没有可用 RDP 测试服务器，尚未实测服务器拒绝动态显示调整后仍保持连接。Windows 的真实应用分屏、菜单/Select、遮挡恢复及 DPI/跨显示器移动仍未完成人工验收；现有真实 HWND 生命周期测试与纯状态测试不能替代这些项目。macOS 的自动验证以推送后的 PR CI 状态为准，未在本地 macOS 实机验收。此前提及的未修改测试 `stat -c` 问题不混入本轮修复。
