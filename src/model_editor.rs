//! Typed model editor; saving and the surrounding dialog belong to ModelSettings.
use gpui_kit::component::Selectable;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{Context, Entity, Subscription, Window, div, prelude::*};
use lumakit_design::components::{
    label,
    settings::{select, select_button},
};
use lumakit_design::metrics::{space, text as styles};
use std::{collections::BTreeMap, fmt::Display, str::FromStr};

use lumakit_ai::{
    Protocol,
    catalog::ModelEntry,
    model_config::{ModelType, PriceTier},
};
mod inputs;
mod pricing;
#[cfg(test)]
mod tests;
mod view;

#[derive(Default)]
pub(crate) struct EditorState {
    text: BTreeMap<String, String>,
    errors: BTreeMap<String, String>,
}
impl EditorState {
    fn clear(&mut self, prefix: &str) {
        self.text.retain(|key, _| !key.starts_with(prefix));
        self.errors.retain(|key, _| !key.starts_with(prefix));
    }
    fn remove_tier(&mut self, removed: usize) {
        fn shift(map: &mut BTreeMap<String, String>, removed: usize) {
            *map = std::mem::take(map)
                .into_iter()
                .filter_map(|(key, value)| {
                    let Some(rest) = key.strip_prefix("pricing.") else {
                        return Some((key, value));
                    };
                    let (index, field) = rest.split_once('.')?;
                    let index: usize = index.parse().ok()?;
                    if index == removed {
                        None
                    } else {
                        Some((
                            format!(
                                "pricing.{}.{}",
                                if index > removed { index - 1 } else { index },
                                field
                            ),
                            value,
                        ))
                    }
                })
                .collect();
        }
        shift(&mut self.text, removed);
        shift(&mut self.errors, removed);
        self.text.remove("pricing.0.start");
        self.errors.remove("pricing.0.start");
    }
}
#[derive(Clone, Copy)]
enum Capability {
    Reasoning,
    Tools,
    Vision,
    Audio,
    Video,
}

fn set_capability(value: &mut ModelEntry, capability: Capability, enabled: bool) {
    match capability {
        Capability::Reasoning => {
            value.settings.reasoning_override = Some(enabled);
            value.settings.reasoning_disabled = !enabled;
        }
        Capability::Tools => {
            value.settings.supports_tools = Some(enabled);
            value.capabilities.tools = enabled;
        }
        Capability::Vision => {
            value.settings.vision_override = Some(enabled);
            value.settings.supports_vision = enabled;
            value.capabilities.vision = enabled;
        }
        Capability::Audio => value.settings.supports_audio = enabled,
        Capability::Video => value.settings.supports_video = enabled,
    }
}
fn set_type(value: &mut ModelEntry, kind: ModelType, state: &mut EditorState) {
    if value.settings.model_type == kind {
        return;
    }
    value.settings.model_type = kind;
    if kind != ModelType::Image {
        value.settings.image_edit = false;
    }
    if kind != ModelType::Chat {
        value.settings.reasoning = None;
        value.settings.reasoning_disabled = false;
        value.settings.reasoning_override = None;
        value.settings.native_web_search = false;
        value.settings.pricing = None;
        value.settings.supports_tools = None;
        state.clear("reasoning");
        state.clear("pricing");
    }
}
fn tier(start: u32) -> PriceTier {
    PriceTier {
        min_input_tokens: start,
        input: 0.0,
        output: 0.0,
        cache_read: None,
        cache_write: None,
    }
}
pub(crate) const MODEL_TYPES: [ModelType; 8] = [
    ModelType::Chat,
    ModelType::Image,
    ModelType::Video,
    ModelType::Tts,
    ModelType::Stt,
    ModelType::Embedding,
    ModelType::Rerank,
    ModelType::Unknown,
];

