use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Bounds, Context, IntoElement, ObjectFit, ParentElement as _, Pixels, RenderImage, Styled as _,
    StyledImage as _, canvas, div, img, rgb,
};
use nyaterm_remote_desktop::{RdpBackend, RdpError, RdpErrorKind, RemoteDesktopViewState};
use nyaterm_ui::NyaDialogWindowExt as _;
use raw_window_handle::RawWindowHandle;

use crate::features::NyaTermApp;
use crate::platform::{
    NativeRdpConnectionEvent, NativeRdpControl, NativeRdpDisconnectInfo, NativeRdpRect,
    NativeRdpSessionDetails, NativeRdpSnapshot,
};

impl NyaTermApp {
    pub(super) fn native_rdp_view(
        &mut self,
        session_id: String,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let app = cx.entity();
        let view_session_id = session_id.clone();
        let overlay_image = self
            .remote_desktop
            .sessions
            .get(&session_id)
            .and_then(|session| session.native_overlay_image.clone());
        let mut view = div().size_full().bg(rgb(0x000000));
        if let Some(image) = overlay_image {
            // The still frame must not take part in layout: it is only a
            // backdrop for the native canvas, and in flow it pushes the canvas
            // (and with it the popup window) one panel height down.
            view = view.child(
                img(image)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            );
        }
        view.child(
            canvas(
                move |bounds, window, cx| {
                    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window)
                    else {
                        return bounds;
                    };
                    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                        return bounds;
                    };
                    let parent = handle.hwnd.get();
                    let scale = window.scale_factor();
                    let rect = native_phys_rect(bounds, scale);
                    let dialog_open = window.has_active_nya_dialog(cx);
                    let app = app.clone();
                    let session_id = view_session_id.clone();
                    window.defer(cx, move |window, cx| {
                        app.update(cx, |this, cx| {
                            let overlay = this.native_rdp_overlay_state(dialog_open, cx);
                            this.ensure_native_rdp(&session_id, rect, parent, overlay, window, cx);
                        });
                    });
                    bounds
                },
                |_bounds, _image_bounds, _window, _cx| {},
            )
            .size_full(),
        )
        .into_any_element()
    }

    fn ensure_native_rdp(
        &mut self,
        session_id: &str,
        rect: NativeRdpRect,
        parent: isize,
        overlay: NativeRdpOverlayState,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.remote_desktop.sessions.get_mut(session_id) else {
            return;
        };
        if let Some(native) = session.native_rdp.as_mut() {
            let was_shown = native.is_shown();
            native.set_bounds(rect);
            let plan = native_host_plan(
                true,
                overlay.obscured,
                overlay.snapshot,
                was_shown,
                session.native_overlay_image.is_some(),
            );
            let mut show_host = plan.show_host;
            match plan.snapshot {
                NativeSnapshotAction::Clear => {
                    if let Some(image) = session.native_overlay_image.take() {
                        // The atlas keeps the full-panel texture until it is
                        // dropped, so every menu used to cost one more of them.
                        let _ = window.drop_image(image);
                        tracing::info!(session_id, "native RDP still frame released");
                    }
                }
                NativeSnapshotAction::Capture => {
                    session.native_overlay_image = native
                        .capture_snapshot()
                        .as_ref()
                        .and_then(native_snapshot_image);
                    if session.native_overlay_image.is_none() {
                        // No still frame can stand in for the popup, so give the
                        // panel to the menu instead of covering it.
                        tracing::warn!(session_id, "native RDP still frame capture failed");
                        show_host = false;
                    } else {
                        // The current frame was built before the still existed.
                        cx.notify();
                    }
                }
                NativeSnapshotAction::Keep => {}
            }
            native.set_visible(show_host);
            let revealed = !was_shown && native.is_shown();
            if revealed && self.shell.clear_resize_handle_hover() {
                cx.notify();
            }
            return;
        }
        // Painting this view is proof that its tab is laid out, so the control
        // may exist even when another split pane owns the active session.
        let Some(mut config) = session.native_rdp_config.take() else {
            return;
        };
        if config.display.mode == nyaterm_remote_desktop::RdpDisplayMode::FitWindow {
            let (width, height) = fit_native_desktop_dimensions(rect);
            config.display.width = width;
            config.display.height = height;
        }
        tracing::debug!(session_id, "creating Windows RDP control");
        let mut fallback = None;
        match NativeRdpControl::create(parent, rect, &config) {
            Ok(mut native) => {
                let overlay_image = if overlay.obscured && overlay.snapshot {
                    native
                        .capture_snapshot()
                        .as_ref()
                        .and_then(native_snapshot_image)
                } else {
                    None
                };
                native.set_visible(!overlay.obscured);
                if !overlay.obscured && self.shell.clear_resize_handle_hover() {
                    cx.notify();
                }
                session.native_overlay_image = overlay_image;
                session.native_rdp = Some(native);
                session.state = RemoteDesktopViewState::Connecting;
                session.error = None;
                tracing::info!(session_id, "Windows RDP control created");
                self.ensure_remote_desktop_periodic_clock(cx);
                let session_id = session_id.to_string();
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(1))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.start_native_rdp_connection(&session_id, cx);
                    });
                })
                .detach();
                cx.notify();
            }
            Err(message) => {
                if let Some(fallback_backend) = native_creation_fallback(config.backend) {
                    tracing::warn!(
                        session_id,
                        error = %message,
                        "Windows RDP control failed; falling back to the built-in runtime"
                    );
                    let mut fallback_config = config;
                    fallback_config.backend = fallback_backend;
                    fallback = Some(fallback_config);
                } else {
                    tracing::warn!(session_id, error = %message, "Windows RDP control failed");
                    session.state = RemoteDesktopViewState::Failed;
                    session.error =
                        Some(RdpError::new(RdpErrorKind::HelperMissing, message).into());
                    cx.notify();
                }
            }
        }
        if let Some(config) = fallback {
            self.launch_rdp_runtime(session_id, config);
            self.ensure_remote_desktop_periodic_clock(cx);
            cx.notify();
        }
    }

    fn start_native_rdp_connection(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.remote_desktop.sessions.get_mut(session_id) else {
            return;
        };
        let Some(native) = session.native_rdp.as_mut() else {
            return;
        };
        match native.start() {
            Ok(()) => {
                session.state = RemoteDesktopViewState::Connecting;
                session.error = None;
            }
            Err(message) => {
                tracing::warn!(session_id, error = %message, "Windows RDP connect failed");
                session.state = RemoteDesktopViewState::Failed;
                session.error = Some(RdpError::new(RdpErrorKind::HelperMissing, message).into());
                cx.notify();
            }
        }
    }

    /// Full-window layers that would be covered by the native popup.
    ///
    /// The popup lives above the GPUI window, so it has to disappear for every
    /// overlay that expects to paint over the workspace; otherwise the lock
    /// screen and quick switch would be partly hidden and the RDP surface would
    /// still take their input. Menus additionally keep a still frame behind
    /// them; the lock screen deliberately does not.
    pub(in crate::features) fn native_rdp_overlay_state(
        &mut self,
        dialog_open: bool,
        cx: &mut Context<Self>,
    ) -> NativeRdpOverlayState {
        let menu = self.native_rdp_menu_overlay_obscured(dialog_open, cx);
        let locked = self.security.screen_locked();
        NativeRdpOverlayState {
            obscured: locked || menu,
            snapshot: menu && !locked,
        }
    }

    fn native_rdp_menu_overlay_obscured(
        &mut self,
        dialog_open: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        dialog_open
            || self.quick_switch_open(cx)
            || self.commands.quick_editor_is_inline()
            || self.commands.quick_details().is_some()
            || self.commands.quick_variable_prompt().is_some()
            || self.session.dialog_tab_actions_target().is_some()
            || self.session.dialog_session_info_is_open()
            || self.session.dialog_color_picker_is_open()
            || self.shell.header_status_menu_is_open()
            || self.shell.open_tabs_menu_is_open()
            || self.shell.new_session_menu_is_open()
            || self.shell.activity_bar_context_menu().is_some()
            || self.connection_state.inline_editor_panel_draft().is_some()
            || self.pending_ai_full_access().is_some()
            || !self.mcp_pending_approval_requests().is_empty()
            || self.session.prompt_has_active_ssh_auth()
    }

    /// Live properties of the hosted MsTscAx control for the session-info view.
    pub(in crate::features) fn native_rdp_session_details(
        &self,
        session_id: &str,
    ) -> Option<NativeRdpSessionDetails> {
        self.remote_desktop
            .sessions
            .get(session_id)?
            .native_rdp
            .as_ref()
            .map(NativeRdpControl::session_details)
    }

    /// Workspace tab roots whose RDP view is actually laid out.
    ///
    /// `visible_terminal_session_ids` keeps the last active session in
    /// pane-focus mode, which is stale when the focused tab is Settings or an
    /// editor; use the focused tab itself there instead.
    pub(in crate::features) fn native_rdp_visible_tab_roots(&self) -> HashSet<String> {
        if self.shell.pane_focus_mode() {
            return self
                .active_workspace_tab()
                .filter(|key| crate::models::WorkspaceTabId::from_layout_key(key).is_session())
                .filter(|id| !self.shell.session_tab_is_hidden(id))
                .into_iter()
                .collect();
        }
        self.visible_terminal_session_ids()
            .into_iter()
            .filter(|id| !self.shell.session_tab_is_hidden(id))
            .map(str::to_string)
            .collect()
    }

    pub(super) fn drive_native_rdp_sessions(
        &mut self,
        window: &mut gpui::Window,
        visible_tab_roots: &HashSet<String>,
        overlay_obscured: bool,
        overlay_snapshot: bool,
    ) -> bool {
        let allowed: HashMap<String, bool> = self
            .remote_desktop
            .sessions
            .iter()
            .filter(|(_, session)| session.native_rdp.is_some())
            .map(|(session_id, session)| {
                let tab_visible =
                    visible_tab_roots.contains(&self.tab_root_for_session(session_id));
                (
                    session_id.clone(),
                    tab_visible && native_rdp_state_is_displayable(session.state),
                )
            })
            .collect();
        let mut dirty = false;
        let mut cursor_over_native = false;
        for (session_id, session) in &mut self.remote_desktop.sessions {
            let Some(native) = session.native_rdp.as_mut() else {
                continue;
            };
            let was_shown = native.is_shown();
            cursor_over_native |= was_shown && native.cursor_is_inside();
            let plan = native_host_plan(
                allowed.get(session_id).copied().unwrap_or(false),
                overlay_obscured,
                overlay_snapshot,
                was_shown,
                session.native_overlay_image.is_some(),
            );
            let mut show = plan.show_host;
            match plan.snapshot {
                NativeSnapshotAction::Clear => {
                    if let Some(image) = session.native_overlay_image.take() {
                        let _ = window.drop_image(image);
                        tracing::info!(session_id, "native RDP still frame released");
                    }
                }
                NativeSnapshotAction::Capture => {
                    session.native_overlay_image = native
                        .capture_snapshot()
                        .as_ref()
                        .and_then(native_snapshot_image);
                    if session.native_overlay_image.is_none() {
                        tracing::warn!(session_id, "native RDP still frame capture failed");
                        show = false;
                    } else {
                        // The still replaces the hidden popup, so the panel needs
                        // a repaint before the menu is readable.
                        dirty = true;
                    }
                }
                NativeSnapshotAction::Keep => {}
            }
            native.set_visible(show);
            dirty |= was_shown != native.is_shown();
            let Some(event) = native.poll_connection() else {
                continue;
            };
            match event {
                NativeRdpConnectionEvent::Connected => {
                    session.state = RemoteDesktopViewState::Connected;
                    session.error = None;
                    dirty = true;
                }
                NativeRdpConnectionEvent::Disconnected(info) => {
                    let message = native_disconnect_message(&info);
                    tracing::info!(
                        ?info,
                        %message,
                        "Windows RDP connection ended"
                    );
                    native.set_visible(false);
                    session.state = RemoteDesktopViewState::Failed;
                    session.error = Some(RdpError::new(RdpErrorKind::Session, message).into());
                    dirty = true;
                }
            }
        }
        // The native host is a separate top-level window: GPUI never sees the
        // pointer leave the resize handles it covers, so their dwell highlight
        // would stay lit.
        if cursor_over_native && self.shell.clear_resize_handle_hover() {
            dirty = true;
        }
        dirty
    }
}

