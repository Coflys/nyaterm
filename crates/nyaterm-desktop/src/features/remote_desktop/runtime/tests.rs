use std::time::{Duration, Instant};

use gpui::Modifiers;
use nyaterm_core::ConnectionAuth;
use nyaterm_remote_desktop::{
    CursorPosition, RdpError, RdpErrorKind, RdpServerCapabilities, RemoteDesktopViewState,
    VncServerCapabilities, VncSessionState,
};

use super::{
    MAINTENANCE_INTERVAL, POINTER_MOVE_INTERVAL, RESIZE_DEBOUNCE, cancel_rdp_reconnect_schedule,
    clear_rdp_reconnect_after_frame, defer_rdp_pointer_move, inline_remote_desktop_password,
    rdp_error_is_retryable, rdp_key_modifiers, rdp_reconnect_delay,
    record_remote_cursor_position_if_sent, remote_committed_text_supported,
    remote_desktop_password_id, remote_desktop_periodic_delay, remote_modifier_transitions,
    remote_state_clears_input, secure_attention_available, vnc_capabilities_for_state,
    vnc_input_allowed, vnc_keysym_for_key,
};
use crate::features::remote_desktop::state::RemoteDesktopSessionState;

#[test]
fn fit_window_connect_waits_for_the_panel_then_asks_for_its_size() {
    use super::start::{RdpStartPlan, plan_rdp_start};
    use nyaterm_remote_desktop::{RdpDisplayConfig, RdpDisplayMetrics, RdpDisplayMode};

    let display = RdpDisplayConfig {
        mode: RdpDisplayMode::FitWindow,
        width: 1920,
        height: 1080,
        ..Default::default()
    };
    assert_eq!(
        plan_rdp_start(&display, None),
        RdpStartPlan::WaitForPanel,
        "the saved 1920x1080 must not be negotiated while the panel is still unknown"
    );

    let measured = RdpDisplayMetrics {
        width: 1403,
        height: 749,
        desktop_scale_factor: 150,
        physical_size_mm: None,
    };
    let RdpStartPlan::Connect(negotiated) = plan_rdp_start(&display, Some(measured)) else {
        panic!("a measured panel is enough to connect");
    };
    assert_eq!((negotiated.width, negotiated.height), (1402, 748));
    assert_eq!(negotiated.desktop_scale_factor, 150);
    assert_eq!(negotiated.mode, RdpDisplayMode::FitWindow);
}

#[test]
fn fixed_size_connect_never_waits_and_never_takes_the_panel_size() {
    use super::start::{RdpStartPlan, plan_rdp_start};
    use nyaterm_remote_desktop::{RdpDisplayConfig, RdpDisplayMetrics, RdpDisplayMode};

    let display = RdpDisplayConfig {
        mode: RdpDisplayMode::Fixed,
        width: 1024,
        height: 768,
        ..Default::default()
    };
    assert_eq!(
        plan_rdp_start(&display, None),
        RdpStartPlan::Connect(display.clone())
    );
    let RdpStartPlan::Connect(negotiated) = plan_rdp_start(
        &display,
        Some(RdpDisplayMetrics {
            width: 2560,
            height: 1440,
            desktop_scale_factor: 200,
            physical_size_mm: None,
        }),
    ) else {
        panic!("a fixed size connects without measuring");
    };
    assert_eq!(negotiated, display);
}

/// A fit-window request carrying the saved 1920x1080, which is the desktop the
/// server must stop being asked for once the panel is known.
fn fit_window_rdp_config(name: &str) -> nyaterm_remote_desktop::RdpSessionConfig {
    use nyaterm_remote_desktop::{
        RdpCertificatePolicy, RdpClipboardConfig, RdpDisplayConfig, RdpDisplayMode,
        RdpReconnectConfig, RdpSessionConfig,
    };

    RdpSessionConfig {
        relay: None,
        name: name.into(),
        host: "127.0.0.1".into(),
        port: 1,
        username: String::new(),
        domain: String::new(),
        password: None,
        use_nla: false,
        certificate_policy: RdpCertificatePolicy::default(),
        administrative_session: false,
        display: RdpDisplayConfig {
            mode: RdpDisplayMode::FitWindow,
            width: 1920,
            height: 1080,
            ..Default::default()
        },
        clipboard: RdpClipboardConfig::default(),
        reconnect: RdpReconnectConfig::default(),
    }
}

