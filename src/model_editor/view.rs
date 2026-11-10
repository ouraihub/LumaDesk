//! Cherry's field hierarchy adapted to native Luma controls, not its autosave.
use super::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Selectable, button::ButtonVariants, switch::Switch};
use gpui_kit::{ClipboardItem, IntoElement, Render};
use lumakit_ai::model_reference;
use lumakit_design::LumaTheme;
mod classification;
mod limits;

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
        lumakit_gpui::providers::field(
            "用途",
            self.select(
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
            ),
            &r,
        )
        .child(label(styles::METADATA, description).text_color(r.text_subtle))
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
        let mut body = lumakit_gpui::providers::form_fields([
            lumakit_gpui::providers::field(
                "模型 ID",
                Input::new(&self.inputs["id"].0)
                    .aria_label("模型 ID")
                    .readonly(self.existing)
                    .suffix(copy)
                    .w_full()
                    .min_w_0(),
                &r,
            )
            .into_any_element(),
            self.field("name", "模型名称", false, cx).into_any_element(),
            self.field("group", "分组名称", false, cx)
                .into_any_element(),
            self.purpose_select(cx).into_any_element(),
        ])
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
                    lumakit_gpui::providers::form_fields(std::iter::empty()).child(
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
