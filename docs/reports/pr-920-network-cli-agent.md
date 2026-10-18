# PR #920 网络设备 CLI Agent 执行改进

日期：2026-10-18。原 PR HEAD 为 `29c7e5adaa02678a30a9846d232f63eac23576d8`；后续整合 `migration/gpui` 的 `607f4d44b` 共享终端执行架构，保留原作者提交和增强提交历史。

## 问题与实际调用链

原 PR 避免在网络设备上发送 POSIX Marker，但未复用 NetworkDevice 配置，且会将 `root#`、`bash-5.2#` 误判为设备。原始 SendOnly 使用有界 UI Tail 的长度及 900ms 静止判断完成，无法可靠处理慢速输出、分页、交互、截断和 UI 渲染降级；后台及 MCP 另有额外 SSH Exec 路径。

目标分支现已将传统 Agent、Native Agent、集成 CLI 和外部 MCP 统一到 `TerminalExecutionState`，因此最终实现直接扩展该权威状态，不保留旧 Agent 专属观察器或重复的作用域捕获状态。

当前调用链：

1. 命令卡片经过既有风险评估、审批和目标校验，调用 `begin_ai_agent_execution`；Native Agent / MCP 经过 capability 权限、作用域和审批，调用 `dispatch_shared_terminal_execute`。
2. 全部进入 `start_terminal_execution`，检查会话、显式 Profile、后台策略及自动执行目标占用。
3. 正常 Shell 继续使用原 Marker 包装和 Terminal 捕获；SendOnly 直接发送原始命令，并将 RawCliCapture 放在同一 TerminalExecutionTask 内。
4. 前台发送前使用既有 transport OutputBoundary；SessionEventBridge 顺序转交边界与输出，UI 消费边界后才启用捕获。边界前排队数据不会进入新命令结果。
5. Session 输出经过原有有状态解码器，再进入共享 `process_output`。Raw 捕获按事件增量追加，与 UI Tail 长度和 render_degraded 无关。
6. 既有执行时钟轮询 Prompt 和整体截止时间。完成、失败、取消、断线、输出丢失及 owner 清理统一移除任务并回复一次。Native/传统 Agent 对不完整结果停止自动继续；外部 MCP 收到错误或明确的不完整标记。

## 修改范围

| 文件 | 最终实现 |
| --- | --- |
| `nyaterm-core/src/network_cli_prompt.rs` | 显式 Profile 优先；Auto 复用 NetworkDevice，再考虑轻量被动 Prompt；排除 Shell 与交互提示；有界 ANSI/CR 末行观察；后台执行策略。 |
| `nyaterm-core/src/raw_cli_capture.rs` | 每命令独立的输出、末行、活动时间与捕获上限，区分 Prompt、交互、未确认静止、超时与截断。 |
| `nyaterm-core/src/lib.rs` | 声明上述模块。 |
| `nyaterm-desktop/src/features/ai/ai_agent_runtime.rs` | 读取现有 SSH launch config 的 NetworkDevice；统一后台限制；传统 Agent 遇失败或不完整输出停止自动继续。 |
| `nyaterm-desktop/src/features/ai/harness_runtime.rs` | Native Agent 对交互等待、不完整输出、超时、取消和断线阻止自动继续。 |
| `nyaterm-desktop/src/features/capability_runtime/terminal_execution.rs` | 将 Raw 捕获接入共享任务、输出边界、事件路由、轮询与收尾，供所有调用来源复用。 |
| `nyaterm-desktop/src/features/capability_runtime/terminal_execution/tests.rs` | 原有 Marker/后台/隐藏终端覆盖；新增 Raw 边界、事件、策略、丢失、取消、断线与连续命令回归。 |
| `docs/ai-terminal-execution.md` | 更新 SendOnly 执行语义。 |

不改变持久化配置、连接模型、凭据或 MCP 返回结构，不新增依赖或 Arc/Mutex。正常 Shell Marker 构建/解析与上游控制序列、draining/retired 处理保持一致。

## 执行语义

