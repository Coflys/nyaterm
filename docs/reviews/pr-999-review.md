# PR #999 本地审查、修复与验证

审查日期：2026-10-27。本报告记录独立 worktree 中的修复与验证结果；后续提交和推送状态以 PR 历史为准。未执行 PR 评论或合并。

## 工作区与依据

- 原仓库：`E:\Projects\Rust\nyaterm`。本次不修改原工作区的其他进行中改动。
- 修复 worktree：`E:\Projects\Rust\nyaterm-pr-999`。
- 分支：`fix/pr-999-rdp-review`。
- 重新获取的 main：`b9ffd20189545b8654b097d6273ebeb158acedae`。
- 重新获取的 PR HEAD：`3e71f755c2348ad49ee16a5bac52bb91bfac0ee2`，相对 main 为一个提交、38 个文件；修复以此 HEAD 为基础。
- 按附件和本仓库 AGENTS.md 核对完整差异及调用链；依赖行为以 Cargo.lock 和实际编译缓存中的固定 fork 为依据。

固定依赖证据：

| 依赖 | 实际固定版本及核对结论 |
| --- | --- |
| tracing-subscriber | 0.3.23，默认启用 tracing-log；`src/util.rs:61` 的初始化先安装 Subscriber，再安装 LogTracer。手动注册 Logger 后自动桥接报错，不能用 `.ok()` 忽略。 |
| GPUI / zed | `c054331114f9abdc44e5d587cd23314db6ac5aff`。Windows `directx_devices.rs:121` 的 Using GPU 日志在设备创建尝试之前；`directx_renderer.rs:823` 的 gpu_specs 读取实际设备适配器和 DXGI 软件标志，Windows window 实现已提供该 API。 |
| IronRDP | `1bc71d95c6414d5603792cb6cf8913e903205a33`。client 未覆盖性能选项时使用 PerformanceFlags::default，pdu `client_info.rs:886` 为 `0x86`；Progressive 的 Simple / First / Upgrade 解码与相关测试均存在，不能据 PR 注释认定整个格式不支持。 |

没有修改这些 revision、fork 分支或 `temp/vendor`。

## 问题与修复清单

以下位置相对于 worktree 根目录，行号指本地修复后代码。

