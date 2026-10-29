use std::ffi::c_void;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use nyaterm_remote_desktop::{RdpCertificatePolicy, RdpDisplayConfig, RdpSessionConfig};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, ClientToScreen,
    CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
    GetDIBits, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::Com::{
    DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS, IDispatch,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Ole::{DISPID_PROPERTYPUT, OleInitialize};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, EnumWindows, GA_ROOT, GW_HWNDNEXT, GW_OWNER, GWL_EXSTYLE,
    GetAncestor, GetClientRect, GetCursorPos, GetForegroundWindow, GetWindow, GetWindowLongW,
    GetWindowRect, GetWindowThreadProcessId, IsWindow, IsWindowVisible, MoveWindow, PostMessageW,
    SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
    SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WM_CLOSE, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_EX_DLGMODALFRAME, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE, WindowFromPoint,
};
use windows::core::{BOOL, BSTR, GUID, HSTRING, IUnknown, Interface, PCWSTR, s, w};

const MSTSC_PROG_ID: &str = "MsTscAx.MsTscAx";
const DISPLAY_RESIZE_DEBOUNCE: Duration = Duration::from_millis(300);

static ATL_READY: OnceLock<bool> = OnceLock::new();
static OLE_READY: OnceLock<Result<(), String>> = OnceLock::new();

/// Whether the ActiveX host prerequisites are present.
///
/// This is only a prerequisite check: the control itself can still fail to
/// activate, which is why `Auto` also has a creation-failure fallback.
pub(crate) fn available() -> bool {
    ensure_ole().is_ok() && ensure_atl().is_ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeRdpRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NativeRdpConnectionEvent {
    Connected,
    Disconnected(NativeRdpDisconnectInfo),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NativeRdpDisconnectInfo {
    pub basic_reason: Option<u32>,
    pub extended_reason: Option<u32>,
    /// The ActiveX `GetErrorDescription` text, which is the same text mstsc
    /// shows for the same disconnect.
    pub description: Option<String>,
}

/// Live information exposed by the hosted Windows RDP control.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NativeRdpSessionDetails {
    pub version: Option<String>,
    pub cipher_strength: Option<u32>,
    pub status_text: Option<String>,
    pub desktop_width: Option<u32>,
    pub desktop_height: Option<u32>,
    pub color_depth: Option<u32>,
}

/// A still frame of the native control, used behind in-window menus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeRdpSnapshot {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub(crate) struct NativeRdpControl {
    owner: HWND,
    host: HWND,
    surface: HWND,
    rect: NativeRdpRect,
    control: IDispatch,
    connected_once: bool,
    started: bool,
    desired_visible: bool,
    shown: bool,
    fit_window: bool,
    last_display_size: (u32, u32),
    pending_display_size: Option<((u32, u32), Instant)>,
    modal_sibling: Option<HWND>,
}

