use gpui::{IntoElement, RenderOnce, SharedString};
use gpui_kit::component::text::TextView;

/// Stable NyaTerm boundary for selectable Markdown rendered by gpui-kit.
#[derive(IntoElement)]
pub struct NyaMarkdown {
    id: SharedString,
    source: SharedString,
}

impl NyaMarkdown {
    pub fn new(id: impl Into<SharedString>, source: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
        }
    }
}

impl RenderOnce for NyaMarkdown {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        TextView::markdown(self.id, self.source)
            .selectable(true)
            .on_link_click(|url, _, _, cx| {
                let Ok(parsed) = url::Url::parse(url) else {
                    return;
                };
                if matches!(parsed.scheme(), "http" | "https" | "mailto") {
                    cx.open_url(url.as_ref());
                }
            })
    }
}
