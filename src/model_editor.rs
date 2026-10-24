//! 仅绘制模型字段；弹窗、滚动区域和保存操作由调用方管理。
use std::{collections::BTreeMap, fmt::Display, str::FromStr};
use eframe::egui;
use lumakit_ai::{Protocol, catalog::ModelEntry, model_config::{ModelType, ReasoningConfig, ReasoningDialect, ReasoningEffort, ModelPricing, PriceTier}};
use crate::theme::tokens as t;

/// 每次打开模型时新建状态，包括重新打开尚未保存的模型，避免草稿串用。
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
                if index == removed { None }
                else { Some((format!("pricing.{}.{}", if index > removed { index - 1 } else { index }, field), value)) }
            }).collect();
        }
        shift(&mut self.text, removed);
        shift(&mut self.errors, removed);
        self.text.remove("pricing.0.start");
        self.errors.remove("pricing.0.start");
    }
}

fn error(ui: &mut egui::Ui, message: &str) {
    ui.colored_label(ui.visuals().error_fg_color, message);
}

fn chip(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let text = egui::RichText::new(label).color(if selected { ui.visuals().hyperlink_color } else { ui.visuals().text_color() });
    let mut button = egui::Button::new(text).corner_radius(t::RADIUS);
    if selected {
        button = button.fill(ui.visuals().hyperlink_color.gamma_multiply(0.10))
            .stroke(egui::Stroke::new(1.0, ui.visuals().hyperlink_color));
    }
    let response = ui.add(button);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, label));
    response.on_hover_text(label)
}

fn optional_text(ui: &mut egui::Ui, label: &str, value: &mut Option<String>, hint: &str) {
    let id = ui.label(label).id;
    let text = value.get_or_insert_with(String::new);
    ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY).hint_text(hint)).labelled_by(id);
    if value.as_ref().is_some_and(|s| s.trim().chars().count() > 80) {
        error(ui, &format!("{label}不能超过 80 个字符"));
    }
}

fn number<T: Copy + Display + FromStr>(ui: &mut egui::Ui, state: &mut EditorState, key: &str,
    label: &str, value: &mut Option<T>, required: bool, valid: impl Fn(T) -> bool) {
    let label_id = ui.label(label).id;
    let text = state.text.entry(key.into()).or_insert_with(|| value.map(|v| v.to_string()).unwrap_or_default());
    ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY).id_salt(key)).labelled_by(label_id);
    let parsed = if text.trim().is_empty() && !required { Ok(None) }
        else { text.trim().parse::<T>().ok().filter(|v| valid(*v)).map(Some).ok_or(()) };
    match parsed {
        Ok(next) => { *value = next; state.errors.remove(key); }
        Err(()) => {
            let message = format!("{label}：请输入有效{}数字", if required { "的必填" } else { "的非负" });
            error(ui, &message);
            state.errors.insert(key.into(), message);
        }
    }
}

fn tokens(ui: &mut egui::Ui, state: &mut EditorState, key: &str, label: &str,
    value: &mut Option<u32>, presets: &[(&str, u32)]) {
    ui.horizontal_wrapped(|ui| {
        for &(name, preset) in presets {
            if chip(ui, *value == Some(preset), name).clicked() {
                *value = Some(preset);
                state.text.insert(key.into(), preset.to_string());
                state.errors.remove(key);
            }
        }
        if ui.button("清空").on_hover_text(format!("清空{label}")).clicked() {
            *value = None; state.text.insert(key.into(), String::new()); state.errors.remove(key);
        }
    });
    number(ui, state, key, label, value, false, |v| v > 0);
}