impl NativeRdpControl {
    pub(crate) fn create(
        parent: isize,
        rect: NativeRdpRect,
        config: &RdpSessionConfig,
    ) -> Result<Self, String> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Self::create_inner(parent, rect, config)
        }))
        .unwrap_or_else(|_| Err("the Windows RDP host panicked".to_string()))
    }

    fn create_inner(
        parent: isize,
        rect: NativeRdpRect,
        config: &RdpSessionConfig,
    ) -> Result<Self, String> {
        tracing::debug!("native RDP: initializing OLE");
        ensure_ole()?;
        tracing::debug!("native RDP: initializing ATL host");
        ensure_atl()?;
        if parent == 0 || rect.width <= 0 || rect.height <= 0 {
            return Err("native RDP parent or size is invalid".to_string());
        }

        tracing::debug!("native RDP: creating docked host window");
        let owner = HWND(parent as *mut c_void);
        let host = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0x0000_0080),
                w!("STATIC"),
                w!(""),
                WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                rect.width,
                rect.height,
                Some(owner),
                None,
                None,
                None,
            )
        }
        .map_err(|error| format!("cannot create the native RDP host window: {error}"))?;
        tracing::debug!("native RDP: creating AtlAxWin");
        let title = HSTRING::from(MSTSC_PROG_ID);
        let control_hwnd = match unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("AtlAxWin"),
                PCWSTR(title.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                rect.width,
                rect.height,
                Some(host),
                None,
                None,
                None,
            )
        } {
            Ok(hwnd) => hwnd,
            Err(error) => {
                // The host is a raw HWND at this point; leaving it behind would
                // strand an empty popup above the workspace when Auto falls back.
                unsafe {
                    let _ = DestroyWindow(host);
                }
                return Err(format!("cannot create the Windows RDP control: {error}"));
            }
        };

        tracing::debug!("native RDP: acquiring automation interface");
        let control = unsafe { control_from_window(control_hwnd) }.inspect_err(|_| unsafe {
            let _ = DestroyWindow(host);
        })?;
        let native = Self {
            owner,
            host,
            surface: control_hwnd,
            rect,
            control,
            connected_once: false,
            started: false,
            desired_visible: true,
            shown: true,
            fit_window: config.display.mode == nyaterm_remote_desktop::RdpDisplayMode::FitWindow,
            last_display_size: (config.display.width, config.display.height),
            pending_display_size: None,
            modal_sibling: None,
        };
        native.reposition();
        tracing::debug!(
            has_password = config.password.is_some(),
            "native RDP: applying connection settings"
        );
        native.configure(config)?;
        Ok(native)
    }

    pub(crate) fn start(&mut self) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        tracing::debug!("native RDP: starting connection");
        self.invoke_method("Connect")
            .map_err(|error| format!("cannot start the Windows RDP control: {error}"))?;
        self.started = true;
        tracing::debug!("native RDP: connection started");
        Ok(())
    }

    fn configure(&self, config: &RdpSessionConfig) -> Result<(), String> {
        put_bstr(&self.control, "Server", &config.host)?;
        if !config.username.is_empty() {
            put_bstr(&self.control, "UserName", &config.username)?;
        }
        if !config.domain.is_empty() {
            put_bstr(&self.control, "Domain", &config.domain)?;
        }
        let width = i32::try_from(config.display.width).unwrap_or(1920);
        let height = i32::try_from(config.display.height).unwrap_or(1080);
        put_i32(&self.control, "DesktopWidth", width)?;
        put_i32(&self.control, "DesktopHeight", height)?;
        put_i32(
            &self.control,
            "ColorDepth",
            i32::from(config.display.color_depth),
        )?;

        let advanced = get_dispatch(&self.control, "AdvancedSettings2")?;
        put_i32(&advanced, "RDPPort", i32::from(config.port))?;
        put_bool(&advanced, "EnableCredSspSupport", config.use_nla)?;
        put_bool(&advanced, "SmartSizing", true)?;
        put_i32(
            &advanced,
            "AuthenticationLevel",
            authentication_level(config.certificate_policy),
        )?;
        put_bool(
            &advanced,
            "ConnectToServerConsole",
            config.administrative_session,
        )?;
        put_i32(
            &advanced,
            "PerformanceFlags",
            performance_flags(&config.display) as i32,
        )?;
        put_bool(
            &advanced,
            "RedirectClipboard",
            !matches!(
                config.clipboard.mode,
                nyaterm_remote_desktop::RdpClipboardMode::Disabled
            ),
        )?;
        put_bool(&advanced, "EnableAutoReconnect", config.reconnect.enabled)?;

        if let Some(password) = &config.password {
            tracing::debug!("native RDP: setting the saved password");
            if let Err(error) = put_bstr(&advanced, "ClearTextPassword", password.expose_secret()) {
                // Continue without it: the server can still prompt, but the
                // saved-password path needs a diagnosable failure.
                tracing::warn!(%error, "native RDP: saved password was not accepted");
            }
        }
        Ok(())
    }

    pub(crate) fn set_bounds(&mut self, rect: NativeRdpRect) {
        if rect.width <= 0 || rect.height <= 0 {
            return;
        }
        self.rect = rect;
        if self.fit_window {
            let size = display_size(rect);
            if size != self.last_display_size
                && self.pending_display_size.map(|(pending, _)| pending) != Some(size)
            {
                self.pending_display_size = Some((size, Instant::now()));
            }
        }
        self.reposition();
    }

    fn reposition(&self) {
        let mut position = POINT {
            x: self.rect.x,
            y: self.rect.y,
        };
        unsafe {
            let _ = ClientToScreen(self.owner, &mut position);
            let mut current = RECT::default();
            let host_changed = GetWindowRect(self.host, &mut current).is_ok()
                && (current.left != position.x
                    || current.top != position.y
                    || current.right - current.left != self.rect.width
                    || current.bottom - current.top != self.rect.height);
            if host_changed {
                let _ = MoveWindow(
                    self.host,
                    position.x,
                    position.y,
                    self.rect.width,
                    self.rect.height,
                    true,
                );
            }
            let mut surface = RECT::default();
            let surface_changed = GetClientRect(self.surface, &mut surface).is_ok()
                && (surface.right != self.rect.width || surface.bottom != self.rect.height);
            if surface_changed {
                let _ = MoveWindow(self.surface, 0, 0, self.rect.width, self.rect.height, true);
            }
        }
    }

    pub(crate) fn set_visible(&mut self, visible: bool) {
        self.desired_visible = visible;
        self.sync_visibility();
    }

    pub(crate) fn is_shown(&self) -> bool {
        self.shown
    }

    pub(crate) fn cursor_is_inside(&self) -> bool {
        if !self.shown {
            return false;
        }
        let mut point = POINT::default();
        if unsafe { GetCursorPos(&mut point) }.is_err() {
            return false;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(self.host, &mut rect) }.is_err() {
            return false;
        }
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    }

    fn sync_visibility(&mut self) {
        // The host is an owned top-level popup, so it must not hide just because
        // another NyaTerm window (such as the connection editor) owns the
        // foreground. Hiding it blacked out the panel; keeping it above did the
        // same to the dialog, so the ordering is enforced below instead.
        let visible = self.desired_visible;
        if self.shown != visible {
            if visible {
                self.reposition();
            }
            unsafe {
                let _ = ShowWindow(self.host, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
            }
            self.shown = visible;
        }
        if visible {
            self.sync_sibling_order();
        }
    }

    /// Keep the popup below the NyaTerm window that owns the foreground, and
    /// pin it under an open modal dialog until that dialog closes.
    ///
    /// Owned windows do not inherit the GPUI window tree's z-order, so a modal
    /// editor can otherwise be covered by the RDP surface. Input to the popup is
    /// blocked while a modal is open, so clicking the visible RDP area cannot
    /// raise it back over the dialog.
    fn sync_sibling_order(&mut self) {
        let foreground = unsafe { GetForegroundWindow() };
        if !foreground.is_invalid() {
            let root = unsafe { GetAncestor(foreground, GA_ROOT) };
            let sibling = if root.is_invalid() { foreground } else { root };
            if sibling != self.owner && sibling != self.host && self.is_own_window(sibling) {
                let style = unsafe { GetWindowLongW(sibling, GWL_EXSTYLE) } as u32;
                if style & WS_EX_DLGMODALFRAME.0 != 0 && self.belongs_to_owner(sibling) {
                    self.modal_sibling = Some(sibling);
                }
                // Inserting the host after a topmost window would make it topmost too.
                if style & WS_EX_TOPMOST.0 == 0 {
                    self.place_below(sibling);
                }
            }
        }
        // The foreground sample above misses a dialog that was opened and then
        // covered by a click on the RDP host before the next poll. Enumerating
        // owned top-level windows finds it without depending on activation.
        if self.modal_sibling.is_none() {
            self.modal_sibling = find_modal_sibling(self.owner, self.host);
        }
        self.enforce_modal_sibling();
    }

    fn is_own_window(&self, window: HWND) -> bool {
        let mut process_id = 0;
        unsafe {
            let _ = GetWindowThreadProcessId(window, Some(&mut process_id));
        }
        process_id == std::process::id()
    }

    /// Whether `window` belongs to this workspace window rather than another
    /// NyaTerm window. A modal in another workspace must not disable this host.
    fn belongs_to_owner(&self, window: HWND) -> bool {
        owner_chain_contains(window, self.owner)
    }

    fn place_below(&self, sibling: HWND) {
        if unsafe { GetWindow(sibling, GW_HWNDNEXT) }.ok() == Some(self.host) {
            return;
        }
        let reordered = unsafe {
            SetWindowPos(
                self.host,
                Some(sibling),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        };
        if reordered.is_ok() {
            tracing::debug!(?sibling, "native RDP: host lowered below a NyaTerm window");
        }
    }

    fn enforce_modal_sibling(&mut self) {
        let Some(modal) = self.modal_sibling else {
            return;
        };
        let alive = self.is_own_window(modal)
            && unsafe { IsWindow(Some(modal)).as_bool() }
            && unsafe { IsWindowVisible(modal).as_bool() };
        if !alive {
            self.modal_sibling = None;
            if !unsafe { IsWindowEnabled(self.host).as_bool() } {
                unsafe {
                    let _ = EnableWindow(self.host, true);
                }
            }
            return;
        }
        self.place_below(modal);
        if unsafe { IsWindowEnabled(self.host).as_bool() } {
            unsafe {
                let _ = EnableWindow(self.host, false);
            }
        }
    }

    pub(crate) fn poll_connection(&mut self) -> Option<NativeRdpConnectionEvent> {
        self.reposition();
        self.sync_visibility();
        self.flush_display_resize();
        let connected = get_bool(&self.control, "Connected").unwrap_or(false);
        if connected && !self.connected_once {
            self.connected_once = true;
            Some(NativeRdpConnectionEvent::Connected)
        } else if !connected && self.connected_once {
            self.connected_once = false;
            Some(NativeRdpConnectionEvent::Disconnected(
                self.disconnect_info(),
            ))
        } else {
            None
        }
    }

    pub(crate) fn session_details(&self) -> NativeRdpSessionDetails {
        NativeRdpSessionDetails {
            version: get_text(&self.control, "Version")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            cipher_strength: get_u32(&self.control, "CipherStrength")
                .ok()
                .filter(|value| *value > 0),
            status_text: get_text(&self.control, "ConnectedStatusText")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            desktop_width: get_u32(&self.control, "DesktopWidth")
                .ok()
                .filter(|value| *value > 0),
            desktop_height: get_u32(&self.control, "DesktopHeight")
                .ok()
                .filter(|value| *value > 0),
            color_depth: get_u32(&self.control, "ColorDepth")
                .ok()
                .filter(|value| *value > 0),
        }
    }

    /// Capture what is currently on screen in the host client area.
    ///
    /// The screen DC is used instead of the window DC so the ActiveX output is
    /// captured regardless of how it paints; this runs only when a menu is about
    /// to be shown over the popup.
    pub(crate) fn capture_snapshot(&self) -> Option<NativeRdpSnapshot> {
        if !self.shown {
            return None;
        }
        let mut rect = RECT::default();
        if unsafe { GetClientRect(self.host, &mut rect) }.is_err() {
            return None;
        }
        let width = rect.right;
        let height = rect.bottom;
        if width <= 0 || height <= 0 {
            return None;
        }
        let mut origin = POINT { x: 0, y: 0 };
        let _ = unsafe { ClientToScreen(self.host, &mut origin) };
        let screen = unsafe { GetDC(None) };
        if screen.is_invalid() {
            return None;
        }
        let memory = unsafe { CreateCompatibleDC(Some(screen)) };
        if memory.is_invalid() {
            unsafe {
                ReleaseDC(None, screen);
            }
            return None;
        }
        let bitmap = unsafe { CreateCompatibleBitmap(screen, width, height) };
        if bitmap.is_invalid() {
            unsafe {
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
            }
            return None;
        }
        let previous = unsafe { SelectObject(memory, HGDIOBJ(bitmap.0)) };
        let blt = unsafe {
            BitBlt(
                memory,
                0,
                0,
                width,
                height,
                Some(screen),
                origin.x,
                origin.y,
                SRCCOPY | CAPTUREBLT,
            )
        };
        // GetDIBits requires the bitmap not to be selected into a DC.
        unsafe {
            SelectObject(memory, previous);
        }
        let mut pixels = Vec::new();
        if blt.is_ok() {
            pixels = vec![0u8; width as usize * height as usize * 4];
            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            // Negative height stores the first captured row first.
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB.0;
            let lines = unsafe {
                GetDIBits(
                    screen,
                    bitmap,
                    0,
                    height as u32,
                    Some(pixels.as_mut_ptr().cast()),
                    &mut info,
                    DIB_RGB_COLORS,
                )
            };
            if lines <= 0 {
                pixels.clear();
            }
        }
        unsafe {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
        }
        if !pixels.is_empty() {
            let center = POINT {
                x: origin.x + width / 2,
                y: origin.y + height / 2,
            };
            let at_point = unsafe { WindowFromPoint(center) };
            let top = unsafe { GetAncestor(at_point, GA_ROOT) };
            let index = ((height as usize / 2) * width as usize + width as usize / 2) * 4;
            let sample = pixels.get(index..index + 4).unwrap_or(&[0, 0, 0, 0]);
            tracing::info!(
                x = origin.x,
                y = origin.y,
                width,
                height,
                host = ?self.host,
                top = ?top,
                top_is_host = top == self.host,
                r = sample[2],
                g = sample[1],
                b = sample[0],
                "captured still frame"
            );
        }
        (!pixels.is_empty()).then_some(NativeRdpSnapshot {
            width: width as u32,
            height: height as u32,
            bgra: pixels,
        })
    }

    fn disconnect_info(&self) -> NativeRdpDisconnectInfo {
        let basic_reason = get_u32(&self.control, "DisconnectReason").ok();
        let extended_reason = get_u32(&self.control, "ExtendedDisconnectReason").ok();
        NativeRdpDisconnectInfo {
            description: self.error_description(basic_reason, extended_reason),
            basic_reason,
            extended_reason,
        }
    }

    fn error_description(&self, basic: Option<u32>, extended: Option<u32>) -> Option<String> {
        let dispid = get_disp_id(&self.control, "GetErrorDescription").ok()?;
        // IDispatch arguments are reversed: disconnectReason is the first
        // parameter, extendedDisconnectReason the second.
        let mut arguments = [
            VARIANT::from(extended.unwrap_or(0)),
            VARIANT::from(basic.unwrap_or(0)),
        ];
        let params = DISPPARAMS {
            rgvarg: arguments.as_mut_ptr(),
            rgdispidNamedArgs: std::ptr::null_mut(),
            cArgs: 2,
            cNamedArgs: 0,
        };
        let mut result = VARIANT::default();
        unsafe {
            self.control
                .Invoke(
                    dispid,
                    &GUID::zeroed(),
                    0,
                    DISPATCH_METHOD,
                    &params,
                    Some(&mut result),
                    None,
                    None,
                )
                .ok()?;
        }
        let text = BSTR::try_from(&result).ok()?.to_string();
        (!text.trim().is_empty()).then_some(text)
    }

    fn flush_display_resize(&mut self) {
        if !self.connected_once {
            return;
        }
        let Some(((width, height), requested_at)) = self.pending_display_size else {
            return;
        };
        if requested_at.elapsed() < DISPLAY_RESIZE_DEBOUNCE {
            return;
        }
        self.pending_display_size = None;
        // Record both outcomes so a rejected resize does not retry on every
        // subsequent render pass; the control keeps SmartSizing as a fallback.
        self.last_display_size = (width, height);
        match update_session_display_settings(&self.control, width, height) {
            Ok(()) => {
                tracing::info!(width, height, "Windows RDP display resized");
            }
            Err(error) => {
                tracing::warn!(%error, "Windows RDP display resize failed");
            }
        }
    }

    fn invoke_method(&self, name: &str) -> Result<(), String> {
        let dispid = get_disp_id(&self.control, name)?;
        let params = DISPPARAMS::default();
        unsafe {
            self.control
                .Invoke(
                    dispid,
                    &GUID::zeroed(),
                    0,
                    DISPATCH_METHOD,
                    &params,
                    None,
                    None,
                    None,
                )
                .map_err(|error| error.to_string())
        }
    }
}

impl Drop for NativeRdpControl {
    fn drop(&mut self) {
        tracing::debug!("native RDP control releasing");
        unsafe {
            let _ = ShowWindow(self.host, SW_HIDE);
        }
        // Never call Disconnect here. The control can already be disconnected
        // (for example when another session replaced it), and invoking it while
        // the tab is being torn down can re-enter the UI thread and take the
        // process down. Closing the host releases the ActiveX and its socket;
        // the queued close also keeps the teardown out of the current update.
        if unsafe { PostMessageW(Some(self.host), WM_CLOSE, WPARAM(0), LPARAM(0)) }.is_err() {
            unsafe {
                let _ = DestroyWindow(self.host);
            }
        }
    }
}

fn owner_chain_contains(window: HWND, expected: HWND) -> bool {
    let mut owner = unsafe { GetWindow(window, GW_OWNER) }.ok();
    for _ in 0..8 {
        let Some(current) = owner else {
            return false;
        };
        if current.is_invalid() {
            return false;
        }
        if current == expected {
            return true;
        }
        owner = unsafe { GetWindow(current, GW_OWNER) }.ok();
    }
    false
}

struct ModalSearch {
    owner: HWND,
    host: HWND,
    process_id: u32,
    found: Option<HWND>,
}

fn find_modal_sibling(owner: HWND, host: HWND) -> Option<HWND> {
    let mut search = ModalSearch {
        owner,
        host,
        process_id: std::process::id(),
        found: None,
    };
    unsafe {
        let _ = EnumWindows(
            Some(find_modal_proc),
            LPARAM(&mut search as *mut ModalSearch as isize),
        );
    }
    search.found
}

unsafe extern "system" fn find_modal_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let search = unsafe { &mut *(lparam.0 as *mut ModalSearch) };
    if hwnd == search.owner || hwnd == search.host {
        return BOOL(1);
    }
    if !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return BOOL(1);
    }
    let mut process_id = 0;
    unsafe {
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    if process_id != search.process_id {
        return BOOL(1);
    }
    let style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
    if style & WS_EX_DLGMODALFRAME.0 == 0 || !owner_chain_contains(hwnd, search.owner) {
        return BOOL(1);
    }
    search.found = Some(hwnd);
    BOOL(0)
}

