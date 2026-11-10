//! GPUI controls over the existing catalog, key and discovery operations, laid out as ThinkRail
//! Native's providers page: a provider rail, the selected provider's detail, and in-page dialogs.
use super::*;
use crate::model_editor::{MODEL_TYPES, ModelEditor};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    Disableable, FocusTrapElement, Icon, Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    switch::Switch,
};
use gpui_kit::{
    AnyElement, ClipboardItem, ElementId, FocusHandle, IntoElement, Pixels, Render, SharedString,
    Subscription, div, prelude::*, px,
};
use lumakit_design::components::settings as ui;
use lumakit_design::components::{IconButton, label};
use lumakit_design::metrics::{radius, space, text};
use lumakit_design::{LumaTheme, Roles};
use lumakit_gpui::providers as shared;

mod actions;
mod detail;
mod dialogs;
mod inputs;
mod keys;
mod models;
mod picker;
mod rail;

/// Dialog column width (ThinkRail's wide form dialogs).
const DIALOG_WIDTH: Pixels = px(640.);
/// Cherry's model side panel uses a compact one-column form.
const MODEL_EDITOR_WIDTH: Pixels = px(400.);
/// The dialog layer is at least this tall, so a short page still frames the dialog.
const DIALOG_MIN_HEIGHT: Pixels = px(560.);
/// Window width above which the provider rail sits beside the detail (settings nav + rail + detail).
const WIDE_PAGE: Pixels = px(880.);
const PROVIDER_FILTERS: [(u8, &str); 3] = [(0, "全部平台"), (1, "已启用"), (2, "已停用")];
const CAPABILITY_FILTERS: [(u8, &str); 5] = [
    (0, "全部"),
    (1, "视觉"),
    (2, "推理"),
    (3, "工具"),
    (4, "联网"),
];

fn roles(cx: &App) -> Roles {
    LumaTheme::global(cx).roles
}
/// A muted one-line hint (ThinkRail's subtle metadata line).
fn hint(content: impl Into<SharedString>, r: &Roles) -> gpui_kit::Div {
    label(text::METADATA, content).text_color(r.text_subtle)
}