fn fits(dialect: ReasoningDialect, protocol: Protocol) -> bool {
    match dialect {
        ReasoningDialect::AnthropicBudget | ReasoningDialect::AnthropicAdaptive => protocol == Protocol::AnthropicMessages,
        ReasoningDialect::OpenAi => protocol != Protocol::AnthropicMessages,
        _ => protocol == Protocol::OpenAiChat,
    }
}
fn auto(dialect: ReasoningDialect) -> bool {
    matches!(dialect, ReasoningDialect::ThinkingType | ReasoningDialect::Qwen)
}
fn reasoning(dialect: ReasoningDialect) -> ReasoningConfig {
    ReasoningConfig {
        dialect,
        efforts: if auto(dialect) { vec![ReasoningEffort::None, ReasoningEffort::Auto] }
            else { vec![ReasoningEffort::Low, ReasoningEffort::Medium, ReasoningEffort::High] },
        budget_tokens: if dialect == ReasoningDialect::AnthropicBudget { Some(1024) } else { None },
    }
}
fn set_type(value: &mut ModelEntry, kind: ModelType, state: &mut EditorState) {
    if value.settings.model_type == kind { return; }
    value.settings.model_type = kind;
    if kind != ModelType::Chat {
        value.settings.reasoning = None;
        value.settings.native_web_search = false;
        value.settings.pricing = None;
        value.settings.supports_tools = None;
        state.clear("reasoning"); state.clear("pricing");
    }
}
fn tier(start: u32) -> PriceTier {
    PriceTier { min_input_tokens: start, input: 0.0, output: 0.0, cache_read: None, cache_write: None }
}