pub(crate) struct ModelEditor {
    pub value: ModelEntry,
    protocol: Protocol,
    existing: bool,
    state: EditorState,
    inputs: BTreeMap<String, (Entity<InputState>, Subscription)>,
    more: bool,
    other_types: bool,
}
impl ModelEditor {
    pub fn new(
        value: ModelEntry,
        protocol: Protocol,
        existing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            value,
            protocol,
            existing,
            state: EditorState::default(),
            inputs: BTreeMap::new(),
            more: true,
            other_types: false,
        };
        this.prepare_inputs(window, cx);
        this
    }
    pub fn validate(&self) -> Result<(), String> {
        let id = self.value.id.trim();
        if id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) {
            return Err("模型 ID 须为 1–512 字节且不含控制字符".into());
        }
        if let Some(error) = self.state.errors.values().next() {
            return Err(error.clone());
        }
        lumakit_ai::model_reference::effective_settings(
            &self.value.id,
            self.protocol,
            &self.value.settings,
        )
        .validate(self.protocol)
    }
    fn field(&self, key: &str, name: &str, readonly: bool, cx: &Context<Self>) -> gpui_kit::Div {
        lumakit_gpui::providers::field(
            name.to_owned(),
            Input::new(&self.inputs[key].0)
                .aria_label(name.to_owned())
                .readonly(readonly)
                .w_full()
                .min_w_0(),
            &lumakit_design::LumaTheme::global(cx).roles,
        )
    }
    fn action(
        &self,
        id: impl Into<gpui_kit::ElementId>,
        name: impl Into<gpui_kit::SharedString>,
        selected: bool,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        let name = name.into();
        Button::new(id)
            .small()
            .outline()
            .label(name.clone())
            .accessibility_label(name)
            .on_click(cx.listener(move |this, _, window, cx| {
                action(this, window, cx);
                cx.notify();
            }))
            .when(selected, |button| {
                button.bg(lumakit_design::LumaTheme::global(cx)
                    .roles
                    .control_bg_selected)
            })
    }
    /// A dropdown for one choice among `options`; `apply` writes the pick, then the editor re-renders.
    fn select<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        name: &'static str,
        current: Option<T>,
        options: Vec<(T, &'static str)>,
        cx: &Context<Self>,
        apply: impl Fn(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) -> gpui_kit::AnyElement {
        let shown = options
            .iter()
            .find(|(value, _)| Some(*value) == current)
            .map_or("请选择", |(_, title)| *title);
        let page = cx.entity();
        select(
            select_button(id, name, shown).w_full(),
            current,
            options
                .into_iter()
                .map(|(value, title)| (value, title.into())),
            move |value, window, cx| {
                page.update(cx, |this, cx| {
                    apply(this, value, window, cx);
                    cx.notify();
                })
            },
        )
    }
    fn presets(
        &self,
        key: &'static str,
        values: &[(&'static str, u32)],
        cx: &Context<Self>,
    ) -> gpui_kit::Div {
        let mut row = div().flex().flex_wrap().gap(space::S4);
        for &(label, value) in values {
            let selected = match key {
                "context" => self.value.settings.context_window,
                "input" => self.value.settings.max_input,
                _ => self.value.settings.max_output,
            } == Some(value);
            row = row.child(
                self.action(
                    format!("{key}-{value}"),
                    label,
                    selected,
                    cx,
                    move |this, window, cx| {
                        let text = value.to_string();
                        this.apply_text(key, text.clone());
                        this.inputs[key]
                            .0
                            .update(cx, |input, cx| input.set_value(text, window, cx));
                    },
                )
                .selected(selected)
                .toggled(selected)
                .accessibility_label(format!(
                    "{}：{label}（{value} Token）",
                    match key {
                        "context" => "上下文窗口",
                        "input" => "最大输入",
                        _ => "最大输出",
                    }
                )),
            );
        }
        row.child(
            self.action(
                format!("{key}-clear"),
                "自动",
                false,
                cx,
                move |this, window, cx| {
                    this.apply_text(key, String::new());
                    this.inputs[key]
                        .0
                        .update(cx, |input, cx| input.set_value("", window, cx));
                },
            )
            .accessibility_label(format!(
                "清空{}覆盖，恢复自动",
                match key {
                    "context" => "上下文窗口",
                    "input" => "最大输入",
                    _ => "最大输出",
                }
            )),
        )
    }
}
/// Label over an option set or input inside the editor.
fn group_label(content: impl Into<gpui_kit::SharedString>) -> gpui_kit::Div {
    label(styles::TITLE_COMPACT, content)
}
fn text<T: Display>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}
fn valid_price(value: f64) -> bool {
    value.is_finite() && (0.0..=1e6).contains(&value)
}
fn parse<T: Copy + FromStr>(
    text: &str,
    target: &mut Option<T>,
    required: bool,
    valid: impl Fn(T) -> bool,
) -> Result<(), ()> {
    let value = if text.trim().is_empty() && !required {
        None
    } else {
        Some(
            text.trim()
                .parse::<T>()
                .ok()
                .filter(|v| valid(*v))
                .ok_or(())?,
        )
    };
    *target = value;
    Ok(())
}