pub(super) struct ModelView {
    inputs: HashMap<&'static str, Entity<InputState>>,
    _subscriptions: Vec<Subscription>,
    model: Option<Entity<ModelEditor>>,
    focus: FocusHandle,
    modal_open: bool,
    previous_focus: Option<FocusHandle>,
}
impl ModelSettings {
    pub fn create(root: &Path, window: &mut Window, cx: &mut App) -> Result<Entity<Self>, String> {
        let mut state = Self::load(root)?;
        Ok(cx.new(|cx| {
            state.init_view(window, cx);
            state
        }))
    }
    fn button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        self.on(Button::new(id).label(label), cx, action)
    }
    /// Attaches `action` to an already built button (icon buttons, rows) and repaints afterwards.
    fn on(
        &self,
        button: Button,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        button.on_click(cx.listener(move |this, _, window, cx| {
            action(this, window, cx);
            cx.notify();
        }))
    }
    /// An icon-only action that repaints after `action`.
    fn icon(
        &self,
        id: impl Into<ElementId>,
        icon: IconName,
        tooltip: &'static str,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> IconButton {
        IconButton::new(id, icon)
            .tooltip(tooltip)
            .on_click(cx.listener(move |this, _, window, cx| {
                action(this, window, cx);
                cx.notify();
            }))
    }
    /// The page notice and, while an address is unsaved, the retry entry.
    fn status(&self, cx: &Context<Self>) -> Option<gpui_kit::Div> {
        let r = roles(cx);
        let retry =
            self.dirty && self.editor.is_none() && self.sync_dialog.is_none() && self.job.is_none();
        if self.notice.is_empty() && !retry {
            return None;
        }
        let notice = (!self.notice.is_empty()).then(|| {
            if self.error {
                ui::notice(self.notice.clone(), ui::Tone::Error, &r)
            } else {
                hint(self.notice.clone(), &r).into_any_element()
            }
        });
        Some(
            h_flex()
                .flex_wrap()
                .gap(space::S8)
                .children(notice.map(|n| div().flex_1().min_w(px(0.)).child(n)))
                .when_some(
                    self.diagnostic_trace.clone().filter(|_| self.error),
                    |d, trace| {
                        d.child(
                            Button::new("copy-model-diagnostic")
                                .small()
                                .link()
                                .label("复制诊断 ID")
                                .accessibility_label(format!("复制诊断 ID {trace}"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(trace.clone()))
                                }),
                        )
                    },
                )
                .when(retry, |d| {
                    d.child(
                        self.button("retry-save", "重试保存修改", cx, |this, _, _| {
                            this.commit()
                        })
                        .link(),
                    )
                }),
        )
    }
    /// Provider rail and detail, side by side when wide. The page and the background behind a
    /// dialog both draw this, so they cannot drift apart.
    fn columns(&self, wide: bool, cx: &Context<Self>) -> gpui_kit::Div {
        shared::page(
            self.providers_view(cx).into_any_element(),
            self.detail_view(cx).into_any_element(),
            wide,
        )
    }
}
impl Render for ModelSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let modal = self.delete_provider.is_some()
            || self.delete_endpoint.is_some()
            || self.delete_key.is_some()
            || self.editor.is_some()
            || self.sync_dialog.is_some();
        let focus = self.view.as_ref().unwrap().focus.clone();
        let view = self.view.as_mut().unwrap();
        if modal && !view.modal_open {
            view.previous_focus = window.focused(cx);
            focus.focus(window, cx);
        } else if !modal && view.modal_open {
            if let Some(previous) = view.previous_focus.take() {
                previous.focus(window, cx);
            } else {
                view.inputs["provider-search"].update(cx, |input, cx| input.focus(window, cx));
            }
        }
        view.modal_open = modal;
        let r = roles(cx);
        let wide = window.viewport_size().width > WIDE_PAGE;
        let escape = cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                if this.delete_provider.is_some()
                    || this.delete_endpoint.is_some()
                    || this.delete_key.is_some()
                {
                    this.delete_provider = None;
                    this.delete_endpoint = None;
                    this.delete_key = None;
                } else if this.editor.is_some() {
                    this.close_editor(window, cx);
                } else if this.sync_dialog.is_some() {
                    this.sync_dialog = None;
                } else if let Some(job) = &this.job {
                    job.cancel.cancel();
                } else {
                    this.model_query.clear();
                    this.set_input("model-search", "", window, cx);
                }
                cx.notify();
            }
        });
        let header = ui::page_header("模型服务", None, &r);
        if !modal {
            return v_flex()
                .id("model-settings")
                .w_full()
                .gap(space::S16)
                .track_focus(&focus)
                .on_key_down(escape)
                .child(header)
                .child(self.columns(wide, cx))
                .into_any_element();
        }
        let content = if self.delete_provider.is_some()
            || self.delete_endpoint.is_some()
            || self.delete_key.is_some()
        {
            self.delete_view(cx)
        } else if self.editor.is_some() {
            self.editor_view(cx)
        } else {
            self.sync_view(cx)
        };
        let model_editor = matches!(self.editor, Some(EditorDialog::Model { .. }));
        let tall_dialog = model_editor || self.sync_dialog.is_some();
        let dialog = v_flex()
            .id("model-settings")
            .w_full()
            .when(tall_dialog, |d| d.h_full())
            .min_h(px(0.))
            .track_focus(&focus)
            .p(space::S12)
            .on_key_down(escape)
            .child(content);
        // Keep the dialog bounded when adding models grows the background page.
        div()
            .relative()
            .w_full()
            .h_full()
            .min_h(DIALOG_MIN_HEIGHT)
            .overflow_hidden()
            .child(
                v_flex()
                    .id("models-background")
                    .w_full()
                    .gap(space::S16)
                    .child(header)
                    .child(self.columns(wide, cx)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .flex()
                    .justify_center()
                    .items_start()
                    .p(space::S12)
                    .bg(r.overlay)
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(if model_editor {
                                MODEL_EDITOR_WIDTH
                            } else if matches!(self.editor, Some(EditorDialog::Keys { .. })) {
                                lumakit_design::metrics::size::SETTINGS_DIALOG
                            } else {
                                DIALOG_WIDTH
                            })
                            .when(tall_dialog, |d| d.h_full())
                            .min_h(px(0.))
                            .rounded(radius::LG)
                            .border_1()
                            .border_color(r.border_default)
                            .bg(r.container_elevated_bg)
                            .shadow_lg()
                            .child(dialog.focus_trap("model-settings-dialog", &focus)),
                    ),
            )
            .into_any_element()
    }
}