/// What the still frame behind an in-window menu should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeSnapshotAction {
    /// Drop the still; the live popup paints the panel again.
    Clear,
    /// Take the still from the popup that is still on screen.
    Capture,
    /// Keep the still that is already there.
    Keep,
}

/// Whether the popup is on screen for this update, and what its still does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativeHostPlan {
    show_host: bool,
    snapshot: NativeSnapshotAction,
}

/// Plan one update of the popup and the still frame that replaces it.
///
/// The popup is a separate top-level window, so an in-window menu is only
/// readable while the popup is hidden. Hiding it without a replacement paints
/// a black panel, and clearing the replacement before the popup is back paints
/// a black panel the other way around. Both swaps therefore happen one update
/// late: the popup is only hidden once its still frame exists, and the still
/// frame is only dropped once the popup is back on screen, where it covers the
/// panel anyway.
fn native_host_plan(
    host_allowed: bool,
    overlay_obscured: bool,
    overlay_snapshot: bool,
    host_shown: bool,
    has_snapshot: bool,
) -> NativeHostPlan {
    if !host_allowed {
        return NativeHostPlan {
            show_host: false,
            snapshot: NativeSnapshotAction::Clear,
        };
    }
    if !overlay_obscured {
        return NativeHostPlan {
            show_host: true,
            snapshot: if host_shown {
                NativeSnapshotAction::Clear
            } else {
                NativeSnapshotAction::Keep
            },
        };
    }
    if !overlay_snapshot {
        // The lock screen must never keep a frame of the remote desktop.
        return NativeHostPlan {
            show_host: false,
            snapshot: NativeSnapshotAction::Clear,
        };
    }
    if has_snapshot {
        return NativeHostPlan {
            show_host: false,
            snapshot: NativeSnapshotAction::Keep,
        };
    }
    // The popup is the only source of a still frame: keep it on screen for
    // this update so the next one can hide it behind the captured frame.
    NativeHostPlan {
        show_host: host_shown,
        snapshot: if host_shown {
            NativeSnapshotAction::Capture
        } else {
            NativeSnapshotAction::Keep
        },
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::features) struct NativeRdpOverlayState {
    pub obscured: bool,
    /// A still frame should replace the hidden popup behind an in-window menu.
    pub snapshot: bool,
}

/// The capture is already BGRA, which is what GPUI's render atlas uploads, so
/// the channels stay put and only the alpha is forced opaque (`BitBlt` leaves
/// it undefined). Swapping red and blue here turns a blue desktop orange.
fn native_snapshot_bgra_opaque(snapshot: &NativeRdpSnapshot) -> Option<image::RgbaImage> {
    let expected = snapshot.width as usize * snapshot.height as usize * 4;
    if snapshot.bgra.len() != expected {
        return None;
    }
    let mut pixels = Vec::with_capacity(expected);
    for pixel in snapshot.bgra.as_chunks::<4>().0 {
        pixels.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 0xff]);
    }
    image::RgbaImage::from_raw(snapshot.width, snapshot.height, pixels)
}