/// The held connect has to be started by a real layout, because that is the only
/// measurement a connecting tab ever gets: the framebuffer canvas, which reports
/// the same bounds, does not exist until a frame arrives.
#[test]
fn a_drawn_connecting_tab_hands_its_panel_to_the_connect() {
    use gpui::{
        AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, VisualTestContext, Window, div, px,
    };
    use nyaterm_core::test_support::TestTempDir;

    use super::resize::fit_window_display_metrics;
    use crate::features::NyaTermApp;
    use crate::models::SessionLaunchConfig;

    struct PanelHost {
        app: Entity<NyaTermApp>,
        session_id: String,
    }

    impl Render for PanelHost {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let session_id = self.session_id.clone();
            div().w(px(1_281.)).h(px(721.)).child(
                self.app
                    .update(cx, |app, cx| app.remote_desktop_view(session_id, cx)),
            )
        }
    }

    let root = TestTempDir::new("nyaterm-rdp-panel-measurement");
    let mut cx = TestAppContext::single();
    let _test_app_cleanup = crate::test_support::TestAppCleanup::new(&cx);
    let session_id = "rdp-panel";
    let app = crate::features::test_support::app_with_visible_local_session(
        &mut cx,
        root.path(),
        session_id,
    );
    let config = fit_window_rdp_config("panel");
    cx.update_entity(&app, |app, cx| {
        app.sync_component_theme(cx);
        app.session.metadata_mut(session_id).unwrap().launch_config =
            SessionLaunchConfig::Rdp(config.clone());
        app.remote_desktop.insert_connecting(session_id.to_string());
        app.begin_rdp_runtime(session_id, config, cx);
        assert!(
            app.remote_desktop
                .sessions
                .get(session_id)
                .unwrap()
                .deferred_start
                .is_some(),
            "nothing has drawn the tab yet, so the connect is still held"
        );
    });

    let host_app = app.clone();
    let (_host, vcx) = cx.add_window_view(move |_, _| PanelHost {
        app: host_app,
        session_id: session_id.to_string(),
    });
    let vcx: &mut VisualTestContext = vcx;
    for _ in 0..2 {
        vcx.run_until_parked();
        vcx.update(|window, cx| {
            window.refresh();
            _ = window.draw(cx);
        });
    }
    vcx.run_until_parked();

    app.update(&mut cx, |app, _| {
        let session = app.remote_desktop.sessions.get(session_id).unwrap();
        let viewport = session
            .viewport
            .expect("the connecting tab reports the panel it was drawn into");
        assert_eq!(
            (
                f32::from(viewport.size.width),
                f32::from(viewport.size.height)
            ),
            (1_281., 721.),
            "the probe takes the panel's own bounds, not the window's"
        );
        assert!(
            session.deferred_start.is_none(),
            "the frame consumed the held launch request"
        );
        assert_eq!(
            session.last_resize,
            Some(fit_window_display_metrics(viewport, session.viewport_scale_factor).clamped()),
            "the desktop is negotiated from the panel that was drawn"
        );
        assert_ne!(
            session
                .last_resize
                .map(|metrics| (metrics.width, metrics.height)),
            Some((1_920, 1_080)),
            "the saved size must not be the first negotiation"
        );
    });
}

#[test]
fn a_collapsed_pane_is_not_taken_for_a_panel_size() {
    use super::resize::panel_is_measurable;
    use gpui::{Bounds, point, px, size};

    assert!(panel_is_measurable(Bounds::new(
        point(px(0.), px(0.)),
        size(px(1_024.), px(640.))
    )));
    for (width, height) in [(1_024., 20.), (20., 640.), (0., 0.)] {
        assert!(
            !panel_is_measurable(Bounds::new(
                point(px(0.), px(0.)),
                size(px(width), px(height))
            )),
            "a {width}x{height} pane must not negotiate or resize the desktop"
        );
    }
}

