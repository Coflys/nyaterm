use std::collections::HashSet;

use gpui::{
    Bounds, Context, IntoElement, ParentElement as _, Pixels, Styled as _, canvas, div, rgb,
};
use nyaterm_remote_desktop::activex::{
    NativeRdpConnectionEvent, NativeRdpControl, NativeRdpDisconnectInfo, NativeRdpRect,
    NativeRdpSessionDetails,
};
use nyaterm_remote_desktop::{RdpError, RdpErrorKind, RemoteDesktopViewState};
use nyaterm_ui::NyaDialogWindowExt as _;
use raw_window_handle::RawWindowHandle;

use crate::features::NyaTermApp;

impl NyaTermApp {
    pub(super) fn native_rdp_view(
        &mut self,
        session_id: String,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let app = cx.entity();
        div()
            .size_full()
            .bg(rgb(0x101010))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(0x999999))
                    .child(rust_i18n::t!("dialog.rdpActiveXHidden").to_string()),
            )
            .child(
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
                        let rect = native_phys_rect(bounds, window.scale_factor());
                        let dialog_open = window.has_active_nya_dialog(cx)
                            || nyaterm_ui::nya_popup_menu_is_active(window, cx);
                        let app = app.clone();
                        let session_id = session_id.clone();
                        window.defer(cx, move |window, cx| {
                            app.update(cx, |this, cx| {
                                let mut overlay = this.native_rdp_overlay_state(dialog_open, cx);
                                overlay.obscured |= !window.is_visible()
                                    || !this
                                        .native_rdp_visible_tab_roots()
                                        .contains(&this.tab_root_for_session(&session_id));
                                this.ensure_native_rdp(&session_id, rect, parent, overlay, cx);
                            });
                        });
                        bounds
                    },
                    |_, _, _, _| {},
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
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.remote_desktop.sessions.get_mut(session_id) else {
            return;
        };
        if let Some(native) = session.native_rdp.as_mut() {
            native.set_bounds(rect);
            native.set_visible(!overlay.obscured && native_rdp_state_is_displayable(session.state));
            return;
        }
        let Some(mut config) = session.native_rdp_config.take() else {
            return;
        };
        if config.display.mode == nyaterm_remote_desktop::RdpDisplayMode::FitWindow {
            let (width, height) = fit_native_desktop_dimensions(rect);
            config.display.width = width;
            config.display.height = height;
            config.display.desktop_scale_factor = rect.scale_percent;
        }
        match NativeRdpControl::create(parent, rect, &config) {
            Ok(mut native) => {
                native.set_visible(!overlay.obscured);
                session.native_rdp = Some(native);
                session.state = RemoteDesktopViewState::Connecting;
                session.error = None;
                self.ensure_remote_desktop_periodic_clock(cx);
            }
            Err(message) => {
                session.state = RemoteDesktopViewState::Failed;
                session.error = Some(RdpError::new(RdpErrorKind::Unsupported, message).into());
            }
        }
        cx.notify();
    }

    pub(in crate::features) fn native_rdp_overlay_state(
        &mut self,
        dialog_open: bool,
        cx: &mut Context<Self>,
    ) -> NativeRdpOverlayState {
        let menu = self.native_rdp_menu_overlay_obscured(dialog_open, cx);
        NativeRdpOverlayState {
            obscured: self.security.screen_locked() || menu,
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
        obscured: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let visible: HashSet<String> = self
            .remote_desktop
            .sessions
            .iter()
            .filter(|(id, _)| visible_tab_roots.contains(&self.tab_root_for_session(id)))
            .map(|(id, _)| id.clone())
            .collect();
        // A restored background tab still needs a running session. Its saved
        // dimensions are used after the same bounded layout wait as IronRDP;
        // it stays hidden until the tab has a visible panel.
        if let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window)
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            let pending: Vec<_> = self
                .remote_desktop
                .sessions
                .iter()
                .filter_map(|(id, session)| {
                    session.native_rdp_config.as_ref().map(|config| {
                        let rect = session
                            .viewport
                            .map(|bounds| native_phys_rect(bounds, window.scale_factor()))
                            .unwrap_or(NativeRdpRect {
                                x: 0,
                                y: 0,
                                width: config.display.width as i32,
                                height: config.display.height as i32,
                                scale_percent: (window.scale_factor() * 100.)
                                    .round()
                                    .clamp(100., 500.)
                                    as u32,
                            });
                        (id.clone(), rect)
                    })
                })
                .collect();
            for (id, rect) in pending {
                self.ensure_native_rdp(
                    &id,
                    rect,
                    handle.hwnd.get(),
                    NativeRdpOverlayState {
                        obscured: obscured || !window.is_visible() || !visible.contains(&id),
                    },
                    cx,
                );
            }
        }
        let mut dirty = false;
        for (id, session) in &mut self.remote_desktop.sessions {
            let Some(native) = session.native_rdp.as_mut() else {
                continue;
            };
            native.set_visible(
                window.is_visible()
                    && visible.contains(id)
                    && !obscured
                    && native_rdp_state_is_displayable(session.state),
            );
            if window.is_window_active() && native.cursor_is_inside() {
                dirty |= self.shell.clear_resize_handle_hover();
            }
            while let Some(event) = native.poll_connection() {
                match event {
                    NativeRdpConnectionEvent::Connected => {
                        session.state = RemoteDesktopViewState::Connected;
                        session.error = None;
                    }
                    NativeRdpConnectionEvent::Reconnecting => {
                        session.state = RemoteDesktopViewState::Reconnecting;
                    }
                    NativeRdpConnectionEvent::Disconnected(info) => {
                        session.state = RemoteDesktopViewState::Failed;
                        session.error = Some(
                            RdpError::new(RdpErrorKind::Session, native_disconnect_message(&info))
                                .into(),
                        );
                        native.set_visible(false);
                    }
                    NativeRdpConnectionEvent::Failed(error) => {
                        session.state = RemoteDesktopViewState::Failed;
                        session.error = Some(error.into());
                        native.set_visible(false);
                    }
                }
                dirty = true;
            }
        }
        dirty
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::features) struct NativeRdpOverlayState {
    pub obscured: bool,
}