| 问题 / 是否存在 | 根因与修复 | 主要位置 | 影响 |
| --- | --- | --- | --- |
| P0 Logger 重复注册：存在 | 删除自定义 LogBridge，使用 tracing-subscriber 自带桥接；init_tracing 返回 `Result<Option<WorkerGuard>>`，启动函数传播错误。文件 appender 不可用时明确警告并保留控制台。 | [app/main.rs](../../crates/nyaterm-app/src/main.rs):37、84；[core diagnostics](../../crates/nyaterm-core/src/diagnostics/mod.rs) | 初始化异常现在会使启动失败并返回错误；正常 JSONL、控制台、轮转、导出和 RUST_LOG 路径保留。 |
| P0 RDPEGFX 开关无效：存在；完全不支持 Progressive 的笼统解释：证据不成立 | 移除可交互开关，显示本版本不可用说明；持久化字段和显式值保留。Helper 策略仍对 true/false 请求均返回 false；修正注释，不开放未经实机验证的路径。 | [RDP editor](../../crates/nyaterm-desktop/src/features/pages/connections/editor/connection/rdp.rs)；[rdp-helper/main.rs](../../crates/nyaterm-rdp-helper/src/main.rs):600、669 | EGFX 本次仍不可用；不影响原有位图路径，也不覆盖用户保存的请求。 |
| P0 新字段与 IPC 10 混用：存在 | IPC 升至 11，新字段补齐 serde 默认值。应用仅在 ServerHello 验证成功后发送 Connect；Helper 在拒绝不匹配版本前刷出实际版本。双方均拒绝不匹配版本，握手 EOF/超时提示检查配套 Helper。 | [protocol.rs](../../crates/nyaterm-remote-desktop/src/protocol.rs):7、113；[session.rs](../../crates/nyaterm-remote-desktop/src/session.rs):900；[vnc.rs](../../crates/nyaterm-remote-desktop/src/vnc.rs):854；两个 Helper 的 main.rs 与 lifecycle.rs | 应用和 RDP/VNC Helper 必须配套升级；IPC 10 二进制不可与本次应用混用。磁盘配置不因此失去兼容性。 |
| P1 旧连接默认显示回归：存在 | 配置层、IPC 层 Default 和缺字段反序列化统一为旧 Helper 性能标志 `0x86`；逐位测试正反向布尔映射。普通连接和启动恢复继续共用转换。 | [connection.rs](../../crates/nyaterm-core/src/models/connection.rs):656；[protocol.rs](../../crates/nyaterm-remote-desktop/src/protocol.rs):155、180；[rdp-helper/main.rs](../../crates/nyaterm-rdp-helper/src/main.rs):678 | 对缺少新字段的旧连接恢复旧显示行为；已有显式选项不迁移、不覆盖。 |
| P1 GPU 误判及日志候选设备混淆：存在 | 删除日志字符串解析、全局设备名锁；窗口打开后一次读取 Window::gpu_specs，缓存于现有 shell 诊断状态；依据软件标志区分 Hardware / Software，无结果或空设备名为 Unknown。 | [platform/mod.rs](../../crates/nyaterm-desktop/src/platform/mod.rs):33；[shell/state.rs](../../crates/nyaterm-desktop/src/features/shell/state.rs):585；[root.rs](../../crates/nyaterm-desktop/src/features/root.rs):181 | 诊断表示本地 GPUI 渲染设备，不代表远程 GPU；不根据远程会话或设备名称推断 WARP。 |
| P1 会话诊断含义混淆：存在 | 本地远程会话状态移到 Client details，非 Windows 显示不可用；标签明确为 NLA/管理会话/分辨率/色深等配置请求。安全协商未获得时不可用；RTT、带宽、帧率未测量。图形路径描述为位图更新，不声称必然无压缩。 | [session_state.rs](../../crates/nyaterm-desktop/src/features/session/session_state.rs):119；[locales](../../crates/nyaterm-desktop/locales) 中六种语言 | 不再把请求或静态策略显示为服务端协商结果；没有新增伪造的 Codec/性能测量值。 |
| P1 stderr 无界增长、敏感原文转发和线程丢失：存在 | 固定 4 KiB 读取块，每行最多保留 4 KiB，超出部分持续排空。只接受严格数字字段的受控 options 记录；其他文本和无效 UTF-8 丢弃计数。线程返回有界摘要，退出清理时 join 后记录；排空期间不调用 tracing。线程创建失败 kill/wait 子进程并返回错误。 | [helper_process.rs](../../crates/nyaterm-remote-desktop/src/helper_process.rs):250、344、430；[session.rs](../../crates/nyaterm-remote-desktop/src/session.rs):1232；[vnc.rs](../../crates/nyaterm-remote-desktop/src/vnc.rs):1183 | 原始第三方 stderr 不再进入日志；运行中错误仍走 typed IPC。受控摘要在退出后以 DEBUG 记录，降低默认日志噪声及泄漏风险。stdout IPC 不变。 |
| P1 会话窗口窄屏/缩放边界：存在 | 宽度取视口可用宽度与 560px 较小值；去除强制最小高度；按实测高度限制拖动，视口及内容重排后再次校正。标题和新增关闭按钮位于滚动区外；保留已有 scroll handle、父级 scrollbar 和文本选择。 | [session_overlays.rs](../../crates/nyaterm-desktop/src/features/panels/session_overlays.rs):18、168；[session/state/mod.rs](../../crates/nyaterm-desktop/src/features/session/state/mod.rs):827 | SessionFeatureState 保持唯一所有权；关闭重开重置位置、拖动及滚动；仅状态变化时通知重绘。 |
| selectable_overlay_layer 必然事件穿透：未复现 | 保留既有 occlude/选择机制，增加背景指针、父级及滚动处理器隔离测试；移除会话窗口父级点击抢焦点的行为。 | [chrome.rs](../../crates/nyaterm-desktop/src/features/view_widgets/chrome.rs):563；[session_overlays.rs](../../crates/nyaterm-desktop/src/features/panels/session_overlays.rs):279 | 不扩大 overlay 基础设施变更范围；既有选择复制和相关焦点测试继续通过。 |

## 数据、协议与依赖影响

旧显示默认值：

| 字段 | 缺字段默认值 |
| --- | --- |
| graphics_pipeline | false（显式 true 仍保留，但实际策略禁用） |
| display_wallpaper / display_themes | true / true |
| enable_font_smoothing / enable_desktop_composition | true / false |
| disable_full_window_drag / disable_menu_animations | true / true |
| disable_cursor_shadow / disable_cursor_blinking | false / false |