fn ensure_ole() -> Result<(), String> {
    OLE_READY
        .get_or_init(|| {
            unsafe { OleInitialize(None) }
                .map_err(|error| format!("OLE initialization failed: {error}"))
        })
        .clone()
}

fn ensure_atl() -> Result<(), String> {
    if *ATL_READY.get_or_init(|| {
        let Ok(module) = (unsafe { LoadLibraryW(w!("atl.dll")) }) else {
            return false;
        };
        let Some(init) = (unsafe { GetProcAddress(module, s!("AtlAxWinInit")) }) else {
            return false;
        };
        let init: unsafe extern "system" fn() -> i32 = unsafe { std::mem::transmute(init) };
        unsafe { init() != 0 }
    }) {
        Ok(())
    } else {
        Err("Windows ActiveX hosting (atl.dll) is unavailable".to_string())
    }
}

unsafe fn control_from_window(hwnd: HWND) -> Result<IDispatch, String> {
    let module = unsafe { LoadLibraryW(w!("atl.dll")) }
        .map_err(|error| format!("cannot load atl.dll: {error}"))?;
    let get_control = unsafe { GetProcAddress(module, s!("AtlAxGetControl")) }
        .ok_or_else(|| "AtlAxGetControl is unavailable".to_string())?;
    let get_control: unsafe extern "system" fn(HWND, *mut *mut c_void) -> i32 =
        unsafe { std::mem::transmute(get_control) };
    let mut unknown = std::ptr::null_mut();
    let result = unsafe { get_control(hwnd, &mut unknown) };
    if result < 0 || unknown.is_null() {
        return Err(format!(
            "cannot acquire the Windows RDP control (0x{result:08X})"
        ));
    }
    let unknown = unsafe { IUnknown::from_raw(unknown) };
    unknown
        .cast::<IDispatch>()
        .map_err(|error| format!("the Windows RDP control has no automation interface: {error}"))
}

