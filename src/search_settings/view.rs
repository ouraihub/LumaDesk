//! AutoNote-shaped provider cards, shared key controls and advanced search tools.
use super::*;
use catalog::{FETCH_PROVIDERS, SEARCH_PROVIDERS};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    Disableable, FocusTrapElement, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, Textarea},
};
use gpui_kit::{AnyElement, ClipboardItem, IntoElement, Render, div, prelude::*, px};
use lumakit_design::{
    components::{label, settings as ui},
    metrics::{radius, size, space, text},
};

const PROVIDER_WIDTH: gpui_kit::Pixels = px(136.);
const BLACKLIST_HEIGHT: gpui_kit::Pixels = px(96.);
const RESULT_HEIGHT: gpui_kit::Pixels = px(320.);
fn card() -> gpui_kit::Div {
    v_flex()
        .w_full()
        .min_w_0()
        .gap(space::S12)
        .p(space::S16)
        .rounded(radius::LG)
}
fn field(name: &'static str, input: impl IntoElement, cx: &gpui_kit::App) -> gpui_kit::Div {
    ui::field(name, input, &lumakit_design::LumaTheme::global(cx).roles)
}
mod advanced;
mod key_dialog;
mod keys;
mod providers;

impl Render for SearchSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = lumakit_design::LumaTheme::global(cx).roles;
        let page = v_flex()
            .id("search-settings")
            .w_full()
            .max_w(size::CONTENT_MAX)
            .min_w_0()
            .gap(space::S16)
            .child(ui::page_header("网络搜索", None, &r))
            .child(self.provider_ui(false, cx))
            .child(self.provider_ui(true, cx))
            .child(self.advanced_view(cx))
            .child(self.tools_view(cx));
        if let Some(index) = self.key_dialog {
            page.child(self.key_dialog_view(index, window, cx))
                .into_any_element()
        } else {
            page.into_any_element()
        }
    }
}
