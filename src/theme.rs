const EXPLORE_CSS: &str = concat!(
    include_str!("assets/styles/explore/window.css"), "\n",
    include_str!("assets/styles/explore/header_bar.css"), "\n",
    include_str!("assets/styles/explore/content_view.css"), "\n",
    include_str!("assets/styles/explore/info_panel.css"), "\n",
    include_str!("assets/styles/explore/status_bar.css"), "\n",
    include_str!("assets/styles/explore/context_menu.css"), "\n",
    include_str!("assets/styles/explore/dialogs.css"),
);

pub fn init_theme() {
    babydra_ui_kit::ui::theme::init_theme_with_css(EXPLORE_CSS);
}
