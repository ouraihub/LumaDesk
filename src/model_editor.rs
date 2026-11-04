//! Typed model editor; saving and the surrounding dialog belong to ModelSettings.
use std::{collections::BTreeMap, fmt::Display, str::FromStr};
use gpui_kit::{div, prelude::*, ClipboardItem, Context, Entity, IntoElement, Render, Subscription, Window};
use gpui_kit::component::{button::{Button, ButtonVariants}, input::{Input, InputEvent, InputState}, Disableable, Selectable, Sizable};
use lumakit_design::{LumaTheme, Roles};
use lumakit_design::components::label;
use lumakit_design::metrics::{radius, space, text as styles};
use lumakit_ai::{Protocol, catalog::ModelEntry, model_config::{ModelType, ReasoningConfig, ReasoningDialect, ReasoningEffort, ModelPricing, PriceTier}};

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
fn fits(dialect: ReasoningDialect, protocol: Protocol) -> bool {
    match dialect {
        ReasoningDialect::AnthropicBudget | ReasoningDialect::AnthropicAdaptive => protocol == Protocol::AnthropicMessages,
        ReasoningDialect::OpenAi => protocol != Protocol::AnthropicMessages,
        _ => protocol == Protocol::OpenAiChat,
    }
}
fn auto(dialect: ReasoningDialect) -> bool { matches!(dialect, ReasoningDialect::ThinkingType | ReasoningDialect::Qwen) }
fn reasoning(dialect: ReasoningDialect) -> ReasoningConfig {
    ReasoningConfig { dialect, efforts: if auto(dialect) { vec![ReasoningEffort::None, ReasoningEffort::Auto] }
        else { vec![ReasoningEffort::Low, ReasoningEffort::Medium, ReasoningEffort::High] },
        budget_tokens: if dialect == ReasoningDialect::AnthropicBudget { Some(1024) } else { None } }
}
fn set_type(value: &mut ModelEntry, kind: ModelType, state: &mut EditorState) {
    if value.settings.model_type == kind { return; }
    value.settings.model_type = kind;
    if kind != ModelType::Chat {
        value.settings.reasoning = None; value.settings.native_web_search = false;
        value.settings.pricing = None; value.settings.supports_tools = None;
        state.clear("reasoning"); state.clear("pricing");
    }
}
fn tier(start: u32) -> PriceTier { PriceTier { min_input_tokens: start, input: 0.0, output: 0.0, cache_read: None, cache_write: None } }
pub(crate) const MODEL_TYPES: [ModelType; 8] = [ModelType::Chat, ModelType::Image, ModelType::Embedding, ModelType::Rerank, ModelType::Video, ModelType::Tts, ModelType::Stt, ModelType::Unknown];