fn authentication_level(policy: RdpCertificatePolicy) -> i32 {
    match policy {
        RdpCertificatePolicy::Insecure => 0,
        RdpCertificatePolicy::Strict | RdpCertificatePolicy::RejectChanged => 1,
        RdpCertificatePolicy::Prompt | RdpCertificatePolicy::TrustOnFirstUse => 2,
    }
}

fn performance_flags(display: &RdpDisplayConfig) -> u32 {
    let mut flags = 0;
    if !display.display_wallpaper {
        flags |= 0x0000_0001;
    }
    if display.disable_full_window_drag {
        flags |= 0x0000_0002;
    }
    if display.disable_menu_animations {
        flags |= 0x0000_0004;
    }
    if !display.display_themes {
        flags |= 0x0000_0008;
    }
    if display.disable_cursor_shadow {
        flags |= 0x0000_0020;
    }
    if display.disable_cursor_blinking {
        flags |= 0x0000_0040;
    }
    if display.enable_font_smoothing {
        flags |= 0x0000_0080;
    }
    if display.enable_desktop_composition {
        flags |= 0x0000_0100;
    }
    flags
}

fn display_size(rect: NativeRdpRect) -> (u32, u32) {
    let dim = |value: i32| (value.clamp(200, 8192) as u32) & !1;
    (dim(rect.width), dim(rect.height))
}

