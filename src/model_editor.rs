//! Typed model editor; saving and the surrounding dialog belong to ModelSettings.
use std::{collections::BTreeMap, fmt::Display, str::FromStr};
use gpui_kit::{div, prelude::*, Context, Entity, Subscription, Window};
use gpui_kit::component::{button::{Button, ButtonVariants}, input::{Input, InputEvent, InputState}, Sizable};
use gpui_kit::component::Selectable;
use lumakit_design::components::{label, settings::{select, select_button}};
use lumakit_design::metrics::{space, text as styles};

use lumakit_ai::{Protocol, catalog::ModelEntry, model_config::{ModelType, PriceTier}};
mod view;
mod pricing;

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
            *map = std::mem::take(map).into_iter().filter_map(|(key, value)| {
                let Some(rest) = key.strip_prefix("pricing.") else { return Some((key, value)); };
                let (index, field) = rest.split_once('.')?;
                let index: usize = index.parse().ok()?;
                if index == removed { None } else { Some((format!("pricing.{}.{}", if index > removed { index - 1 } else { index }, field), value)) }
            }).collect();
        }
        shift(&mut self.text, removed); shift(&mut self.errors, removed);
        self.text.remove("pricing.0.start"); self.errors.remove("pricing.0.start");
    }
}
#[derive(Clone, Copy)]
enum Capability { Reasoning, Tools, Vision, Audio, Video }

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
    if value.settings.model_type == kind { return; }
    value.settings.model_type = kind;
    if kind != ModelType::Image { value.settings.image_edit = false; }
    if kind != ModelType::Chat {
        value.settings.reasoning = None; value.settings.reasoning_disabled = false; value.settings.reasoning_override = None; value.settings.native_web_search = false;
        value.settings.pricing = None; value.settings.supports_tools = None;
        state.clear("reasoning"); state.clear("pricing");
    }
}
fn tier(start: u32) -> PriceTier { PriceTier { min_input_tokens: start, input: 0.0, output: 0.0, cache_read: None, cache_write: None } }
pub(crate) const MODEL_TYPES: [ModelType; 8] = [ModelType::Chat, ModelType::Image, ModelType::Video, ModelType::Tts, ModelType::Stt, ModelType::Embedding, ModelType::Rerank, ModelType::Unknown];

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
    pub fn new(value: ModelEntry, protocol: Protocol, existing: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self { value, protocol, existing, state: EditorState::default(), inputs: BTreeMap::new(), more: true, other_types: false };
        this.prepare_inputs(window, cx); this
    }
    pub fn validate(&self) -> Result<(), String> {
        let id = self.value.id.trim();
        if id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) { return Err("模型 ID 须为 1–512 字节且不含控制字符".into()); }
        if let Some(error) = self.state.errors.values().next() { return Err(error.clone()); }
        lumakit_ai::model_reference::effective_settings(&self.value.id, self.protocol, &self.value.settings).validate(self.protocol)
    }
    fn prepare_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut values = vec![("id".into(), self.value.id.clone()), ("name".into(), self.value.settings.name.clone().unwrap_or_default()),
            ("group".into(), self.value.settings.group_name.clone().unwrap_or_default()),
            ("context".into(), text(self.value.settings.context_window)), ("input".into(), text(self.value.settings.max_input)), ("output".into(), text(self.value.settings.max_output))];
        if let Some(pricing) = &self.value.settings.pricing {
            for (index, price) in pricing.tiers.iter().enumerate() {
                for (field, value) in [("start", price.min_input_tokens.to_string()), ("input", price.input.to_string()), ("output", price.output.to_string()), ("read", text(price.cache_read)), ("write", text(price.cache_write))] {
                    values.push((format!("pricing.{index}.{field}"), value));
                }
            }
        }
        self.inputs.retain(|key, _| values.iter().any(|(name, _)| name == key));
        for (key, initial) in values {
            if self.inputs.contains_key(&key) { continue; }
            let initial = self.state.text.entry(key.clone()).or_insert(initial).clone();
            let placeholder = match key.as_str() { "name" => "留空使用模型 ID", "group" => "留空自动分组", "context" => "自动使用来源参考或默认容量", "input" => "留空不单独限制输入", "output" => "留空使用默认输出预算", _ => "" };
            let input = cx.new(|cx| InputState::new(window, cx).default_value(initial).placeholder(placeholder));
            let name = key.clone();
            let subscription = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.apply_text(&name, value); cx.notify();
                }
            });
            self.inputs.insert(key, (input, subscription));
        }
    }
    fn reset(&mut self, prefix: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.state.clear(prefix); self.inputs.retain(|key, _| !key.starts_with(prefix)); self.prepare_inputs(window, cx);
    }
    fn apply_text(&mut self, key: &str, value: String) {
        self.state.text.insert(key.into(), value.clone());
        self.state.errors.remove(key);
        let result = match key {
            "id" => { if !self.existing { self.value.id = value; } Ok(()) },
            "name" => { self.value.settings.name = (!value.trim().is_empty()).then_some(value); Ok(()) },
            "group" => { self.value.settings.group_name = (!value.trim().is_empty()).then_some(value); Ok(()) },
            "context" => parse(&value, &mut self.value.settings.context_window, false, |v| v > 0),
            "input" => parse(&value, &mut self.value.settings.max_input, false, |v| v > 0),
            "output" => parse(&value, &mut self.value.settings.max_output, false, |v| v > 0),
            _ => {
                let result = key.strip_prefix("pricing.").and_then(|s| s.split_once('.')).and_then(|(index, field)| index.parse::<usize>().ok().map(|index| (index, field)));
                if let Some((index, field)) = result {
                    if let Some(price) = self.value.settings.pricing.as_mut().and_then(|p| p.tiers.get_mut(index)) {
                        match field {
                            "start" => { let mut target = Some(price.min_input_tokens); let result = parse(&value, &mut target, true, |_| true); if let Some(v) = target { price.min_input_tokens = v; } result },
                            "input" | "output" => { let target = if field == "input" { &mut price.input } else { &mut price.output }; let mut amount = Some(*target); let result = parse(&value, &mut amount, true, valid_price); if let Some(v) = amount { *target = v; } result },
                            "read" => parse(&value, &mut price.cache_read, false, valid_price),
                            "write" => parse(&value, &mut price.cache_write, false, valid_price),
                            _ => Ok(()),
                        }
                    } else { Ok(()) }
                } else { Ok(()) }
            }
        };
        if result.is_err() { self.state.errors.insert(key.into(), format!("{key}：请输入有效数字（预算/容量为正整数，价格为 0–1000000）")); }
    }
    fn field(&self, key: &str, name: &str, readonly: bool) -> gpui_kit::Div {
        div().flex().flex_col().min_w_0().gap(space::S4).child(group_label(name.to_owned()))
            .child(Input::new(&self.inputs[key].0).aria_label(name.to_owned()).readonly(readonly).w_full().min_w_0())
    }
    fn action(&self, id: impl Into<gpui_kit::ElementId>, name: impl Into<gpui_kit::SharedString>, selected: bool, cx: &Context<Self>, action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> Button {
        let name = name.into();
        Button::new(id).small().outline().label(name.clone()).accessibility_label(name).on_click(cx.listener(move |this, _, window, cx| { action(this, window, cx); cx.notify(); }))
            .when(selected, |button| button.bg(lumakit_design::LumaTheme::global(cx).roles.control_bg_selected))
    }
    /// A dropdown for one choice among `options`; `apply` writes the pick, then the editor re-renders.
    fn select<T: Copy + PartialEq + 'static>(&self, id: &'static str, name: &'static str, current: Option<T>, options: Vec<(T, &'static str)>, cx: &Context<Self>,
        apply: impl Fn(&mut Self, T, &mut Window, &mut Context<Self>) + 'static) -> gpui_kit::AnyElement {
        let shown = options.iter().find(|(value, _)| Some(*value) == current).map_or("请选择", |(_, title)| *title);
        let page = cx.entity();
        select(select_button(id, name, shown).w_full(), current, options.into_iter().map(|(value, title)| (value, title.into())),
            move |value, window, cx| page.update(cx, |this, cx| { apply(this, value, window, cx); cx.notify(); }))
    }
    fn presets(&self, key: &'static str, values: &[(&'static str, u32)], cx: &Context<Self>) -> gpui_kit::Div {
        let mut row = div().flex().flex_wrap().gap(space::S4);
        for &(label, value) in values {
            let selected = match key { "context" => self.value.settings.context_window, "input" => self.value.settings.max_input, _ => self.value.settings.max_output } == Some(value);
            row = row.child(self.action(format!("{key}-{value}"), label, selected, cx, move |this, window, cx| {
                let text = value.to_string(); this.apply_text(key, text.clone());
                this.inputs[key].0.update(cx, |input, cx| input.set_value(text, window, cx));
            }).selected(selected).toggled(selected).accessibility_label(format!("{}：{label}（{value} Token）", match key { "context" => "上下文窗口", "input" => "最大输入", _ => "最大输出" })));
        }
        row.child(self.action(format!("{key}-clear"), "自动", false, cx, move |this, window, cx| {
            this.apply_text(key, String::new()); this.inputs[key].0.update(cx, |input, cx| input.set_value("", window, cx));
        }).accessibility_label(format!("清空{}覆盖，恢复自动", match key { "context" => "上下文窗口", "input" => "最大输入", _ => "最大输出" })))
    }
}
/// Label over an option set or input inside the editor.
fn group_label(content: impl Into<gpui_kit::SharedString>) -> gpui_kit::Div { label(styles::TITLE_COMPACT, content) }
fn text<T: Display>(value: Option<T>) -> String { value.map(|v| v.to_string()).unwrap_or_default() }
fn valid_price(value: f64) -> bool { value.is_finite() && (0.0..=1e6).contains(&value) }
fn parse<T: Copy + FromStr>(text: &str, target: &mut Option<T>, required: bool, valid: impl Fn(T) -> bool) -> Result<(), ()> {
    let value = if text.trim().is_empty() && !required { None } else { Some(text.trim().parse::<T>().ok().filter(|v| valid(*v)).ok_or(())?) };
    *target = value; Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_numeric_draft_preserves_saved_value() {
        let mut price = Some(2.0); assert!(parse("NaN", &mut price, true, valid_price).is_err()); assert_eq!(price, Some(2.0));
        assert!(parse("", &mut price, false, valid_price).is_ok()); assert_eq!(price, None);
        let mut tokens = Some(1024_u32); assert!(parse("-1", &mut tokens, true, |v| v >= 1024).is_err()); assert_eq!(tokens, Some(1024));
        assert!(parse("", &mut tokens, false, |v| v > 0).is_ok()); assert_eq!(tokens, None);
        assert!(parse("0", &mut tokens, false, |v| v > 0).is_err()); assert_eq!(tokens, None);
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    #[test]
    fn capability_choices_override_catalog_without_selecting_request_effort() {
        let mut model = ModelEntry { id: "gpt-6.1-sol".into(), capabilities: Default::default(), manual: true, settings: Default::default() };
        for capability in [Capability::Reasoning, Capability::Tools, Capability::Vision, Capability::Audio, Capability::Video] {
            set_capability(&mut model, capability, false);
        }
        let off = model.effective_settings(Protocol::OpenAiChat);
        assert!(!off.supports_reasoning() && !off.supports_vision && !off.supports_audio && !off.supports_video);
        assert_eq!(off.supports_tools, Some(false));
        for capability in [Capability::Reasoning, Capability::Tools, Capability::Vision, Capability::Audio, Capability::Video] {
            set_capability(&mut model, capability, true);
        }
        let restored: ModelEntry = serde_json::from_str(&serde_json::to_string(&model).unwrap()).unwrap();
        let on = restored.effective_settings(Protocol::OpenAiChat);
        assert!(on.supports_reasoning() && on.supports_vision && on.supports_audio && on.supports_video);
        assert_eq!(on.supports_tools, Some(true));
        assert!(on.reasoning_effort.is_none());
        let mut unknown = model;
        unknown.id = "unknown-model".into();
        let declared = unknown.effective_settings(Protocol::OpenAiChat);
        assert!(declared.supports_reasoning());
        assert!(declared.reasoning.is_none()); // A capability declaration does not invent a wire protocol.
    }
}
