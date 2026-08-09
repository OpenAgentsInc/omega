use gpui::{AnyElement, Context, ScrollHandle, Window, prelude::*};

use crate::SettingsWindow;

/// Render the plugin-registered settings page for the sub-page currently on
/// top of the stack. One render function serves every registered page: the
/// active sub-page link's key selects the page view built at window
/// construction from the plugin registry.
pub(crate) fn render_plugin_settings_page(
    settings_window: &SettingsWindow,
    _scroll_handle: &ScrollHandle,
    _window: &mut Window,
    _cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let active_page_key = settings_window
        .sub_page_stack
        .last()
        .and_then(|sub_page| sub_page.link.json_path);
    settings_window
        .plugin_settings_pages
        .iter()
        .find(|(page_key, _)| Some(*page_key) == active_page_key)
        .map(|(_, page)| page.clone().into_any_element())
        .unwrap_or_else(|| gpui::Empty.into_any_element())
}