fn update_session_display_settings(
    control: &IDispatch,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let dispid = get_disp_id(control, "UpdateSessionDisplaySettings")?;
    // IDispatch arguments are reversed: width, height, physical width/height,
    // orientation, desktop scale, device scale.
    let mut arguments = [
        VARIANT::from(100u32),
        VARIANT::from(100u32),
        VARIANT::from(0u32),
        VARIANT::from(0u32),
        VARIANT::from(0u32),
        VARIANT::from(height),
        VARIANT::from(width),
    ];
    let params = DISPPARAMS {
        rgvarg: arguments.as_mut_ptr(),
        rgdispidNamedArgs: std::ptr::null_mut(),
        cArgs: arguments.len() as u32,
        cNamedArgs: 0,
    };
    unsafe {
        control
            .Invoke(
                dispid,
                &GUID::zeroed(),
                0,
                DISPATCH_METHOD,
                &params,
                None,
                None,
                None,
            )
            .map_err(|error| error.to_string())
    }
}

fn get_disp_id(control: &IDispatch, name: &str) -> Result<i32, String> {
    let wide = HSTRING::from(name);
    let names = [PCWSTR(wide.as_ptr())];
    let mut dispid = 0;
    unsafe {
        control
            .GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, 0, &mut dispid)
            .map_err(|error| format!("RDP control has no property {name}: {error}"))?;
    }
    Ok(dispid)
}

