# 2026-10-27 IronRDP 同步与 NyaTerm 适配

## 固定版本

- NyaTerm 基线：`59d1d5427e6c8316ff83c31e134759f53de501bf`。
- IronRDP 上游实际默认分支是 `master`，固定到 `3ca12ee63687980ace30cd6ed59c2ab4f57cf157`，比原基线新增 90 个提交。
- `nyakang/IronRDP:nyaterm` 从 `1bc71d95c6414d5603792cb6cf8913e903205a33` 更新到 `b10cd243bc9442a07fcd776568a5fdaf1c64492f`。
- 原 fork HEAD 保留于远端 `archive/nyaterm-20261020`，已有 NyaTerm revision 仍可获取。
- NyaTerm 的九个 IronRDP patch 和 lockfile 中的 27 个 IronRDP 包统一使用新 revision；没有混用 registry 类型或刷新无关包。现有 reqwest 因上游 feature 合并增加 `rustls-native-certs` 依赖。
- SSPI 继续固定到 `d2b04e63ca9a72f2a70b456186e2472b27f0d45d`。

## 兼容调整

| NyaTerm 直接依赖 | 原版本 | 新版本 |
| --- | --- | --- |
| ironrdp-input | 0.7 | 0.8 |
| ironrdp-cliprdr | 0.7 | 0.8 |
| ironrdp-core | 0.2 | 0.3 |
| ironrdp-pdu | 0.9 | 0.10 |
| ironrdp-tls | 0.2 | 0.3 |
| ironrdp-svc（测试依赖） | 0.8 | 0.9 |

上游新增 Windows 平台 H.264 解码器，并可能据此默认启用 RDPEGFX。Helper 显式调用 `with_graphics_pipeline(false)`，保留原位图路径。上游已经包含 Progressive 按 surface 保存 tile 状态的修复（#2117），但本次没有开放图形管线。

保留 strict TLS 验证、证书决策回调、NLA/密码组合策略、独立 CLIPRDR 后端、完整帧请求及 smart-card 认证排除。配置、凭据、备份、同步数据和 Helper IPC 版本均不变。

上游新增服务器重定向。fork 让应用提供的证书回调优先于服务器携带的目标证书提示，避免绕过应用拒绝。NyaTerm 只为原配置 endpoint 创建证书例外提示；其他 endpoint 的无效证书直接拒绝，避免按原主机名称展示或保存信任。受平台证书库正常验证的普通直连重定向仍使用上游流程；没有新增重定向管理界面。

认证 loopback relay 绑定原始目标，不能转发到服务器选择的新目标。fork 在重定向修改目标或凭据之前明确报错，继续保留认证前导、失败时不回退直连、原 TLS/CredSSP 身份及禁用 UDP 的策略。

新增网络统计等信息事件继续由现有忽略分支处理，没有把未经展示适配的数据作为 UI 实测结果。

## 验证

IronRDP 在 Windows / 固定 Rust 1.94.1 下通过 rustls、rustls+clipboard、rustls+clipboard+udp 检查，以及 connector、acceptor、VMConnect 检查。29 项 client library 测试通过，覆盖完整帧、原目标身份、relay 认证顺序、证书拒绝优先级和 relay 重定向拒绝。Connector 的 test 命令没有启用的测试目标，不能视为实际协议测试。

Client Clippy 和 workspace rustfmt 检查通过；保留上游既有 MSRV 配置不一致和 stable rustfmt 不支持 nightly 选项的警告。两个新增重定向回归测试均在修复前复现失败。

NyaTerm 的图形管线回归测试同样在修复前复现失败。新增配置测试覆盖 NLA/密码组合、strict TLS、证书回调、禁用原生剪贴板和 endpoint 信任范围。

NyaTerm 在 Windows 上使用正式 git pins 执行以下命令，均退出 0：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p nyaterm-rdp-helper -p nyaterm-vnc-helper -p nyaterm-remote-desktop --locked` | 140 项通过，包括两个 Helper 的握手、断开、崩溃/挂起回收及脚本对端集成测试 |
| `cargo check --workspace --locked` | 通过 |
| `cargo test --workspace --locked` | 4,360 项通过，30 项 ignored，0 失败 |
| `cargo clippy --workspace --all-targets --locked` | 通过，无警告 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo build -p nyaterm-rdp-helper -p nyaterm-vnc-helper --locked` | 通过，两个可执行文件位于同一 target/debug 目录 |

## 未验证范围与评审建议

没有可用于本次测试的真实 RDP/VNC 服务端，未验证真实 NLA、剪贴板文件传输、连接代理/重定向、硬件或 WARP 图形场景；没有本轮交互式 GUI 验证及 Linux/macOS 本地执行。脚本协议对端和 Helper 进程测试不能替代这些结果。

IronRDP fork 的 GitHub Actions 已被平台禁用，手动 workflow dispatch 返回 HTTP 422，无法取得本次 revision 的 Linux CI 结果。完整上游 workspace、FFI、web 和可选媒体功能测试不在本次验证范围。

建议保持适配 PR 为 draft，待跨平台 CI 和真实 RDP 会话验证后再合并。PR #999 的配置、IPC 和诊断修复属于另一条未合并的改动，本次适配基于 main，未将其一并引入。