pub(crate) struct ModelEditor {
    pub value: ModelEntry,
    protocol: Protocol,
    existing: bool,
    state: EditorState,
    inputs: BTreeMap<String, (Entity<InputState>, Subscription)>,
    more: bool,
}
impl ModelEditor {
    pub fn new(value: ModelEntry, protocol: Protocol, existing: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self { value, protocol, existing, state: EditorState::default(), inputs: BTreeMap::new(), more: true };
        this.prepare_inputs(window, cx); this
    }
    pub fn validate(&self) -> Result<(), String> {
        let id = self.value.id.trim();
        if id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) { return Err("模型 ID 须为 1–512 字节且不含控制字符".into()); }
        if let Some(error) = self.state.errors.values().next() { return Err(error.clone()); }
        self.value.settings.validate(self.protocol)
    }
    fn prepare_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut values = vec![("id".into(), self.value.id.clone()), ("name".into(), self.value.settings.name.clone().unwrap_or_default()),
            ("group".into(), self.value.settings.group_name.clone().unwrap_or_default()),
            ("context".into(), text(self.value.settings.context_window)), ("output".into(), text(self.value.settings.max_output))];
        if let Some(reasoning) = &self.value.settings.reasoning { values.push(("reasoning.budget".into(), text(reasoning.budget_tokens))); }
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
            let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
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
            "output" => parse(&value, &mut self.value.settings.max_output, false, |v| v > 0),
            "reasoning.budget" => if let Some(config) = &mut self.value.settings.reasoning {
                let required = config.dialect == ReasoningDialect::AnthropicBudget;
                parse(&value, &mut config.budget_tokens, required, |v| v >= if required { 1024 } else { 1 })
            } else { Ok(()) },
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
    fn field(&self, key: &str, label: &str, disabled: bool) -> gpui_kit::Div {
        div().flex().flex_col().gap(space::S4).child(group_label(label.to_owned())).child(Input::new(&self.inputs[key].0).disabled(disabled))
    }
    fn action(&self, id: impl Into<gpui_kit::ElementId>, label: impl Into<gpui_kit::SharedString>, selected: bool, cx: &Context<Self>, action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> Button {
        Button::new(id).small().outline().label(label).selected(selected).on_click(cx.listener(move |this, _, window, cx| { action(this, window, cx); cx.notify(); }))
    }
    fn presets(&self, key: &'static str, values: &[(&'static str, u32)], cx: &Context<Self>) -> gpui_kit::Div {
        let mut row = div().flex().flex_wrap().gap(space::S4);
        for &(label, value) in values {
            row = row.child(self.action(format!("{key}-{value}"), label, false, cx, move |this, window, cx| {
                let text = value.to_string(); this.apply_text(key, text.clone());
                this.inputs[key].0.update(cx, |input, cx| input.set_value(text, window, cx));
            }));
        }
        row.child(self.action(format!("{key}-clear"), "清空", false, cx, move |this, window, cx| {
            this.apply_text(key, String::new()); this.inputs[key].0.update(cx, |input, cx| input.set_value("", window, cx));
        }))
    }
}
impl Render for ModelEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r: Roles = LumaTheme::global(cx).roles;
        let mut body = div().flex().flex_col().gap(space::S12).w_full()
            .child(self.field("id", "模型 ID", self.existing))
            .child(div().flex().child(self.action("copy-id", "复制模型 ID", false, cx, |this, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(this.value.id.clone())))))
            .child(self.field("name", "模型名称（留空使用 ID）", false)).child(self.field("group", "分组名称（留空自动分组）", false));
        let mut usage = div().flex().flex_wrap().gap(space::S4);
        for (label, kind, vision) in [("对话", ModelType::Chat, false), ("图像生成", ModelType::Image, false), ("图像编辑", ModelType::Image, true)] {
            usage = usage.child(self.action(label, label, self.value.settings.model_type == kind && (kind == ModelType::Chat || self.value.capabilities.vision == vision), cx, move |this, window, cx| {
                set_type(&mut this.value, kind, &mut this.state); if kind == ModelType::Image { this.value.capabilities.vision = vision; } this.prepare_inputs(window, cx);
            }));
        }
        body = body.child(usage).child(div().flex().child(self.action("more", if self.more { "收起更多设置" } else { "更多设置" }, self.more, cx, |this, _, _| this.more = !this.more)));
        if self.more {
            let mut kinds = div().flex().flex_wrap().gap(space::S4);
            for (index, kind) in MODEL_TYPES.into_iter().enumerate() {
                kinds = kinds.child(self.action(("kind", index), kind.label(), self.value.settings.model_type == kind, cx, move |this, window, cx| { set_type(&mut this.value, kind, &mut this.state); this.prepare_inputs(window, cx); }));
            }
            body = body.child(group_label("模型类型")).child(kinds);
            if self.value.settings.model_type == ModelType::Chat {
                let tools = self.value.settings.supports_tools;
                body = body.child(div().flex().flex_wrap().gap(space::S4)
                    .child(self.action("reasoning", "推理", self.value.settings.reasoning.is_some(), cx, |this, window, cx| {
                        this.value.settings.reasoning = if this.value.settings.reasoning.is_some() { None } else { Some(reasoning(if this.protocol == Protocol::AnthropicMessages { ReasoningDialect::AnthropicBudget } else { ReasoningDialect::OpenAi })) }; this.reset("reasoning", window, cx);
                    }))
                    .child(self.action("tools", match tools { None => "工具 · 未探测", Some(true) => "工具 · 支持", Some(false) => "工具 · 不支持" }, tools == Some(true), cx, |this, _, _| {
                        this.value.settings.supports_tools = match this.value.settings.supports_tools { None => Some(true), Some(true) => Some(false), Some(false) => None }; this.value.capabilities.tools = this.value.settings.supports_tools == Some(true);
                    }))
                    .child(self.action("vision", "视觉输入", self.value.capabilities.vision, cx, |this, _, _| this.value.capabilities.vision = !this.value.capabilities.vision)));
                if self.protocol != Protocol::OpenAiChat || self.value.settings.native_web_search {
                    body = body.child(div().flex().child(self.action("web", "原生联网（可能产生额外费用）", self.value.settings.native_web_search, cx, |this, _, _| this.value.settings.native_web_search = !this.value.settings.native_web_search)));
                }
                if let Some(config) = &self.value.settings.reasoning {
                    let mut dialects = div().flex().flex_wrap().gap(space::S4);
                    for (index, (dialect, label)) in [(ReasoningDialect::OpenAi, "OpenAI"), (ReasoningDialect::AnthropicBudget, "Anthropic 预算"), (ReasoningDialect::AnthropicAdaptive, "Anthropic 自适应"), (ReasoningDialect::ThinkingType, "Thinking type"), (ReasoningDialect::Qwen, "Qwen")].into_iter().enumerate() {
                        if fits(dialect, self.protocol) { dialects = dialects.child(self.action(("dialect", index), label, config.dialect == dialect, cx, move |this, window, cx| { this.value.settings.reasoning = Some(reasoning(dialect)); this.reset("reasoning", window, cx); })); }
                    }
                    let mut efforts = div().flex().flex_wrap().gap(space::S4);
                    for (index, (effort, label)) in [(ReasoningEffort::None, "none"), (ReasoningEffort::Auto, "auto"), (ReasoningEffort::Minimal, "minimal"), (ReasoningEffort::Low, "low"), (ReasoningEffort::Medium, "medium"), (ReasoningEffort::High, "high"), (ReasoningEffort::Xhigh, "xhigh"), (ReasoningEffort::Max, "max")].into_iter().enumerate() {
                        if effort == ReasoningEffort::Auto && !auto(config.dialect) { continue; }
                        efforts = efforts.child(self.action(("effort", index), label, config.efforts.contains(&effort), cx, move |this, _, _| { if let Some(config) = &mut this.value.settings.reasoning { if config.efforts.contains(&effort) { config.efforts.retain(|e| *e != effort); } else { config.efforts.push(effort); } } }));
                    }
                    body = body.child(group_label("推理方言")).child(dialects).child(group_label("允许的推理强度（至少一项；首项为默认，重新选择移到末尾）")).child(efforts);
                    if matches!(config.dialect, ReasoningDialect::AnthropicBudget | ReasoningDialect::Qwen) { body = body.child(self.field("reasoning.budget", "推理 token 预算", false)); }
                }
            }
            body = body.child(div().flex().child(self.action("streaming", "支持流式输出", self.value.capabilities.streaming, cx, |this, _, _| { this.value.capabilities.streaming = !this.value.capabilities.streaming; this.value.settings.supports_streaming = this.value.capabilities.streaming; })))
                .child(self.field("context", "上下文窗口（token，留空为 32000）", false))
                .child(self.presets("context", &[("128K", 128000), ("200K", 200000), ("256K", 262144), ("400K", 400000), ("512K", 524288), ("1M", 1000000)], cx))
                .child(self.field("output", "最大输出（token，留空为 4096）", false))
                .child(self.presets("output", &[("16K", 16384), ("32K", 32768), ("64K", 65536), ("128K", 131072)], cx));
            if self.value.settings.model_type == ModelType::Chat {
                body = body.child(group_label("价格（每百万 token）"));
                if let Some(pricing) = &self.value.settings.pricing {
                    let mut currencies = div().flex().gap(space::S4);
                    for currency in ["USD", "CNY"] { currencies = currencies.child(self.action(currency, currency, pricing.currency == currency, cx, move |this, _, _| { if let Some(pricing) = &mut this.value.settings.pricing { pricing.currency = currency.into(); } })); }
                    body = body.child(currencies);
                    for (index, _) in pricing.tiers.iter().enumerate() {
                        let mut row = div().flex().flex_col().gap(space::S8).p(space::S12).border_1().border_color(r.border_muted).rounded(radius::MD).child(group_label(format!("第 {} 档", index + 1)));
                        for (field, label) in [("start", "起点（输入 token）"), ("input", "输入价格"), ("output", "输出价格"), ("read", "缓存读取（留空同输入）"), ("write", "缓存写入（留空同输入）")] { row = row.child(self.field(&format!("pricing.{index}.{field}"), label, index == 0 && field == "start")); }
                        row = row.child(div().flex().justify_end().child(self.action(("remove-tier", index), "删除此档", false, cx, move |this, window, cx| {
                            if let Some(pricing) = &mut this.value.settings.pricing { pricing.tiers.remove(index); if let Some(first) = pricing.tiers.first_mut() { first.min_input_tokens = 0; } else { this.value.settings.pricing = None; } }
                            this.state.remove_tier(index); this.inputs.retain(|key, _| !key.starts_with("pricing.")); this.prepare_inputs(window, cx);
                        }).danger())); body = body.child(row);
                    }
                    let next = pricing.tiers.last().map_or(Some(0), |p| p.min_input_tokens.checked_add(200000));
                    body = body.child(div().flex().gap(space::S4).child(self.action("add-tier", "添加价格档", false, cx, move |this, window, cx| {
                        if let (Some(pricing), Some(next)) = (&mut this.value.settings.pricing, next) { if pricing.tiers.len() < 8 { pricing.tiers.push(tier(next)); } } this.prepare_inputs(window, cx);
                    }).disabled(pricing.tiers.len() >= 8 || next.is_none()))
                    .child(self.action("clear-price", "清除价格", false, cx, |this, window, cx| { this.value.settings.pricing = None; this.reset("pricing", window, cx); })));
                } else { body = body.child(div().flex().child(self.action("add-price", "添加价格", false, cx, |this, window, cx| { this.value.settings.pricing = Some(ModelPricing { currency: "USD".into(), tiers: vec![tier(0)] }); this.reset("pricing", window, cx); }))); }
            }
        }
        if let Err(error) = self.validate() { body = body.child(label(styles::METADATA, error).text_color(r.feedback_error)); }
        body
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
    fn reasoning_defaults_fit_protocol_and_budget() {
        assert!(fits(ReasoningDialect::OpenAi, Protocol::OpenAiChat));
        assert!(!fits(ReasoningDialect::OpenAi, Protocol::AnthropicMessages));
        assert!(!fits(ReasoningDialect::Qwen, Protocol::OpenAiResponses));
        assert_eq!(reasoning(ReasoningDialect::AnthropicBudget).budget_tokens, Some(1024));
        let mut state = EditorState::default();
        state.text.insert("pricing.1.input".into(), "invalid".into()); state.errors.insert("pricing.1.input".into(), "invalid".into());
        state.remove_tier(0); assert_eq!(state.text.get("pricing.0.input").map(String::as_str), Some("invalid"));
        assert!(state.errors.contains_key("pricing.0.input")); assert!(reasoning(ReasoningDialect::Qwen).efforts.contains(&ReasoningEffort::Auto));
    }
    #[test]
    fn invalid_numeric_draft_preserves_saved_value() {
        let mut price = Some(2.0); assert!(parse("NaN", &mut price, true, valid_price).is_err()); assert_eq!(price, Some(2.0));
        assert!(parse("", &mut price, false, valid_price).is_ok()); assert_eq!(price, None);
        let mut tokens = Some(1024_u32); assert!(parse("-1", &mut tokens, true, |v| v >= 1024).is_err()); assert_eq!(tokens, Some(1024));
    }
}
