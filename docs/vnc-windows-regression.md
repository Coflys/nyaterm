# Windows 11 VNC 分屏回归（#874）

本次改动限制前端待绘制队列，处理隐藏面板及 WebGL 恢复。它不限制 Tauri IPC
在途数据，也不能证明原闪退已修复。没有受控的 VNC/SSH 远端和崩溃证据时，
下列实机项目应保留为“待验证”，Issue #874 保持 Open。

2026-10-19 的自动化结果：7 组 Vitest 共 57 项通过，2 项浏览器集成测试通过，
共享核心完整帧恢复测试通过；`pnpm lint` 通过（另有既存的
`CommandSuggestions.tsx` 依赖警告），`pnpm build:web` 与 5 种语言键检查通过。
未执行真实 VNC/SSH 混合负载、部署服务端会话保留检查或 WebView2 压力回归，
没有采集到此次闪退的事件记录或转储。

## 自动化

```powershell
pnpm exec vitest run src/lib/remoteDesktopFrame.test.ts src/lib/remoteDesktopFrameQueue.test.ts src/lib/vncFrameSubscription.test.ts src/components/remote-desktop/RemoteDesktopSurface.test.tsx src/components/remote-desktop/renderer.test.ts src/components/vnc/VncPaneHost.test.tsx
pnpm exec playwright test --config e2e/vnc.config.mjs
cargo test --manifest-path src-tauri/crates/nyaterm-core/Cargo.toml bounded_subscriber_recovers_dropped_patches_with_a_complete_frame --locked
pnpm lint
pnpm build:web
pnpm i18n:keys
```

浏览器测试使用受控的 WebSocket 服务器流，验证生产 React/传输适配器和实际
Canvas2D 像素，以及 WebGL 丢失后的画布替换；它不覆盖真实 Web 服务端清理或
Windows WebView2。另在部署的 Web 版本中隐藏 VNC 标签超过 30 秒，再显示，
确认原会话仍存在、画面完整，其他可见分屏持续更新。

## 实机矩阵（待验证）

使用原版 v1.3.0 和本次构建，在原报告的两台 Windows 11 机器分别执行。
远端固定为 1080p，并增加 4K 场景；使用可重复的动态画面，不能只观察静止桌面。

| VNC / SSH                   | 布局与操作                                           | 每个构建的最低时长       |
| --------------------------- | ---------------------------------------------------- | ------------------------ |
| 2 / 3～4，含 X11 Forwarding | 不分屏，对照                                         | 60 分钟                  |
| 2 / 3～4                    | 水平、垂直分屏，拖动分隔条                           | 各 60 分钟               |
| 4 / 3～4                    | 水平、垂直混合分屏                                   | 60 分钟                  |
| 4 / 3～4                    | 切换标签、聚焦单面板、移动分屏、断开重连、关闭后重开 | 60 分钟，至少 100 次操作 |

在同负载下另运行禁用 GPU 的对照。完全退出所有 NyaTerm 进程后，在临时
PowerShell 进程中追加参数启动同一个构建；测试后退出该 PowerShell，勿写入
持久环境变量或设置：

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = ($env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS + ' --disable-gpu').Trim()
& 'C:\path\to\nyaterm.exe'
```

确认任务管理器中 WebView2 的启动命令行包含该参数。禁用 GPU 有效与否不能
单独用于判断主程序、渲染进程或驱动哪一层存在缺陷。

## 证据与验收

- 每轮记录构建提交、Windows 版本、GPU/驱动、WebView2 版本、远端分辨率及操作时间。
- 用任务管理器或性能监视器每 10 秒记录 NyaTerm 与所属 WebView2 子进程的
  Private Bytes、Working Set、进程 ID 和 GPU 内存。比较前 10 分钟预热与后续曲线，
  反复执行相同操作后不得持续增长；不能把正常缓存高水位直接判为泄漏。
- 导出应用诊断日志，检查 `vnc.render_recovery` 中的 `peakBytes`/`peakCount`、
  `vnc.frame_recovery_started`、`vnc.frame_recovered`、`vnc.frame_recovery_failed`。
  队列最多 128 个补丁；字节预算为 `max(64 MiB, 完整 RGBA + 44)`，最高 128 MiB。
  正常状态不逐帧写日志；恢复与峰值日志限流为每组件/会话每 5 秒一次。
- 闪退时从“事件查看器 → Windows 日志 → 应用程序”导出对应时间的错误记录，
  保留故障应用程序/进程、故障模块、异常代码、版本及故障偏移。记录是主窗口
  退出、画面空白还是整个进程树退出，并保留可用的 Windows Error Reporting 转储。
- 验收要求：无意外退出、无永久黑屏/花屏，完整帧恢复后像素正确；隐藏时不绘制，
  可见但未聚焦的 VNC 持续更新；断开重连后输入、画面和尺寸正常。

结果记录使用以下字段，失败场景附日志与转储路径：

| 构建 / 机器 | 场景 / 分辨率 | GPU 模式    | 时长 / 操作数 | 内存趋势 | 退出 / 画面异常 | 证据   | 结论   |
| ----------- | ------------- | ----------- | ------------- | -------- | --------------- | ------ | ------ |
| 待填写      | 待填写        | 默认 / 禁用 | 待填写        | 待填写   | 待填写          | 待填写 | 待验证 |

自动化通过后仍需完成实机矩阵与原报告环境复测，再评估是否关闭 Issue。