#[test]
fn an_unmeasured_panel_holds_the_connect_and_the_first_measurement_sizes_it() {
    use gpui::{Bounds, TestAppContext, point, px, size};
    use nyaterm_core::test_support::TestTempDir;
    use nyaterm_remote_desktop::RdpDisplayMetrics;

    use crate::models::SessionLaunchConfig;

    let root = TestTempDir::new("nyaterm-rdp-deferred-start");
    let mut cx = TestAppContext::single();
    let _test_app_cleanup = crate::test_support::TestAppCleanup::new(&cx);
    let session_id = "rdp-deferred";
    let app = crate::features::test_support::app_with_visible_local_session(
        &mut cx,
        root.path(),
        session_id,
    );
    let config = fit_window_rdp_config("deferred");
    app.update(&mut cx, |app, cx| {
        app.session.metadata_mut(session_id).unwrap().launch_config =
            SessionLaunchConfig::Rdp(config.clone());
        app.remote_desktop.insert_connecting(session_id.to_string());
        app.begin_rdp_runtime(session_id, config, cx);
    });
    app.update(&mut cx, |app, _| {
        let session = app.remote_desktop.sessions.get(session_id).unwrap();
        assert!(
            session.deferred_start.is_some(),
            "the connect waits for the panel instead of negotiating the saved size"
        );
        assert!(
            app.remote_desktop.manager.state(session_id).is_none(),
            "nothing was negotiated with the server yet"
        );
    });

    // The first layout of the tab, at 150% window scaling.
    let viewport = Bounds::new(point(px(0.), px(0.)), size(px(1_281.), px(721.)));
    app.update(&mut cx, |app, cx| {
        app.update_rdp_viewport(session_id, viewport, 1.5, cx);
    });
    app.update(&mut cx, |app, _| {
        let session = app.remote_desktop.sessions.get(session_id).unwrap();
        assert!(
            session.deferred_start.is_none(),
            "the measurement consumed the held launch request"
        );
        assert_eq!(
            session.last_resize,
            Some(RdpDisplayMetrics {
                width: 1_922,
                height: 1_082,
                desktop_scale_factor: 150,
                physical_size_mm: None,
            }),
            "the panel desktop is the one being negotiated, not one to be resized to later"
        );
        let negotiated_or_reported = app.remote_desktop.manager.state(session_id).is_some()
            || session.state == RemoteDesktopViewState::Failed;
        assert!(
            negotiated_or_reported,
            "the held request was neither connected nor reported as a failure: state={:?} error={:?}",
            session.state,
            session.error
        );
    });
}