- 显式 Posix、Powershell、Cmd、SendOnly、Disabled 优先。Auto + NetworkDevice 选择 SendOnly；普通未知 SSH Auto 保持 Posix。已知设备不因显式 Posix 而获得额外 SSH Exec 通道；前台仍尊重该选择。
- Raw 命令返回同设备 Prompt 并稳定 400ms 后结束观察；这只说明回到输入状态，成功与退出码仍未知，不返回伪造的 exit 0。常见 `<HOST> → [HOST-view]`、`Router> → Router(config)#` 可比较。
- 已有初始 Prompt 未返回时，回显后即使静止十秒也不算完成。缺少可靠初始 Prompt 时，八秒静止返回未确认错误；完全无输出等待整体超时。
- 分页、确认、密码输入、截断及超时停止内置 Agent 自动执行，不自动翻页或确认。外部 MCP 必须按错误及 timedOut/sourceTruncated 标记处理结果。
- Raw 捕获上限为 1 MiB，独立于 128 KiB UI Tail；超过预算明确不完整。截止时间不随持续输出续期。错误时保留前缀供终端结果展示；输出丢失沿用 sourceTruncated 标记。
- 取消、断线、完成和 owner 清理复用共享生命周期；同目标自动执行及用户输入门禁沿用上游规则。后台 raw/已知设备/Disabled 明确拒绝，需关闭 Background Execution 后使用现有 PTY。

## 与 Netcatty 的比较

在忽略目录 `temp/Netcatty` 只读核对 `binaricat/Netcatty` 的 `f9230e76c36a030967b1091ab181834e93e33325`，包括任务指定的五个文件。

共同点是显式网络配置、原始 PTY 命令、独立输出捕获、输出活动/整体超时/取消以及避免不兼容的额外 Exec。这里复用 Rust 既有共享执行任务与 SessionEventBridge，没有复制 Electron/TypeScript 架构。

Netcatty 的三秒静止、首两个块六秒静止、无响应成功回退及达到输出上限后不再续期，均不能证明命令完成。本实现使用 Prompt 比较，未知、分页、截断和无输出不通过静止时间伪装为成功。

SSH Transport 暂无低成本传递到 metadata 的服务器标识字段；没有为 Banner 重构 Transport，也没有发送探测命令。识别目标是执行环境，厂商正则无法消除 `HOST#` 与自定义 Shell 的歧义，因此没有新增厂商规则引擎或 Regex UI。

## 验证

首次基于原 PR 的实现通过 Windows Core 452 项及 Desktop 159 项针对性测试。此后目标分支已重构；这些历史数字不作为整合后版本的验证结果。

整合后的 Windows 本地验证：

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过。 |
| `python scripts/ci/check_architecture.py` | 通过。 |
| `cargo check -p nyaterm-core -p nyaterm-desktop --locked` | 通过，无编译警告。 |
| `cargo clippy -p nyaterm-core -p nyaterm-desktop --all-targets --locked -- -D warnings` | 通过。 |
| `cargo test -p nyaterm-core --locked` | 452 单元 + 23 集成，共 475 通过；1 个已有手动基准忽略。 |
| `cargo test -p nyaterm-terminal agent_capture:: --locked` | 18 通过，含 CMD/PowerShell 实际执行。 |
| `cargo test -p nyaterm-desktop --lib features::capability_runtime:: --locked` | 20 通过，含隐藏终端 CMD/PowerShell 及新增 Raw 回归。 |
| `cargo test -p nyaterm-desktop --lib features::ai:: --locked` | 104 通过；1 个已有性能基准忽略。 |
| `cargo test -p nyaterm-desktop --lib features::shell::event_pump:: --locked` | 32 通过。 |
| `cargo test -p nyaterm-desktop --lib models::session_event_bridge:: --locked` | 18 通过。 |

Desktop 针对性测试合计 174 项通过。首轮新增测试中的 CR 清理断言及 GPUI 真实线程唤醒配置已修正后重跑通过。没有运行整个 workspace 的最重构建；远程 CI 仍需独立核对。

## 边界与合并条件

- 裸 `HOST#` 仍可能与自定义 Shell 混淆，应优先配置 NetworkDevice 或显式 Shell Profile。
- Raw PTY 无事务 Marker。OutputBoundary 排除边界前的已排队输出，但设备异步日志、边界后到达的旧输出，以及恰好等于 Prompt 的命令输出仍可能影响观察。
- 复杂光标移动、自定义/改名 Prompt、部分嵌套视图和缺少初始 Prompt 会保守失败；末行观察不是完整终端仿真。
- 取消/超时停止本地观察及自动执行，不保证远端命令停止，不盲目注入 Ctrl+C。恢复前应人工检查终端。
- 事件捕获复用 UI 旁路，每事件增量处理、内存有界；没有把整个捕获重构到后台线程。
- 本地结果不等于合并就绪。必须完成新提交的远程 CI、独立代码审查和真实设备验收，特别是 macOS 原报告、慢速输出、分页、编码及 Prompt/视图变化。未执行增强版的硬件或跨平台人工验证。
- Issue #910 保持 Open；PR #920 在验证完成前不合并。目标为非默认分支，合并后应按验收/发布策略另行处理 Issue 关闭。