fn native_snapshot_image(snapshot: &NativeRdpSnapshot) -> Option<Arc<RenderImage>> {
    let image = native_snapshot_bgra_opaque(snapshot)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
}

/// Once a native session is no longer connected the popup must give the panel
/// back to the GPUI error component; there is no framebuffer to fall back to.
fn native_rdp_state_is_displayable(state: RemoteDesktopViewState) -> bool {
    matches!(
        state,
        RemoteDesktopViewState::Connecting | RemoteDesktopViewState::Connected
    )
}

/// Only `Auto` retries with the built-in runtime; an explicit choice is a
/// decision the user should see fail instead of silently changing backend.
fn native_creation_fallback(backend: RdpBackend) -> Option<RdpBackend> {
    (backend == RdpBackend::Auto).then_some(RdpBackend::Builtin)
}

/// The base ActiveX control exposes the basic reason on some systems and the
/// extended reason on others; both are mapped here so the takeover case is
/// explained instead of leaving a white surface.
fn native_disconnect_message(info: &NativeRdpDisconnectInfo) -> String {
    if let Some(description) = info
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return description.to_string();
    }
    match info.extended_reason {
        Some(5) => "The remote session was taken over by another connection.".to_string(),
        Some(3 | 4) => "The remote session timed out.".to_string(),
        Some(12) => "The remote session was logged off.".to_string(),
        Some(7..=9) => "The server denied the remote connection.".to_string(),
        Some(0) | None => match info.basic_reason {
            Some(3) => {
                "The server ended the remote session; it may have been taken over by another connection."
                    .to_string()
            }
            Some(2) => "The remote session was ended by the remote user.".to_string(),
            Some(0) | None => "The remote session ended unexpectedly.".to_string(),
            Some(code) => format!("The remote session was disconnected (reason {code})."),
        },
        Some(code) => format!("The remote session ended (reason {code})."),
    }
}