#[test]
fn vnc_retry_without_password_opens_and_cancels_dialog_without_leasing_root() {
    use gpui::{AppContext as _, Context, IntoElement, Render, TestAppContext, Window};
    use nyaterm_core::test_support::TestTempDir;
    use nyaterm_remote_desktop::VncSessionConfig;
    use nyaterm_ui::{NyaDialogWindowExt as _, nya_root};

    use crate::features::test_support::app_with_visible_local_session;
    use crate::models::SessionLaunchConfig;

    struct EmptyView;
    impl Render for EmptyView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
        }
    }

    let root = TestTempDir::new("nyaterm-vnc-retry-password-dialog");
    let mut cx = TestAppContext::single();
    let _test_app_cleanup = crate::test_support::TestAppCleanup::new(&cx);
    let app = app_with_visible_local_session(&mut cx, root.path(), "vnc-retry");
    app.update(&mut cx, |app, cx| app.sync_component_theme(cx));
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|_| EmptyView);
        nya_root(view, window, cx)
    });
    cx.update(|window, cx| {
        let handle = window.window_handle().downcast().unwrap();
        app.update(cx, |app, cx| {
            app.shell.remember_main_window(handle);
            app.session.metadata_mut("vnc-retry").unwrap().launch_config =
                SessionLaunchConfig::Vnc(VncSessionConfig {
                    username: String::new(),
                    relay: None,
                    name: "VNC retry".into(),
                    host: "127.0.0.1".into(),
                    port: 5900,
                    password: None,
                    security: Default::default(),
                    display: Default::default(),
                    clipboard: Default::default(),
                    reconnect: Default::default(),
                    shared: true,
                    view_only: false,
                });
            app.remote_desktop.insert_connecting("vnc-retry".into());
            app.retry_rdp_runtime("vnc-retry", cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(window.has_active_nya_dialog(cx));
        assert_eq!(
            app.read(cx).remote_desktop.sessions["vnc-retry"].state,
            RemoteDesktopViewState::Connecting,
        );
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(!window.has_active_nya_dialog(cx));
        assert_eq!(
            app.read(cx).remote_desktop.sessions["vnc-retry"].state,
            RemoteDesktopViewState::Disconnected,
        );
    });
}

#[test]
fn reconnect_classification_only_accepts_transient_failures() {
    for kind in [
        RdpErrorKind::Timeout,
        RdpErrorKind::ConnectionRefused,
        RdpErrorKind::Tls,
        RdpErrorKind::Transport,
    ] {
        assert!(rdp_error_is_retryable(kind), "{kind:?}");
    }
    for kind in [
        // The server ended the session and said why, or the session broke in a way that
        // is not a lost connection. Retrying restarts the same decision.
        RdpErrorKind::Session,
        RdpErrorKind::Authentication,
        RdpErrorKind::CertificateRejected,
        RdpErrorKind::Negotiation,
        RdpErrorKind::Clipboard,
        RdpErrorKind::HelperMissing,
        RdpErrorKind::HelperCrashed,
        RdpErrorKind::Ipc,
        RdpErrorKind::Protocol,
        RdpErrorKind::Unsupported,
        RdpErrorKind::ResourceExhausted,
    ] {
        assert!(!rdp_error_is_retryable(kind), "{kind:?}");
    }
}

#[test]
fn reconnect_password_selection_resolves_locked_values_by_id() {
    let auth = ConnectionAuth {
        mode: "password".to_string(),
        password: Some("masked-or-encrypted".to_string().into()),
        password_id: Some("pw-rdp".to_string()),
        has_password: true,
        ..ConnectionAuth::default()
    };

    assert_eq!(inline_remote_desktop_password(Some(&auth)), None);
    assert_eq!(
        remote_desktop_password_id(Some(&auth)).as_deref(),
        Some("pw-rdp")
    );
}

#[test]
fn reconnect_backoff_caps_and_bounds_jitter() {
    let expected = [1, 2, 4, 8, 15, 30, 30];
    for (index, seconds) in expected.into_iter().enumerate() {
        assert_eq!(
            rdp_reconnect_delay(index as u32 + 1, 0),
            Duration::from_secs(seconds)
        );
    }
    assert_eq!(rdp_reconnect_delay(1, 999), Duration::from_millis(1_249));
}

/// A waiting pointer move is the one thing here that needs a fine cadence.
///
/// Its send is budgeted against `POINTER_MOVE_INTERVAL`, so a flush on any coarser
/// schedule is a visibly late cursor -- which is what the runtime tick's 500ms
/// quiet interval was doing, since `runtime_quiet_tick_allowed` has no
/// remote-desktop term. Everything else here debounces or gates itself.
#[test]
fn a_waiting_pointer_move_gets_the_fine_cadence() {
    assert_eq!(
        remote_desktop_periodic_delay(true),
        POINTER_MOVE_INTERVAL,
        "a coalesced pointer move must not wait longer than its own send interval"
    );
    assert_eq!(remote_desktop_periodic_delay(false), MAINTENANCE_INTERVAL);
    assert!(
        MAINTENANCE_INTERVAL < RESIZE_DEBOUNCE,
        "the maintenance cadence has to be finer than the shortest thing it              services, or a resize debounce resolves late"
    );
}

#[test]
fn deferred_rdp_pointer_move_tracks_locally_before_protocol_flush() {
    let mut pending_pointer = Some((3, 4));
    let mut cursor_position = CursorPosition { x: 1, y: 2 };

    defer_rdp_pointer_move(&mut pending_pointer, &mut cursor_position, (9, 7));

    assert_eq!(pending_pointer, Some((9, 7)));
    assert_eq!(cursor_position, CursorPosition { x: 9, y: 7 });
}

#[test]
fn remote_cursor_position_advances_only_after_pointer_send_succeeds() {
    let mut cursor_position = CursorPosition { x: 1, y: 2 };

    assert!(!record_remote_cursor_position_if_sent(
        &mut cursor_position,
        (9, 7),
        false,
    ));
    assert_eq!(cursor_position, CursorPosition { x: 1, y: 2 });

    assert!(record_remote_cursor_position_if_sent(
        &mut cursor_position,
        (9, 7),
        true,
    ));
    assert_eq!(cursor_position, CursorPosition { x: 9, y: 7 });
}

#[test]
fn first_frame_clears_reconnect_attempt_and_error_state() {
    let mut session = RemoteDesktopSessionState {
        reconnect_attempts: 4,
        reconnect_at: Some(Instant::now()),
        error: Some(RdpError::new(RdpErrorKind::Transport, "interrupted").into()),
        ..Default::default()
    };

    clear_rdp_reconnect_after_frame(&mut session);

    assert_eq!(session.reconnect_attempts, 0);
    assert_eq!(session.reconnect_at, None);
    assert_eq!(session.error, None);
}

/// The terminal-error path calls this right after recording the error it just received, so
/// unlike the first-frame reset it must leave `error` in place.
#[test]
fn cancelling_a_pending_retry_keeps_the_error_being_shown() {
    let mut session = RemoteDesktopSessionState {
        reconnect_attempts: 2,
        reconnect_at: Some(Instant::now()),
        error: Some(RdpError::new(RdpErrorKind::Session, "terminated").into()),
        ..Default::default()
    };

    cancel_rdp_reconnect_schedule(&mut session);

    assert_eq!(session.reconnect_attempts, 0);
    assert_eq!(session.reconnect_at, None);
    assert!(session.error.is_some());
}

#[test]
fn committed_text_requires_the_confirmed_protocol_capability() {
    let rdp_supported = RdpServerCapabilities {
        committed_unicode_text: true,
        secure_attention: false,
    };
    let vnc_supported = VncServerCapabilities {
        committed_unicode_keysyms: true,
    };

    assert!(!remote_committed_text_supported(false, None, None));
    assert!(!remote_committed_text_supported(
        false,
        Some(RdpServerCapabilities::default()),
        None,
    ));
    assert!(remote_committed_text_supported(
        false,
        Some(rdp_supported),
        None,
    ));
    assert!(!remote_committed_text_supported(true, None, None));
    assert!(!remote_committed_text_supported(
        true,
        None,
        Some(VncServerCapabilities::default()),
    ));
    assert!(remote_committed_text_supported(
        true,
        None,
        Some(vnc_supported),
    ));
}

#[test]
fn rdp_shifted_punctuation_keeps_the_reported_modifier_state() {
    let reported = Modifiers::default();
    assert!(rdp_key_modifiers("!", reported, true).shift);
    assert!(rdp_key_modifiers("?", reported, true).shift);
    assert!(!rdp_key_modifiers("!", reported, false).shift);
    assert!(!rdp_key_modifiers("a", reported, true).shift);
}

#[test]
fn modifier_transitions_press_in_order_release_in_reverse_and_toggle_capslock() {
    let pressed = remote_modifier_transitions(
        Modifiers::default(),
        Modifiers {
            control: true,
            shift: true,
            platform: true,
            ..Default::default()
        },
        None,
        Some(true),
    );
    assert_eq!(
        pressed
            .iter()
            .map(|transition| (transition.key, transition.pressed))
            .collect::<Vec<_>>(),
        vec![("shift", true), ("control", true), ("platform", true)]
    );

    let released = remote_modifier_transitions(
        Modifiers {
            control: true,
            shift: true,
            platform: true,
            ..Default::default()
        },
        Modifiers::default(),
        Some(true),
        Some(false),
    );
    assert_eq!(
        released
            .iter()
            .map(|transition| (transition.key, transition.pressed))
            .collect::<Vec<_>>(),
        vec![
            ("platform", false),
            ("control", false),
            ("shift", false),
            ("capslock", true),
            ("capslock", false),
        ]
    );
}

#[test]
fn vnc_capability_cache_only_survives_connected_state() {
    let supported = Some(VncServerCapabilities {
        committed_unicode_keysyms: true,
    });

    assert_eq!(
        vnc_capabilities_for_state(&VncSessionState::Connected, supported),
        supported,
    );
    assert_eq!(
        vnc_capabilities_for_state(&VncSessionState::Reconnecting, supported),
        None,
    );
    assert_eq!(
        vnc_capabilities_for_state(&VncSessionState::Disconnected, supported),
        None,
    );
}

#[test]
fn physical_vnc_key_mapping_preserves_shortcut_and_navigation_keys() {
    assert_eq!(vnc_keysym_for_key("c", Some("c")), Some(u32::from('c')));
    assert_eq!(vnc_keysym_for_key("left", None), Some(0xff51));
    assert_eq!(vnc_keysym_for_key("F12", None), Some(0xffc9));
}

#[test]
fn vnc_view_only_is_rejected_before_manager_dispatch() {
    assert!(vnc_input_allowed(false));
    assert!(!vnc_input_allowed(true));
}

#[test]
fn secure_attention_requires_rdp_connected_state_and_confirmed_capability() {
    let supported = Some(RdpServerCapabilities {
        committed_unicode_text: true,
        secure_attention: true,
    });
    assert!(secure_attention_available(
        true,
        &RemoteDesktopViewState::Connected,
        supported,
    ));
    assert!(!secure_attention_available(
        false,
        &RemoteDesktopViewState::Connected,
        supported,
    ));
    assert!(!secure_attention_available(
        true,
        &RemoteDesktopViewState::Connecting,
        supported,
    ));
    assert!(!secure_attention_available(
        true,
        &RemoteDesktopViewState::Connected,
        None,
    ));
    assert!(!secure_attention_available(
        true,
        &RemoteDesktopViewState::Connected,
        Some(RdpServerCapabilities::default()),
    ));
}

#[test]
fn disconnecting_states_clear_local_composition_and_key_suppression() {
    assert!(!remote_state_clears_input(
        &RemoteDesktopViewState::Connected
    ));
    assert!(remote_state_clears_input(
        &RemoteDesktopViewState::Reconnecting
    ));
    assert!(remote_state_clears_input(
        &RemoteDesktopViewState::Disconnected
    ));
    assert!(remote_state_clears_input(&RemoteDesktopViewState::Failed));
}

#[test]
fn texture_failure_isolated_and_inactive_framebuffer_can_rebuild() {
    use crate::features::test_support::app_with_visible_local_session;
    use gpui::TestAppContext;
    use nyaterm_core::test_support::TestTempDir;
    use nyaterm_remote_desktop::{
        CursorShape, RDP_FRAMEBUFFER_LIMITS, RemoteDesktopErrorCategory, RemotePoint,
    };

    let root = TestTempDir::new("nyaterm-remote-texture-failure");
    let mut cx = TestAppContext::single();
    let _cleanup = crate::test_support::TestAppCleanup::new(&cx);
    let app = app_with_visible_local_session(&mut cx, root.path(), "healthy");
    struct EmptyView;
    impl gpui::Render for EmptyView {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }
    let (_, cx) = cx.add_window_view(|_, _| EmptyView);
    cx.update(|window, cx| {
        app.update(cx, |app, _| {
            app.remote_desktop.insert_connecting("healthy".into());
            app.remote_desktop.insert_connecting("broken".into());
            app.reset_rdp_framebuffer("healthy", 1, 8, 8, RDP_FRAMEBUFFER_LIMITS, window);
            let texture = app.remote_desktop.sessions["healthy"].texture.unwrap();
            let pixels = app.remote_desktop.sessions["healthy"]
                .framebuffer
                .as_ref()
                .unwrap()
                .pixels()
                .as_ptr();
            app.remote_desktop.release_inactive_textures("local");
            assert!(app.remote_desktop.sessions["healthy"].texture.is_none());
            assert!(app.remote_desktop.sessions["healthy"].texture_dirty);
            assert_eq!(
                app.remote_desktop.sessions["healthy"]
                    .framebuffer
                    .as_ref()
                    .unwrap()
                    .pixels()
                    .as_ptr(),
                pixels
            );
            for old in app.remote_desktop.pending_texture_removals.drain(..) {
                window.remove_dynamic_texture(old);
            }
            app.prepare_remote_desktop_textures("healthy", window);
            assert_ne!(
                app.remote_desktop.sessions["healthy"].texture,
                Some(texture)
            );
            assert!(!app.remote_desktop.sessions["healthy"].texture_dirty);

            let broken = app.remote_desktop.sessions.get_mut("broken").unwrap();
            broken.reconnect_at = Some(Instant::now());
            // Invalid upload data deterministically exercises a platform upload
            // failure without exhausting CPU/GPU resources.
            broken.cursor_shape = Some(CursorShape {
                shape_id: 1,
                width: 1,
                height: 1,
                hotspot: RemotePoint { x: 0, y: 0 },
                pixels: vec![0; 3].into(),
            });
            app.prepare_remote_desktop_textures("broken", window);
            let broken = &app.remote_desktop.sessions["broken"];
            assert!(broken.uploads_stopped);
            assert!(broken.reconnect_at.is_none());
            assert!(broken.framebuffer.is_none());
            assert!(broken.cursor_shape.is_none());
            assert_eq!(broken.state, RemoteDesktopViewState::Failed);
            assert_eq!(
                broken.error.as_ref().unwrap().category,
                RemoteDesktopErrorCategory::ResourceExhausted
            );
            app.reset_rdp_framebuffer("broken", 2, 8, 8, RDP_FRAMEBUFFER_LIMITS, window);
            assert!(app.remote_desktop.sessions["broken"].framebuffer.is_none());
            assert!(app.remote_desktop.sessions["healthy"].texture.is_some());
            assert!(!app.remote_desktop.sessions["healthy"].uploads_stopped);
        });
    });
}