fn get_property(control: &IDispatch, name: &str) -> Result<VARIANT, String> {
    let dispid = get_disp_id(control, name)?;
    let params = DISPPARAMS::default();
    let mut result = VARIANT::default();
    unsafe {
        control
            .Invoke(
                dispid,
                &GUID::zeroed(),
                0,
                DISPATCH_PROPERTYGET,
                &params,
                Some(&mut result),
                None,
                None,
            )
            .map_err(|error| format!("cannot read RDP property {name}: {error}"))?;
    }
    Ok(result)
}

fn get_dispatch(control: &IDispatch, name: &str) -> Result<IDispatch, String> {
    let value = get_property(control, name)?;
    IDispatch::try_from(&value)
        .map_err(|error| format!("RDP property {name} is not an automation object: {error}"))
}

fn get_bool(control: &IDispatch, name: &str) -> Result<bool, String> {
    let value = get_property(control, name)?;
    bool::try_from(&value).map_err(|error| format!("RDP property {name} is not a boolean: {error}"))
}

fn get_u32(control: &IDispatch, name: &str) -> Result<u32, String> {
    let value = get_property(control, name)?;
    i32::try_from(&value)
        .map(|value| value as u32)
        .map_err(|error| format!("RDP property {name} is not a number: {error}"))
}