不改变 redb 表名、键、凭据加密、备份、同步格式或证书验证及认证流程。保留磁盘配置的字段名、已有显式值及未知字段；缺字段补默认值属于兼容读取，不做破坏性迁移。

IPC 为严格匹配的进程间契约：本次版本 11 拒绝旧/未来 Helper，不依赖 serde 容忍未知字段来允许混用。发布时应将应用、RDP Helper、VNC Helper 放在一起；若使用 NYATERM_RDP_HELPER / NYATERM_VNC_HELPER 覆盖路径，也必须更新其目标。

修改六个 crate：nyaterm-app、nyaterm-core、nyaterm-desktop、nyaterm-remote-desktop、nyaterm-rdp-helper、nyaterm-vnc-helper。桌面改动包含 editor、session、shell、platform、overlay、测试及 en/fr/ja/ko/zh-CN/zh-TW 文案；未修改 transport、store、终端解码或第三方源码。

没有增加生产依赖。app 删除直接 tracing-log 依赖，直接 log 依赖改为仅测试使用；自动桥接仍由原有 tracing-subscriber 提供。Cargo.lock 仅移除 app 的直接 tracing-log 依赖关系，没有升级任何包。

## 自动验证结果

平台：本机 Windows x86_64 MSVC，Rust/Cargo 1.99.0。

| 实际命令 | 结果 | 本地日志（target 下，未跟踪） |
| --- | --- | --- |
| `cargo check -p nyaterm-app -p nyaterm-rdp-helper -p nyaterm-vnc-helper` | 通过 | pr999-check.log |
| `cargo test -p nyaterm-core -p nyaterm-remote-desktop -p nyaterm-rdp-helper -p nyaterm-vnc-helper` | 通过 | pr999-domain-tests.log |
| `cargo fmt --all -- --check` | 最终代码通过，exit 0 | pr999-fmt-final.log |
| `cargo test --workspace` | 最终代码通过，exit 0；总数见下 | pr999-tests-verified.log |
| `cargo check --workspace` | 最终代码通过，exit 0 | pr999-check-verified.log |
| `cargo clippy --workspace --all-targets` | 最终代码通过，exit 0，无 warning | pr999-clippy-verified.log |
| `cargo build -p nyaterm-rdp-helper -p nyaterm-vnc-helper` | 通过，exit 0 | pr999-helpers-build.log |
| `cargo build -p nyaterm-app --bin nyaterm` | 通过，exit 0 | pr999-app-build.log；退出码另存 pr999-app-build-exit.txt |
| `git -c core.autocrlf=false diff --check` | 通过 | 无输出 |

最终 workspace 汇总：**4323 passed，0 failed，29 ignored，46 个 suite 汇总**。ignored 项目未计入已验证；它们保持项目原有状态。最终退出码同时记录在 `target/pr999-validation.json`。

有意义的行为覆盖：

- 独立 Windows 测试进程验证日志全局初始化、重复初始化报错、文件不可用降级；trace/debug/error 对 log 和 tracing 的控制台与 JSONL 输出各出现一次，并受 RUST_LOG 一致过滤。
- 旧配置缺字段读取、新数据往返、未知字段保存、编辑器保存并重开显式选项、core 到 IPC 默认值一致；逐位 PerformanceFlags 映射和默认 `0x86`；RDPEGFX 两种请求值均执行禁用策略。
- RDP/VNC 双向版本不匹配、实际版本响应、握手顺序、心跳、普通断开、Helper 崩溃与挂起回收；启动的六个 worker 阶段失败均回滚。
- stderr 8 MiB 超长行、无换行、无效 UTF-8、私密文本丢弃、真实 pipe 大量输出；正常/崩溃/挂起子进程及 stderr worker 启动失败回收。
- GPUI 窄窗口 400×300 / 320×220、拖动边界、缩放后重新限制、短内容自适应、滚动、关闭重开重置、文本选择与 Ctrl-C、背景指针和滚动隔离；相关既有焦点测试随 workspace 执行。

验证过程中的失败及处理：

