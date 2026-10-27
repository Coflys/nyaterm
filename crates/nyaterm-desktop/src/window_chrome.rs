//! Derived window policy. GPUI remains the authority for decoration and capabilities.

use gpui::{Decorations, Window, WindowControls};
use nyaterm_ui::window_chrome::TitleDragBehavior;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Windows,
    MacOS,
    Linux,
}

impl Platform {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOS
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Windows
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowChrome {
    pub minimize: bool,
    pub maximize: bool,
    pub close: bool,
    pub restore: bool,
    pub native_button_gutter: bool,
    pub drag: TitleDragBehavior,
}

impl WindowChrome {
    pub fn main(window: &Window) -> Self {
        Self::for_window(window, false, true)
    }

    pub fn child(window: &Window, sheet: bool) -> Self {
        Self::for_window(window, sheet, false)
    }

    fn for_window(window: &Window, sheet: bool, main: bool) -> Self {
        Self::derive(WindowChromeInput {
            platform: Platform::current(),
            decorations: window.window_decorations(),
            controls: window.window_controls(),
            minimizable: window.is_minimizable(),
            resizable: window.is_resizable(),
            maximized: window.is_maximized(),
            fullscreen: window.is_fullscreen() || window.is_simple_fullscreen(),
            sheet,
            main,
        })
    }

    fn derive(input: WindowChromeInput) -> Self {
        let WindowChromeInput {
            platform,
            decorations,
            controls,
            minimizable,
            resizable,
            maximized,
            fullscreen,
            sheet,
            main,
        } = input;
        let buttons = !fullscreen
            && !sheet
            && match platform {
                Platform::MacOS => false,
                Platform::Windows => true,
                Platform::Linux => matches!(decorations, Decorations::Client { .. }),
            };
        Self {
            minimize: buttons && minimizable && controls.minimize,
            maximize: buttons && resizable && controls.maximize,
            close: buttons,
            restore: maximized,
            native_button_gutter: platform == Platform::MacOS && !sheet && !fullscreen,
            drag: if sheet || fullscreen {
                TitleDragBehavior::Disabled
            } else {
                match platform {
                    Platform::Linux => TitleDragBehavior::Linux {
                        can_maximize: resizable && controls.maximize,
                    },
                    Platform::MacOS if main => TitleDragBehavior::MacOS,
                    Platform::MacOS | Platform::Windows => TitleDragBehavior::Native,
                }
            },
        }
    }
}

struct WindowChromeInput {
    platform: Platform,
    decorations: Decorations,
    controls: WindowControls,
    minimizable: bool,
    resizable: bool,
    maximized: bool,
    fullscreen: bool,
    sheet: bool,
    main: bool,
}

#[cfg(test)]
mod tests;
