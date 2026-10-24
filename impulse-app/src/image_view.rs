//! An image tab (the Swift `TabManager.makeImagePreview`): the picture
//! scaled down to fit with a 20pt margin, centered on the theme's
//! background, never scaled up.

use std::path::PathBuf;

use gpui_kit::{div, img, prelude::*, px, App, Context, FocusHandle, ObjectFit, Window};

/// Files that open as images rather than in the editor.
pub fn is_image_file(path: &std::path::Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "tif"
    ) && (!path.exists() || path.is_file())
}

pub struct ImageView {
    pub path: PathBuf,
    background: gpui_kit::Hsla,
    focus: FocusHandle,
}

impl ImageView {
    pub fn new(path: PathBuf, background: gpui_kit::Hsla, cx: &mut Context<Self>) -> Self {
        ImageView {
            path,
            background,
            focus: cx.focus_handle(),
        }
    }

    pub fn title(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn set_background(&mut self, background: gpui_kit::Hsla, cx: &mut Context<Self>) {
        self.background = background;
        cx.notify();
    }
}

impl Render for ImageView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .size_full()
            .p(px(20.))
            .bg(self.background)
            .child(
                img(self.path.clone())
                    .size_full()
                    .object_fit(ObjectFit::ScaleDown),
            )
    }
}

impl gpui_kit::Focusable for ImageView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
