# 窗口装饰与标题栏交互

## 所有权与平台边界

`nyaterm-desktop::window_chrome` 根据 GPUI 窗口的实际装饰、按钮能力、
可缩放/可最小化标记、最大化、全屏及 Sheet 类型派生展示策略。
这些信息只在渲染时读取，不另存可修改的窗口装饰状态。

`nyaterm-ui::window_chrome` 提供按钮和标题栏拖动区域；desktop 回调继续负责
关闭确认、会话处理、主窗口拖动活动标记及现有 Windows 子窗口恢复修正。
主窗口、锁屏标题栏和公共子窗口标题栏使用同一策略。

Linux 主窗口及 `ChildWindowSpec` 子窗口请求 CSD，子窗口同时提供实际标题。
如果平台实际返回 SSD，顶部应用内容保留，自绘窗口按钮隐藏。按钮还受窗口
能力和窗口标记约束；全屏隐藏常规按钮，最大化时绘制恢复图标。
Windows 主窗口保留原生 caption hit testing，子窗口保留显式点击回调。
主窗口的最大化/恢复完整交给 Windows 原生处理；GPUI 的 Windows `zoom()`
仅执行最大化，不能用它替代原生恢复。其他按钮消费点击后由应用回调执行一次，
保留最小化到托盘和关闭确认；子窗口继续使用既有显式恢复修正。
macOS 保留原生按钮、拖动和双击偏好，Sheet 不添加按钮留白或拖动区域。

## Linux 拖动

每个标题区域使用所属窗口的 GPUI keyed state。左键按下记录位置，移动距离
超过 4 个逻辑像素后只派发一次系统移动操作。按下后使用窗口级移动监听，
因此快速移出标题区域仍能启动拖动；监听通过无绘制、无命中区域的 canvas
在 paint 阶段注册，不增加装饰外框。

释放、区域外释放、再次在区域外按下、丢失左键状态及平台 pointer cancel
清除待拖动状态。未开始拖动的双击在释放时只派发一次最大化/恢复，且需要
窗口可缩放、平台支持最大化并处于非全屏状态。按钮、菜单及输入控件消费
按下事件，避免标题区域同时处理该次交互。

## 外框与几何

`nya_root()` 继续是唯一外框入口，加载、运行和关闭阶段都复用 gpui-kit Root
插件提供的 CSD 边框、阴影、inset 和边缘/四角缩放。窗口位置继续通过
`inner_window_bounds()` 保存，不把阴影尺寸写入恢复数据。

本次保留按钮顺序、尺寸和方角风格，不增加 SSD/布局配置，不改变持久化格式
或依赖 revision，不修改 `temp/vendor/`。

## 验证记录（2026-10-27）

- 纯策略测试覆盖实际 CSD/SSD、窗口标记、平台能力、最大化、全屏和 macOS Sheet。
- GPUI/状态测试覆盖 4px 阈值、只启动一次、跨区域移动、释放/取消、双击、
  多窗口隔离、按钮单次回调、输入焦点和菜单打开。
- `cargo check --workspace -j 1`：通过。
- `cargo test -p nyaterm-ui window_chrome --lib -j 1`：最终 9 项通过，包含 Windows
  原生最大化事件放行与应用按钮回调单次派发。
- `cargo fmt --all -- --check`：通过。
- 最终 `cargo test --workspace -j 1 -- --test-threads=1`：4355 项通过，
  30 项按既有配置忽略，无失败。
- 最终 `cargo check --workspace -j 1`、`cargo clippy --workspace --all-targets -j 1`：通过。
- 追加 UI/desktop 并行回归中代理网络测试的非阻塞 socket 曾报 `WouldBlock`，
  最终完整串行测试中该测试通过。
- 最终 `cargo build -p nyaterm-app --bin nyaterm -j 1`：通过。
- Windows 实机（独立 portable 数据目录）：主窗口最小化/重新激活、关闭、
  按钮最大化/恢复、拖动、标题栏双击均通过。设置子窗口拖动、按钮最大化/恢复、
  标题栏双击最大化/恢复、关闭按钮、Ctrl+W 关闭与回到主窗口均通过。
  模态设置窗口正确隐藏最小化按钮；主窗口重启后位置和尺寸保持一致。
- 首次默认并发 workspace test 构建发生内存分配失败，改用 `-j 1` 重试。
- Ubuntu WSLg：现有 Rust/Cargo 1.75 不支持 workspace resolver 3，Linux 构建
  在解析 manifest 时失败，尚未进入运行验收。
- GNOME/KDE 原生会话及 macOS：未验证。

Linux 实机待核对：边缘和四角缩放、光标、不可缩放窗口、模态与多窗口；
仅一层 CSD 外框；多次重启恢复尺寸不累积阴影。Windows 尚未逐一验收全部
子窗口类型；本次实机子窗口样本为设置窗口。
