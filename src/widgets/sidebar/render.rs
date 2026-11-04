use gtk4::prelude::*;
use gtk4::{Box, Orientation, ScrolledWindow, Separator};

/// Builds the dock layout container for the sidebar matching Settings window dock style.
pub fn build_sidebar_ui() -> (Box, Box, ScrolledWindow, Box, Box) {
    let layout = babydra_ui_kit::components::SidebarLayout::new("explore-sidebar");
    let root = layout.widget();
    root.set_width_request(52);
    root.set_margin_start(6);
    root.set_margin_end(0);
    root.set_margin_top(8);
    root.set_margin_bottom(8);
    (
        layout.widget().clone(),
        layout.header().clone(),
        layout.scroll().clone(),
        layout.navigation().clone(),
        layout.footer().clone(),
    )
}

/// Helper to build a horizontal separator matching Settings sidebar.
pub fn create_sidebar_sep() -> Separator {
    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("sidebar-nav-separator");
    sep.set_margin_top(6);
    sep.set_margin_bottom(6);
    sep
}