fn native_rdp_state_is_displayable(state: RemoteDesktopViewState) -> bool {
    matches!(
        state,
        RemoteDesktopViewState::Connecting
            | RemoteDesktopViewState::Connected
            | RemoteDesktopViewState::Reconnecting
    )
}

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
        x: (f32::from(bounds.origin.x) * scale).round() as i32,
        y: (f32::from(bounds.origin.y) * scale).round() as i32,
        width: (f32::from(bounds.size.width) * scale).round() as i32,
        height: (f32::from(bounds.size.height) * scale).round() as i32,
        scale_percent: ((scale * 100.).round() as u32).clamp(100, 500),
    }
}

fn fit_native_desktop_dimensions(rect: NativeRdpRect) -> (u32, u32) {
    (
        (rect.width.clamp(200, 8192) as u32) & !1,
        (rect.height.clamp(200, 8192) as u32) & !1,
    )
}

#[cfg(test)]
mod tests {
    use super::{native_disconnect_message, native_rdp_state_is_displayable};
    use nyaterm_remote_desktop::RemoteDesktopViewState;
    use nyaterm_remote_desktop::activex::NativeRdpDisconnectInfo;

    #[test]
    fn failed_native_sessions_release_the_panel() {
        assert!(!native_rdp_state_is_displayable(
            RemoteDesktopViewState::Failed
        ));
        assert!(native_rdp_state_is_displayable(
            RemoteDesktopViewState::Connected
        ));
    }

    #[test]
    fn takeover_disconnects_explain_themselves() {
        let info = NativeRdpDisconnectInfo {
            extended_reason: Some(5),
            ..Default::default()
        };
        assert!(native_disconnect_message(&info).contains("taken over"));
    }
}