pub(crate) fn fields(ui: &mut egui::Ui, value: &mut ModelEntry, protocol: Protocol,
    existing: bool, state: &mut EditorState) -> Result<(), String> {
    ui.spacing_mut().item_spacing.y = t::SPACE_SM;
    let label = ui.label("模型 ID").id;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - if existing { t::CONTROL_SMALL + t::SPACE_SM } else { 0.0 }).max(t::CONTROL);
        ui.add_sized([width, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut value.id).interactive(!existing)).labelled_by(label);
        if existing && ui.button(egui::RichText::new("\u{E1CA}").size(t::TITLE))
            .on_hover_text("复制模型 ID").accessible_name("复制模型 ID").clicked() {
            ui.ctx().copy_text(value.id.clone());
        }
    });
    if value.id.trim().is_empty() { error(ui, "模型 ID 不能为空"); }
    optional_text(ui, "模型名称", &mut value.settings.name, &value.id);
    optional_text(ui, "分组名称", &mut value.settings.group_name,
        value.id.split(['-', '/', ':']).next().unwrap_or_default());
    if matches!(value.settings.model_type, ModelType::Chat | ModelType::Image) {
        let label = ui.label("用途").id;
        ui.horizontal_wrapped(|ui| {
            for (name, kind, vision) in [("对话", ModelType::Chat, false), ("图像生成", ModelType::Image, false), ("图像编辑", ModelType::Image, true)] {
                let selected = value.settings.model_type == kind && (kind == ModelType::Chat || value.capabilities.vision == vision);
                if ui.radio(selected, name).labelled_by(label).clicked() {
                    set_type(value, kind, state);
                    if kind == ModelType::Image { value.capabilities.vision = vision; }
                }
            }
        });
    }
    egui::CollapsingHeader::new("更多设置").default_open(true).show(ui, |ui| {
        let label = ui.label("模型类型").id;
        ui.horizontal_wrapped(|ui| {
            for kind in [ModelType::Chat, ModelType::Image, ModelType::Embedding, ModelType::Rerank] {
                if chip(ui, value.settings.model_type == kind, kind.label()).labelled_by(label).clicked() {
                    set_type(value, kind, state);
                }
            }
            egui::ComboBox::from_id_salt("other_model_type").selected_text(
                if matches!(value.settings.model_type, ModelType::Video | ModelType::Tts | ModelType::Stt | ModelType::Unknown) { value.settings.model_type.label() } else { "其它" }
            ).show_ui(ui, |ui| {
                for kind in [ModelType::Video, ModelType::Tts, ModelType::Stt, ModelType::Unknown] {
                    if ui.selectable_label(value.settings.model_type == kind, kind.label()).labelled_by(label).clicked() { set_type(value, kind, state); }
                }
            }).response.labelled_by(label);
        });
        if value.settings.model_type == ModelType::Chat {
            ui.label("模型能力");
            ui.horizontal_wrapped(|ui| {
                let enabled = value.settings.reasoning.is_some();
                if chip(ui, enabled, "推理").clicked() {
                    state.clear("reasoning");
                    value.settings.reasoning = (!enabled).then(|| reasoning(if protocol == Protocol::AnthropicMessages { ReasoningDialect::AnthropicBudget } else { ReasoningDialect::OpenAi }));
                }
                let tools = value.settings.supports_tools;
                if chip(ui, tools == Some(true), if tools.is_none() { "工具 · 未探测" } else if tools == Some(false) { "工具 · 不支持" } else { "工具" }).clicked() {
                    value.settings.supports_tools = match tools { None => Some(true), Some(true) => Some(false), Some(false) => None };
                    value.capabilities.tools = value.settings.supports_tools == Some(true);
                }
                if protocol != Protocol::OpenAiChat && chip(ui, value.settings.native_web_search, "联网").clicked() {
                    value.settings.native_web_search = !value.settings.native_web_search;
                }
            });
            if let Some(config) = &mut value.settings.reasoning {
                let label = ui.label("推理方言").id;
                ui.horizontal_wrapped(|ui| {
                    for (dialect, name) in [(ReasoningDialect::OpenAi, "OpenAI"), (ReasoningDialect::AnthropicBudget, "Anthropic 预算"), (ReasoningDialect::AnthropicAdaptive, "Anthropic 自适应"), (ReasoningDialect::ThinkingType, "Thinking type"), (ReasoningDialect::Qwen, "Qwen")] {
                        if fits(dialect, protocol) && ui.selectable_label(config.dialect == dialect, name).labelled_by(label).clicked() {
                            *config = reasoning(dialect); state.clear("reasoning");
                        }
                    }
                });
                let label = ui.label("允许的推理强度（至少一项）").id;
                ui.horizontal_wrapped(|ui| {
                    for (effort, name) in [(ReasoningEffort::None, "none"), (ReasoningEffort::Auto, "auto"), (ReasoningEffort::Minimal, "minimal"), (ReasoningEffort::Low, "low"), (ReasoningEffort::Medium, "medium"), (ReasoningEffort::High, "high"), (ReasoningEffort::Xhigh, "xhigh"), (ReasoningEffort::Max, "max")] {
                        if effort == ReasoningEffort::Auto && !auto(config.dialect) { continue; }
                        let mut selected = config.efforts.contains(&effort);
                        if ui.checkbox(&mut selected, name).labelled_by(label).changed() {
                            if selected { config.efforts.push(effort); } else { config.efforts.retain(|e| *e != effort); }
                        }
                    }
                });
                ui.weak("首项为默认推理强度；取消后重新勾选会移到末尾。");
                if matches!(config.dialect, ReasoningDialect::AnthropicBudget | ReasoningDialect::Qwen) {
                    let min = if config.dialect == ReasoningDialect::AnthropicBudget { 1024 } else { 1 };
                    number(ui, state, "reasoning.budget", "推理 token 预算", &mut config.budget_tokens,
                        config.dialect == ReasoningDialect::AnthropicBudget, |v| v >= min);
                }
            }
            let label = ui.label("输入模态").id;
            if chip(ui, value.capabilities.vision, "视觉输入").labelled_by(label).clicked() {
                value.capabilities.vision = !value.capabilities.vision;
            }
            if protocol != Protocol::OpenAiChat { ui.small("联网由服务商原生搜索工具执行，可能产生额外费用。"); }
            else if value.settings.native_web_search {
                error(ui, "此协议不支持原生联网搜索，请关闭");
                ui.checkbox(&mut value.settings.native_web_search, "原生联网搜索");
            }
        }
        ui.checkbox(&mut value.capabilities.streaming, "支持流式输出");
        value.settings.supports_streaming = value.capabilities.streaming;
        ui.weak("流式声明保留；用于运行时选择流式或完整响应。");
        tokens(ui, state, "context", "上下文窗口（token，留空为 32000）", &mut value.settings.context_window,
            &[("128K", 128000), ("200K", 200000), ("256K", 262144), ("400K", 400000), ("512K", 524288), ("1M", 1000000)]);
        tokens(ui, state, "output", "最大输出（token，留空为 4096）", &mut value.settings.max_output,
            &[("16K", 16384), ("32K", 32768), ("64K", 65536), ("128K", 131072)]);
        if value.settings.model_type == ModelType::Chat {
            ui.separator();
            ui.label("价格（每百万 token）");
            let mut clear = false;
            if let Some(pricing) = &mut value.settings.pricing {
                let label = ui.label("币种").id;
                ui.horizontal(|ui| {
                    for currency in ["USD", "CNY"] {
                        if ui.selectable_label(pricing.currency == currency, currency).labelled_by(label).clicked() { pricing.currency = currency.into(); }
                    }
                });
                let mut remove = None;
                for (index, price) in pricing.tiers.iter_mut().enumerate() {
                    ui.push_id(index, |ui| {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(format!("第 {} 档", index + 1));
                                if ui.button("删除此档").clicked() { remove = Some(index); }
                            });
                            let prefix = format!("pricing.{index}.");
                            if index == 0 {
                                let label = ui.label("起点（输入 token）").id;
                                ui.add_enabled(false, egui::DragValue::new(&mut price.min_input_tokens)).labelled_by(label);
                            } else {
                                let mut start = Some(price.min_input_tokens);
                                number(ui, state, &format!("{prefix}start"), "起点（输入 token）", &mut start, true, |_| true);
                                if let Some(start) = start { price.min_input_tokens = start; }
                            }
                            for (key, label, target) in [("input", "输入价格", &mut price.input), ("output", "输出价格", &mut price.output)] {
                                let mut amount = Some(*target);
                                number(ui, state, &format!("{prefix}{key}"), label, &mut amount, true, |v: f64| v.is_finite() && (0.0..=1e6).contains(&v));
                                if let Some(amount) = amount { *target = amount; }
                            }
                            for (key, label, target) in [("read", "缓存读取（留空同输入）", &mut price.cache_read), ("write", "缓存写入（留空同输入）", &mut price.cache_write)] {
                                number(ui, state, &format!("{prefix}{key}"), label, target, false, |v: f64| v.is_finite() && (0.0..=1e6).contains(&v));
                            }
                        });
                    });
                }
                if let Some(index) = remove {
                    pricing.tiers.remove(index);
                    if let Some(first) = pricing.tiers.first_mut() { first.min_input_tokens = 0; } else { clear = true; }
                    state.remove_tier(index);
                }
                ui.horizontal(|ui| {
                    let next = pricing.tiers.last().map_or(Some(0), |p| p.min_input_tokens.checked_add(200000));
                    if ui.add_enabled(pricing.tiers.len() < 8 && next.is_some(), egui::Button::new("添加价格档")).clicked() {
                        if let Some(next) = next { pricing.tiers.push(tier(next)); }
                    }
                    clear |= ui.button("清除价格").clicked();
                });
            } else if ui.button("添加价格").clicked() {
                value.settings.pricing = Some(ModelPricing { currency: "USD".into(), tiers: vec![tier(0)] });
                state.clear("pricing");
            }
            if clear { value.settings.pricing = None; state.clear("pricing"); }
        }
    });
    let id = value.id.trim();
    let validation = if id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) { Err("模型 ID 须为 1–512 字节且不含控制字符".into()) }
        else if let Some(message) = state.errors.values().next() { Err(message.clone()) }
        else { value.settings.validate(protocol) };
    if let Err(message) = &validation { error(ui, message); }
    validation
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chip_keyboard_activation_and_disabled_state() {
        let ctx = egui::Context::default();
        let mut selected = false;
        let mut draw = |events, enabled| {
            let mut response = None;
            let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                response = Some(ui.add_enabled_ui(enabled, |ui| chip(ui, selected, "视觉输入")).inner);
            });
            output.textures_delta.clear();
            let response = response.unwrap();
            if response.clicked() { selected = !selected; }
            (response.id, selected)
        };
        let (id, _) = draw(vec![], true);
        ctx.memory_mut(|memory| memory.request_focus(id));
        let event = |pressed| egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
        assert!(draw(vec![event(true)], true).1);
        draw(vec![event(false)], true);
        assert!(draw(vec![event(true)], false).1);
    }

    #[test]
    fn reasoning_defaults_fit_protocol_and_budget() {
        assert!(fits(ReasoningDialect::OpenAi, Protocol::OpenAiChat));
        assert!(!fits(ReasoningDialect::OpenAi, Protocol::AnthropicMessages));
        assert!(!fits(ReasoningDialect::Qwen, Protocol::OpenAiResponses));
        assert_eq!(reasoning(ReasoningDialect::AnthropicBudget).budget_tokens, Some(1024));
        let mut state = EditorState::default();
        state.text.insert("pricing.1.input".into(), "invalid".into());
        state.errors.insert("pricing.1.input".into(), "invalid".into());
        state.remove_tier(0);
        assert_eq!(state.text.get("pricing.0.input").map(String::as_str), Some("invalid"));
        assert!(state.errors.contains_key("pricing.0.input"));
        assert!(reasoning(ReasoningDialect::Qwen).efforts.contains(&ReasoningEffort::Auto));
    }
}
