mod editor;
#[allow(dead_code)]
mod lsp_completion;
#[allow(dead_code)]
mod lsp_hover;
mod settings;
mod settings_page;
mod sidebar;
mod status_bar;
mod terminal;
mod terminal_container;
mod theme;
mod window;

use libadwaita as adw;
use libadwaita::prelude::*;

const APP_ID: &str = "dev.impulse.Impulse";

fn main() {
    env_logger::init();

    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_app| {
        let style_manager = adw::StyleManager::default();
        style_manager.set_color_scheme(adw::ColorScheme::ForceDark);
        let settings = settings::load();
        theme::load_css(theme::get_theme(&settings.color_scheme));
    });

    app.connect_activate(move |app| {
        window::build_window(app);
    });

    app.run();
}