fn get_text(control: &IDispatch, name: &str) -> Result<String, String> {
    let value = get_property(control, name)?;
    BSTR::try_from(&value)
        .map(|value| value.to_string())
        .map_err(|error| format!("RDP property {name} is not text: {error}"))
}

fn put_bstr(control: &IDispatch, name: &str, value: &str) -> Result<(), String> {
    put_property(control, name, VARIANT::from(BSTR::from(value)))
}

fn put_i32(control: &IDispatch, name: &str, value: i32) -> Result<(), String> {
    put_property(control, name, VARIANT::from(value))
}

fn put_bool(control: &IDispatch, name: &str, value: bool) -> Result<(), String> {
    put_property(control, name, VARIANT::from(value))
}

fn put_property(control: &IDispatch, name: &str, value: VARIANT) -> Result<(), String> {
    let dispid = get_disp_id(control, name)?;
    put_variant(control, dispid, value)
}

fn put_variant(control: &IDispatch, dispid: i32, value: VARIANT) -> Result<(), String> {
    let mut arguments = [value];
    let mut names = [DISPID_PROPERTYPUT];
    let params = DISPPARAMS {
        rgvarg: arguments.as_mut_ptr(),
        rgdispidNamedArgs: names.as_mut_ptr(),
        cArgs: 1,
        cNamedArgs: 1,
    };
    unsafe {
        control
            .Invoke(
                dispid,
                &GUID::zeroed(),
                0,
                DISPATCH_PROPERTYPUT,
                &params,
                None,
                None,
                None,
            )
            .map_err(|error| format!("cannot set RDP property {dispid}: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use nyaterm_remote_desktop::{RdpCertificatePolicy, RdpDisplayConfig, RdpDisplayMode};

    use super::{NativeRdpRect, authentication_level, display_size, performance_flags};

    #[test]
    fn display_settings_become_the_rdp_performance_flag_mask() {
        let mut display = RdpDisplayConfig {
            mode: RdpDisplayMode::Fixed,
            width: 1280,
            height: 720,
            ..Default::default()
        };
        assert_eq!(performance_flags(&display), 0x86);

        display.display_wallpaper = false;
        display.display_themes = false;
        display.enable_font_smoothing = false;
        display.disable_cursor_shadow = true;
        display.disable_cursor_blinking = true;
        assert_eq!(performance_flags(&display), 0x6f);
    }

    #[test]
    fn resized_desktops_are_even_and_bounded() {
        assert_eq!(
            display_size(NativeRdpRect {
                x: 0,
                y: 0,
                width: 1279,
                height: 9001,
            }),
            (1278, 8192)
        );
    }

    #[test]
    fn certificate_policy_selects_the_matching_authentication_level() {
        assert_eq!(authentication_level(RdpCertificatePolicy::Insecure), 0);
        assert_eq!(authentication_level(RdpCertificatePolicy::Prompt), 2);
        assert_eq!(
            authentication_level(RdpCertificatePolicy::TrustOnFirstUse),
            2
        );
        assert_eq!(authentication_level(RdpCertificatePolicy::Strict), 1);
        assert_eq!(authentication_level(RdpCertificatePolicy::RejectChanged), 1);
    }
}