fn native_phys_rect(bounds: Bounds<Pixels>, scale: f32) -> NativeRdpRect {
    NativeRdpRect {
        x: (bounds.origin.x.as_f32() * scale).round() as i32,
        y: (bounds.origin.y.as_f32() * scale).round() as i32,
        width: (bounds.size.width.as_f32() * scale).round() as i32,
        height: (bounds.size.height.as_f32() * scale).round() as i32,
    }
}

fn fit_native_desktop_dimensions(rect: NativeRdpRect) -> (u32, u32) {
    let clamp = |value: i32| (value.clamp(200, 8192) as u32) & !1;
    (clamp(rect.width), clamp(rect.height))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::TestAppContext;
    use gpui::{
        IntoElement, ObjectFit, ParentElement as _, RenderImage, Styled as _, StyledImage as _,
        canvas, div, img, rgb,
    };

    use super::{
        NativeSnapshotAction, fit_native_desktop_dimensions, native_creation_fallback,
        native_disconnect_message, native_host_plan, native_rdp_state_is_displayable,
        native_snapshot_bgra_opaque, native_snapshot_image,
    };
    use crate::features::test_support::{
        app_with_visible_local_session, session_focus_test_window,
    };
    use crate::platform::{NativeRdpDisconnectInfo, NativeRdpRect, NativeRdpSnapshot};
    use crate::test_support::{TestAppCleanup, TestConfigDir};
    use nyaterm_remote_desktop::{RdpBackend, RemoteDesktopViewState};

    #[test]
    fn pane_focus_mode_drops_the_stale_session_when_settings_takes_the_tab() {
        let root = TestConfigDir::new("nyaterm-native-pane-focus");
        let mut cx = TestAppContext::single();
        let _cleanup = TestAppCleanup::new(&cx);
        let app = app_with_visible_local_session(&mut cx, root.path(), "local");
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                app.shell.set_pane_focus_mode(true);
                assert!(app.native_rdp_visible_tab_roots().contains("local"));
                app.open_page(crate::models::NavItem::Settings, cx);
                assert!(
                    !app.native_rdp_visible_tab_roots().contains("local"),
                    "the settings tab is the visible target, not the stale session"
                );
                app.select_session("local".to_string(), cx);
                assert!(app.native_rdp_visible_tab_roots().contains("local"));
            });
        });
    }

    #[test]
    fn tab_actions_menu_hides_the_native_host() {
        let root = TestConfigDir::new("nyaterm-native-tab-actions");
        let mut cx = TestAppContext::single();
        let _cleanup = TestAppCleanup::new(&cx);
        let app = app_with_visible_local_session(&mut cx, root.path(), "local");
        let cx = session_focus_test_window(&mut cx, &app);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(!app.native_rdp_overlay_state(false, cx).obscured);
                app.open_tab_actions_at("local".into(), None, window, cx);
                let state = app.native_rdp_overlay_state(false, cx);
                assert!(
                    state.obscured,
                    "the tab context menu must not sit behind the native host"
                );
                assert!(
                    state.snapshot,
                    "a menu overlay should leave a still frame behind it"
                );
                app.shell.set_header_status_menu_open(true);
                let state = app.native_rdp_overlay_state(false, cx);
                assert!(
                    state.obscured,
                    "the header status dropdown must not sit behind the native host"
                );
                assert!(state.snapshot);
            });
        });
    }

    #[test]
    fn the_menu_swap_never_exposes_a_black_panel() {
        // The menu opens while the popup is still on screen: capture the frame
        // and keep the popup for this update, so the frame can be presented.
        let plan = native_host_plan(true, true, true, true, false);
        assert!(plan.show_host);
        assert_eq!(plan.snapshot, NativeSnapshotAction::Capture);
        // The frame exists now: hide the popup behind it.
        let plan = native_host_plan(true, true, true, true, true);
        assert!(!plan.show_host);
        assert_eq!(plan.snapshot, NativeSnapshotAction::Keep);
        // The frame has to survive every later update while the menu is open.
        let plan = native_host_plan(true, true, true, false, true);
        assert!(!plan.show_host);
        assert_eq!(plan.snapshot, NativeSnapshotAction::Keep);
        // The menu closed: put the popup back before dropping the frame, so the
        // panel never paints black between the two.
        let plan = native_host_plan(true, false, false, false, true);
        assert!(plan.show_host);
        assert_eq!(plan.snapshot, NativeSnapshotAction::Keep);
        // The popup is back and covers the panel: now the frame can go.
        let plan = native_host_plan(true, false, false, true, true);
        assert!(plan.show_host);
        assert_eq!(plan.snapshot, NativeSnapshotAction::Clear);
        // The lock screen never keeps a frame of the remote desktop.
        let plan = native_host_plan(true, true, false, true, true);
        assert!(!plan.show_host);
        assert_eq!(plan.snapshot, NativeSnapshotAction::Clear);
        // A hidden tab has no panel at all.
        let plan = native_host_plan(false, false, false, true, true);
        assert!(!plan.show_host);
        assert_eq!(plan.snapshot, NativeSnapshotAction::Clear);
        // A hidden popup cannot be captured, so nothing is invented.
        let plan = native_host_plan(true, true, true, false, false);
        assert!(!plan.show_host);
        assert_eq!(plan.snapshot, NativeSnapshotAction::Keep);
    }

    fn still_frame_probe(
        image: Arc<RenderImage>,
        slot: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    ) -> impl IntoElement {
        let mut view = div().size_full().bg(rgb(0x000000));
        view = view.child(
            img(image)
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .object_fit(ObjectFit::Fill),
        );
        view.child(
            div().size_full().bg(rgb(0x00ff00)).child(
                canvas(
                    move |bounds, _, _| {
                        slot.set(Some(bounds));
                        bounds
                    },
                    |_, _, _, _| {},
                )
                .size_full(),
            ),
        )
    }

    struct EmptyRoot;

    impl gpui::Render for EmptyRoot {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div()
        }
    }

    #[gpui::test]
    fn the_still_frame_does_not_displace_the_native_canvas(cx: &mut TestAppContext) {
        let image = native_snapshot_image(&NativeRdpSnapshot {
            width: 2,
            height: 1,
            bgra: vec![0; 8],
        })
        .expect("still frame image");
        let slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let probe_slot = slot.clone();
        let (_, cx) = cx.add_window_view(|_, _| EmptyRoot);
        let cx: &mut gpui::VisualTestContext = cx;
        cx.draw(
            gpui::point(gpui::px(0.), gpui::px(0.)),
            gpui::size(gpui::px(800.), gpui::px(600.)),
            move |_window, _cx| still_frame_probe(image, probe_slot.clone()).into_any_element(),
        );
        let bounds = slot.get().expect("canvas prepainted");
        assert_eq!(
            bounds.origin,
            gpui::point(gpui::px(0.), gpui::px(0.)),
            "the still frame pushed the native canvas to {bounds:?}"
        );
    }

    #[test]
    fn native_snapshot_keeps_bgra_and_forces_opaque_alpha() {
        let snapshot = NativeRdpSnapshot {
            width: 2,
            height: 1,
            bgra: vec![1, 2, 3, 0, 4, 5, 6, 128],
        };
        let image = native_snapshot_bgra_opaque(&snapshot).expect("valid snapshot");
        assert_eq!(image.dimensions(), (2, 1));
        // GPUI uploads BGRA, so red and blue must not be swapped here: doing so
        // turns a blue remote desktop orange behind a menu.
        assert_eq!(image.get_pixel(0, 0).0, [1, 2, 3, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [4, 5, 6, 255]);

        let truncated = NativeRdpSnapshot {
            width: 2,
            height: 2,
            bgra: vec![0; 4],
        };
        assert!(native_snapshot_bgra_opaque(&truncated).is_none());
    }

    #[test]
    fn a_clean_workspace_does_not_report_an_overlay() {
        let root = TestConfigDir::new("nyaterm-native-overlay-policy");
        let mut cx = TestAppContext::single();
        let _cleanup = TestAppCleanup::new(&cx);
        let app = app_with_visible_local_session(&mut cx, root.path(), "local");
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                assert!(!app.native_rdp_overlay_state(false, cx).obscured);
                assert!(app.native_rdp_overlay_state(true, cx).obscured);
            });
        });
    }

    #[test]
    fn only_auto_falls_back_when_the_control_cannot_be_created() {
        assert_eq!(
            native_creation_fallback(RdpBackend::Auto),
            Some(RdpBackend::Builtin)
        );
        assert_eq!(native_creation_fallback(RdpBackend::Windows), None);
        assert_eq!(native_creation_fallback(RdpBackend::Builtin), None);
    }

    #[test]
    fn takeover_disconnects_explain_themselves() {
        let extended = NativeRdpDisconnectInfo {
            extended_reason: Some(5),
            ..Default::default()
        };
        assert!(native_disconnect_message(&extended).contains("taken over"));

        let basic = NativeRdpDisconnectInfo {
            basic_reason: Some(3),
            ..Default::default()
        };
        assert!(native_disconnect_message(&basic).contains("taken over"));

        let logged_off = NativeRdpDisconnectInfo {
            extended_reason: Some(12),
            ..Default::default()
        };
        assert!(native_disconnect_message(&logged_off).contains("logged off"));

        let described = NativeRdpDisconnectInfo {
            description: Some("The text shown by mstsc.".into()),
            ..Default::default()
        };
        assert_eq!(
            native_disconnect_message(&described),
            "The text shown by mstsc."
        );
        assert!(!native_disconnect_message(&NativeRdpDisconnectInfo::default()).is_empty());
    }

    #[test]
    fn failed_and_disconnected_native_sessions_release_the_panel() {
        assert!(native_rdp_state_is_displayable(
            RemoteDesktopViewState::Connecting
        ));
        assert!(native_rdp_state_is_displayable(
            RemoteDesktopViewState::Connected
        ));
        assert!(!native_rdp_state_is_displayable(
            RemoteDesktopViewState::Disconnecting
        ));
        assert!(!native_rdp_state_is_displayable(
            RemoteDesktopViewState::Disconnected
        ));
        assert!(!native_rdp_state_is_displayable(
            RemoteDesktopViewState::Failed
        ));
    }

    #[test]
    fn native_hosts_require_a_laid_out_tab_and_no_overlay() {
        assert!(native_host_plan(true, false, false, true, false).show_host);
        assert!(!native_host_plan(false, false, false, true, false).show_host);
        assert!(!native_host_plan(false, true, true, true, true).show_host);
        assert!(!native_host_plan(true, true, false, true, false).show_host);
    }

    #[test]
    fn fit_window_dimensions_are_even_and_bounded() {
        assert_eq!(
            fit_native_desktop_dimensions(NativeRdpRect {
                x: 0,
                y: 0,
                width: 3429,
                height: 9001,
            }),
            (3428, 8192)
        );
        assert_eq!(
            fit_native_desktop_dimensions(NativeRdpRect {
                x: 0,
                y: 0,
                width: 0,
                height: 199,
            }),
            (200, 200)
        );
    }
}
