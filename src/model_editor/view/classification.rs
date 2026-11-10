use super::*;

impl ModelEditor {
    pub(super) fn classification_card(&self, cx: &Context<Self>) -> gpui_kit::Div {
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
        let mut section = lumakit_gpui::providers::form_fields(std::iter::empty())
            .child(group_label("模型类型"))
            .child(types);
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
}