1. 初次编译发现 fmt builder 的固有 try_init 返回的错误类型不能直接使用 anyhow Context；改为 `.finish().try_init()` 后通过。未隐藏初始化错误。
2. 首次 workspace 测试出现一个笔记弹窗测试失败：`features::notes::panel::tests::note_delete_dialog_controls_remain_clickable_after_context_menu`，`notes/panel.rs:1000`，断言 `!window.has_active_nya_dialog(cx)`；该轮 desktop 为 2211 passed、1 failed、8 ignored，日志 `pr999-workspace-tests.log`。未修改或弱化该测试。随后完整 `cargo test --workspace --no-fail-fast` 以及最终完整 `cargo test --workspace` 均通过，未复现。根因尚未确定，不能称为已经证明的历史问题。
3. main 基线对照尝试因另一项本地构建持有锁而取消；未获得干净 main 的成功对照结果，也未干扰其构建。
4. 初次 Clippy 的两处 collapsible-if 警告已通过 let-chain 修正；最终无警告。

## Windows 冒烟检查

使用 computer-use 技能，通过 Windows 原生窗口 API 检查实际构建的 debug 应用：

- 将应用及两个 Helper 复制到 `target/pr999-portable-smoke`，放置 `nyaterm-portable` 标记；配置、数据库、缓存及日志均位于该目录的 data 下，不使用日常用户配置。
- 成功启动并看到主窗口正常渲染；通过 Ctrl-Alt-N 打开临时连接对话框，Escape 关闭；没有输入凭据或发起远程连接。
- Alt-F4 正常退出，随后窗口枚举和精确进程路径查询均确认测试副本已退出。退出日志包含 persistence finished、workspace workers stopped；原工作区另一个运行中的 NyaTerm 实例未操作。
- 实际启动生成有效 JSONL。日志出现一条 GPUI ERROR：无法加载 `amd_ags_x64.dll`，因而获取 AMD 驱动版本失败。核对固定 GPUI 的 `directx_renderer.rs:842`，该路径记录错误后使用 `Unknown Driver`，仍返回所选设备的 GpuSpecs 和软件标志；应用渲染与退出未受阻。没有为此增加 DLL、修改 fork 或把该诊断错误隐藏。

本项验证启动、主窗口渲染、普通模态打开关闭和退出；会话信息交互由前述 GPUI 测试覆盖，不等同于真实 RDP/VNC 连接、系统剪贴板或硬件/WARP 切换验证。隔离运行的 JSONL 保留在 `target/pr999-portable-smoke/data/logs`，未跟踪。

## 未验证项目与剩余风险

- 没有可用的已认证 Windows RDP/VNC 服务端，未完成真实连接、断开重连、管理会话分配、服务端安全协商、长时间显示和剪贴板传输测试。自动生命周期/模拟服务端测试不替代这些验证。
- Hardware / WARP 分类已有依据实际 GPUI 设备标志的单元测试，但未分别在真实硬件和 WARP 环境切换实测；本地 Windows 远程会话检测也未在会话切换中实测。
- 没有 Linux/macOS 环境做编译和实机操作；未提供规格的 GPUI 平台显示 Unknown，非 Windows 远程会话状态为不可用。
- 本机隔离副本缺少 AMD AGS DLL，驱动版本诊断降级为 Unknown Driver；这是固定 GPUI 中的非致命诊断路径。若产品需要显示完整 AMD 驱动版本，应另行审查其运行库分发与 fallback，本次不扩展依赖修改范围。
- RDPEGFX 继续不可用。固定 IronRDP 具备 Progressive 解码，尚无实机证据足以证明全部路径可靠或断定 PR 声称的特定图块缺陷。后续应使用脱敏最小复现验证后再单独开放该功能。
- stderr 摘要有意不包含第三方原文；排查协议实现内部问题应使用隔离测试和 typed IPC，不恢复可能携带敏感数据的全文转发。
- 笔记弹窗测试曾发生一次未解释失败，后续两次全量通过；建议 CI 继续关注该测试的稳定性。

## 合并建议

**远程 PR #999 原始 HEAD 不建议直接合并**，上述 P0 日志冲突、无效开关和 IPC 兼容问题在原始提交中存在。

本地修复版已通过 Windows 全 workspace 自动验证，可进入修复复审。建议在配套构建三个可执行文件后，补做真实 RDP/VNC 服务端及硬件/WARP 冒烟检查，再决定合并；不得因本报告的自动验证结果宣称这些场景已经通过。
