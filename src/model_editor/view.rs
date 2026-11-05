//! Cherry's field hierarchy adapted to native Luma controls, not its autosave.
use super::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Selectable, button::ButtonVariants, switch::Switch};
use gpui_kit::{ClipboardItem, IntoElement, Render};
use lumakit_ai::model_reference;
use lumakit_design::{LumaTheme, Roles, metrics::radius};

pub(super) fn card(r: &Roles) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .gap(space::S12)
        .p(space::S12)
        .rounded(radius::LG)
        .border_1()
        .border_color(r.border_muted)
        .bg(r.container_content_bg)
}
impl ModelEditor {
    fn purpose_select(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = LumaTheme::global(cx).roles;
        let current = if purpose_selected(&self.value, ModelType::Chat, false) {
            Some((ModelType::Chat, false))
        } else if purpose_selected(&self.value, ModelType::Image, false) {
            Some((ModelType::Image, false))
        } else {
            None
        };
        let description = match self.value.settings.model_type {
            ModelType::Chat => "文本对话、推理和工具调用。",
            ModelType::Image if self.value.capabilities.vision => {
                "当前为图像编辑分类；宿主尚未接入图像编辑，不提供该调用选项。"
            }
            ModelType::Image => "根据文字描述生成图像。图像编辑尚未接入。",
            _ => "当前为其他模型类型；可在下方模型类型中调整。",
        };
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(space::S4)
            .child(group_label("用途"))
            .child(self.select(
                "usage",
                "用途",
                current,
                vec![
                    ((ModelType::Chat, false), "对话"),
                    ((ModelType::Image, false), "图像生成"),
                ],
                cx,
                |this, (kind, vision), window, cx| {
                    apply_purpose(&mut this.value, kind, vision, &mut this.state);
                    this.prepare_inputs(window, cx);
                },
            ))
            .child(label(styles::METADATA, description).text_color(r.text_subtle))
    }
    fn classification_card(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = LumaTheme::global(cx).roles;
        let kind = self.value.settings.model_type;
        let mut types = div().flex().flex_wrap().gap(space::S6);
        for (index, value) in [
            ModelType::Chat,
            ModelType::Image,
            ModelType::Embedding,
            ModelType::Rerank,
        ]
        .into_iter()
        .enumerate()
        {
            types = types.child(self.type_chip(index, value, cx));
        }
        let show_others = self.other_types
            || matches!(
                kind,
                ModelType::Video | ModelType::Tts | ModelType::Stt | ModelType::Unknown
            );
        types = types.child(
            self.action(
                "other-types",
                if show_others {
                    "收起其他类型"
                } else {
                    "其他类型"
                },
                show_others,
                cx,
                |this, _, _| this.other_types = !this.other_types,
            )
            .icon(if show_others {
                IconName::ChevronUp
            } else {
                IconName::ChevronDown
            }),
        );
        let mut section = card(&r).child(group_label("模型类型")).child(types);
        if show_others {
            let mut others = div().flex().flex_wrap().gap(space::S6);
            for (index, value) in [
                ModelType::Video,
                ModelType::Tts,
                ModelType::Stt,
                ModelType::Unknown,
            ]
            .into_iter()
            .enumerate()
            {
                others = others.child(self.type_chip(index + 4, value, cx));
            }
            section = section.child(others);
        }
        if kind == ModelType::Chat {
            section = section.child(group_label("模型能力"));
            let tools = self.value.effective_settings(self.protocol).supports_tools == Some(true);
            section = section
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(space::S6)
                        .child(self.reasoning_capability(cx))
                        .child(
                            self.action("tools", "工具调用", tools, cx, move |this, _, _| {
                                set_capability(&mut this.value, Capability::Tools, !tools);
                            })
                            .icon(IconName::Wrench)
                            .toggled(tools)
                            .selected(tools),
                        ),
                )
                .child(
                    label(
                        styles::METADATA,
                        match self.value.settings.supports_tools {
                            None if tools => "工具调用：自动识别为支持，可点击覆盖",
                            None => "工具调用：尚未声明，可点击设置",
                            Some(true) => "工具调用：自定义启用",
                            Some(false) => "工具调用：自定义停用",
                        },
                    )
                    .text_color(r.text_subtle),
                );
            if self.protocol != Protocol::OpenAiChat || self.value.settings.native_web_search {
                section = section.child(
                    self.action(
                        "web",
                        "原生联网（可能产生额外费用）",
                        self.value.settings.native_web_search,
                        cx,
                        |this, _, _| {
                            this.value.settings.native_web_search =
                                !this.value.settings.native_web_search
                        },
                    )
                    .toggled(self.value.settings.native_web_search),
                );
            }
        }
        let vision = self.value.effective_settings(self.protocol).supports_vision;
        section
            .child(group_label("输入模态"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(space::S6)
                    .child(
                        self.action("vision", "视觉输入", vision, cx, move |this, _, _| {
                            set_capability(&mut this.value, Capability::Vision, !vision);
                        })
                        .icon(IconName::Eye)
                        .toggled(vision)
                        .selected(vision),
                    )
                    .child(
                        self.action(
                            "audio",
                            "音频输入",
                            self.value.settings.supports_audio,
                            cx,
                            |this, _, _| {
                                let enabled = !this.value.settings.supports_audio;
                                set_capability(&mut this.value, Capability::Audio, enabled);
                            },
                        )
                        .icon(IconName::Ear)
                        .toggled(self.value.settings.supports_audio)
                        .selected(self.value.settings.supports_audio),
                    )
                    .child(
                        self.action(
                            "video",
                            "视频输入",
                            self.value.settings.supports_video,
                            cx,
                            |this, _, _| {
                                let enabled = !this.value.settings.supports_video;
                                set_capability(&mut this.value, Capability::Video, enabled);
                            },
                        )
                        .icon(IconName::Video)
                        .toggled(self.value.settings.supports_video)
                        .selected(self.value.settings.supports_video),
                    ),
            )
            .child(
                label(
                    styles::METADATA,
                    "点击修改能力声明，保存后生效；推理强度在聊天时调整。音频／视频上传尚未接入。",
                )
                .text_color(r.text_subtle),
            )
    }
    fn reasoning_capability(&self, cx: &Context<Self>) -> Button {
        let supported = self
            .value
            .effective_settings(self.protocol)
            .supports_reasoning();
        self.action(
            "reasoning-capability",
            "推理",
            supported,
            cx,
            move |this, _, _| {
                set_capability(&mut this.value, Capability::Reasoning, !supported);
            },
        )
        .icon(IconName::Brain)
        .toggled(supported)
        .selected(supported)
        .accessibility_label("推理能力")
        .tooltip("声明模型是否支持推理；不设置当前请求的强度")
    }
    fn type_chip(&self, index: usize, kind: ModelType, cx: &Context<Self>) -> Button {
        let selected = self.value.settings.model_type == kind;
        self.action(
            ("model-type", index),
            kind.label(),
            selected,
            cx,
            move |this, window, cx| {
                set_type(&mut this.value, kind, &mut this.state);
                this.prepare_inputs(window, cx);
            },
        )
        .icon(match kind {
            ModelType::Chat => IconName::Type,
            ModelType::Image => IconName::Image,
            ModelType::Embedding => IconName::Boxes,
            ModelType::Rerank => IconName::ArrowUpDown,
            ModelType::Video => IconName::Video,
            ModelType::Tts => IconName::Volume2,
            ModelType::Stt => IconName::Mic,
            ModelType::Unknown => IconName::CircleQuestionMark,
        })
        .toggled(selected)
        .selected(selected)
        .accessibility_label(format!("模型类型：{}", kind.label()))
    }
    fn limits_card(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = LumaTheme::global(cx).roles;
        let reference = model_reference::resolve(&self.value.id);
        let effective = model_reference::effective_settings(
            &self.value.id,
            self.protocol,
            &self.value.settings,
        );
        let mut section = card(&r).child(group_label("Token 容量与限制"));
        for (key, title, presets, value, source) in [
            (
                "context",
                "上下文窗口",
                &[
                    ("128K", 128000),
                    ("200K", 200000),
                    ("256K", 262144),
                    ("400K", 400000),
                    ("512K", 524288),
                    ("1M", 1000000),
                ][..],
                self.value.settings.context_window,
                reference
                    .and_then(|model| model.context_window)
                    .map_or_else(
                        || {
                            format!(
                                "无来源参考 · 默认 {} Token",
                                effective.context_window.unwrap_or(32000)
                            )
                        },
                        |value| format!("来源参考 {value} Token"),
                    ),
            ),
            (
                "input",
                "最大输入 Token",
                &[
                    ("128K", 128000),
                    ("200K", 200000),
                    ("256K", 256000),
                    ("512K", 512000),
                    ("1M", 1000000),
                ][..],
                self.value.settings.max_input,
                "无独立来源参考 · 留空不单独限制输入，仍受上下文窗口约束".into(),
            ),
            (
                "output",
                "最大输出 Token",
                &[
                    ("16K", 16384),
                    ("32K", 32768),
                    ("64K", 65536),
                    ("128K", 128000),
                    ("256K", 256000),
                ][..],
                self.value.settings.max_output,
                reference.and_then(|model| model.max_output).map_or_else(
                    || {
                        format!(
                            "无来源参考 · 默认请求预算 {} Token",
                            effective.output_limit()
                        )
                    },
                    |value| {
                        format!(
                            "来源标注上限 {value} Token（仅参考）；自动请求预算 {} Token",
                            effective.output_limit()
                        )
                    },
                ),
            ),
        ] {
            section = section.child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .gap(space::S6)
                    .child(group_label(title))
                    .child(self.presets(key, presets, cx))
                    .child(
                        Input::new(&self.inputs[key].0)
                            .aria_label(title)
                            .w_full()
                            .min_w_0(),
                    )
                    .child(
                        label(
                            styles::METADATA,
                            match value {
                                Some(value) => format!("自定义覆盖 {value} Token · 清空恢复自动"),
                                None => "自动 · 未保存覆盖值".into(),
                            },
                        )
                        .text_color(if value.is_some() {
                            r.text_muted
                        } else {
                            r.text_subtle
                        }),
                    )
                    .child(label(styles::METADATA, source).text_color(r.text_subtle)),
            );
        }
        section
    }
}
impl Render for ModelEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.prepare_inputs(window, cx);
        let r = LumaTheme::global(cx).roles;
        let copy = Button::new("copy-id")
            .small()
            .ghost()
            .icon(IconName::Copy)
            .accessibility_label("复制模型 ID")
            .tooltip("复制模型 ID")
            .on_click(cx.listener(|this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(this.value.id.clone()))
            }));
        let mut body = div()
            .flex()
            .flex_col()
            .min_w_0()
            .w_full()
            .gap(space::S16)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S4)
                    .min_w_0()
                    .child(group_label("模型 ID"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(space::S6)
                            .min_w_0()
                            .child(
                                Input::new(&self.inputs["id"].0)
                                    .aria_label("模型 ID")
                                    .readonly(self.existing)
                                    .flex_1()
                                    .min_w_0(),
                            )
                            .child(copy),
                    ),
            )
            .child(self.field("name", "模型名称", false))
            .child(self.field("group", "分组名称", false))
            .child(self.purpose_select(cx))
            .child(
                self.action("more", "更多设置", self.more, cx, |this, _, _| {
                    this.more = !this.more
                })
                .w_full()
                .ghost()
                .toggled(self.more)
                .icon(if self.more {
                    IconName::ChevronUp
                } else {
                    IconName::ChevronDown
                }),
            );
        if self.more {
            body = body
                .child(self.classification_card(cx))
                .child(self.limits_card(cx))
                .child(
                    card(&r).child(
                        div()
                            .flex()
                            .items_center()
                            .min_w_0()
                            .gap(space::S8)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(label(styles::UI, "支持流式输出")),
                            )
                            .child(
                                Switch::new("streaming")
                                    .small()
                                    .checked(self.value.capabilities.streaming)
                                    .accessibility_label("支持流式输出")
                                    .on_click(cx.listener(|this, value: &bool, _, cx| {
                                        this.value.capabilities.streaming = *value;
                                        this.value.settings.supports_streaming = *value;
                                        cx.notify();
                                    })),
                            ),
                    ),
                );
            if self.value.settings.model_type == ModelType::Chat {
                body = body.child(self.pricing_card(cx));
            }
        }
        if let Err(error) = self.validate() {
            body = body.child(label(styles::METADATA, error).text_color(r.feedback_error));
        }
        body
    }
}
fn purpose_selected(value: &ModelEntry, kind: ModelType, vision: bool) -> bool {
    value.settings.model_type == kind
        && (kind != ModelType::Image || value.capabilities.vision == vision)
}
fn apply_purpose(value: &mut ModelEntry, kind: ModelType, vision: bool, state: &mut EditorState) {
    set_type(value, kind, state);
    if kind == ModelType::Image {
        value.capabilities.vision = vision;
        value.settings.supports_vision = vision;
        value.settings.image_edit = vision;
    } else {
        value.settings.image_edit = false;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn purpose_tracks_type_and_image_input_without_clearing_chat_vision() {
        let mut model = ModelEntry {
            id: "model".into(),
            capabilities: Default::default(),
            manual: true,
            settings: Default::default(),
        };
        let mut state = EditorState::default();
        apply_purpose(&mut model, ModelType::Image, true, &mut state);
        assert!(purpose_selected(&model, ModelType::Image, true));
        assert!(!purpose_selected(&model, ModelType::Image, false));
        assert!(model.settings.supports_vision && model.settings.image_edit);
        apply_purpose(&mut model, ModelType::Chat, false, &mut state);
        assert!(model.capabilities.vision);
        assert!(purpose_selected(&model, ModelType::Chat, false));
        assert!(!model.settings.image_edit);
        apply_purpose(&mut model, ModelType::Embedding, false, &mut state);
        assert!(!purpose_selected(&model, ModelType::Chat, false));
    }
}
