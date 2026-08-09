use gpui::{AnyElement, Context, ScrollHandle, Window, prelude::*};

use crate::SettingsWindow;

pub(crate) fn render_lnmarkets_settings_page(
    settings_window: &SettingsWindow,
    _scroll_handle: &ScrollHandle,
    _window: &mut Window,
    _cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    settings_window
        .lnmarkets_settings_page
        .clone()
        .into_any_element()
}
