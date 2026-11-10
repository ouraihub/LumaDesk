//! Model editor, deletion confirmation and discovery picker views.
use super::*;

impl ModelSettings {
    pub(super) fn editor_view(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let editor = self.editor.as_ref().unwrap();
        let title = match editor {
            EditorDialog::Provider { .. } => "服务商设置",
            EditorDialog::Endpoint { .. } => "线路设置",
            EditorDialog::Model { .. } => "模型设置",
            EditorDialog::Keys { .. } => "API 密钥管理",
        };
        let mut fields = Vec::new();
        let mut destructive = None;
        match editor {
            EditorDialog::Provider { id, .. } => {
                fields.push(shared::field("名称", self.input("name"), &r).into_any_element());
                if let Some(id) = id {
                    let id = id.clone();
                    destructive = Some(
                        self.button(
                            "delete-provider",
                            "删除服务商…",
                            cx,
                            move |this, window, cx| {
                                this.delete_provider = Some(id.clone());
                                this.close_editor(window, cx);
                            },
                        )
                        .danger()
                        .into_any_element(),
                    );
                }
            }
            EditorDialog::Endpoint {
                id, protocol, url, ..
            } => {
                let (page, current) = (cx.entity(), *protocol);
                let protocols = ui::select(
                    ui::select_button("endpoint-protocol", "协议", protocol_name(current)).w_full(),
                    Some(current),
                    [
                        Protocol::OpenAiChat,
                        Protocol::OpenAiResponses,
                        Protocol::AnthropicMessages,
                    ]
                    .map(|p| (p, protocol_name(p).into())),
                    move |value, _, cx| {
                        page.update(cx, |this, cx| {
                            if let Some(EditorDialog::Endpoint { protocol, .. }) = &mut this.editor
                            {
                                *protocol = value;
                            }
                            cx.notify();
                        })
                    },
                );
                fields.push(shared::field("线路名称", self.input("name"), &r).into_any_element());
                fields.push(
                    shared::field("API 地址", self.input("editor-url"), &r).into_any_element(),
                );
                fields.push(shared::field("协议", protocols, &r).into_any_element());
                fields.push(
                    ui::request_line(
                        match model_url(url, *protocol) {
                            Ok(url) => format!("GET {url}"),
                            Err(error) => error,
                        },
                        &r,
                    )
                    .into_any_element(),
                );
                if let Some(id) = id {
                    let id = id.clone();
                    destructive = Some(
                        self.button(
                            "delete-endpoint",
                            "删除线路…",
                            cx,
                            move |this, window, cx| {
                                this.delete_endpoint = Some(id.clone());
                                this.close_editor(window, cx);
                            },
                        )
                        .danger()
                        .into_any_element(),
                    );
                }
            }
            EditorDialog::Model { .. } => {
                if let Some(model) = self.view.as_ref().and_then(|view| view.model.clone()) {
                    fields.push(model.into_any_element());
                }
                fields.push(
                    hint("保存后从下一次调用生效；正在生成的回复不受影响。", &r).into_any_element(),
                );
            }
            EditorDialog::Keys { provider, endpoint } => {
                fields.extend(
                    self.keys_manager(provider, endpoint, cx)
                        .map(IntoElement::into_any_element),
                );
            }
        }
        let keys = matches!(editor, EditorDialog::Keys { .. });
        let cancel = self
            .button(
                "close-editor",
                if keys { "关闭" } else { "取消" },
                cx,
                |this, window, cx| this.close_editor(window, cx),
            )
            .outline()
            .into_any_element();
        let save = (!keys).then(|| {
            self.button("save-editor", "保存", cx, |this, window, cx| {
                this.save_editor(window, cx)
            })
            .primary()
            .into_any_element()
        });
        let body = shared::form_fields(fields).into_any_element();
        let footer = shared::form_footer(destructive, cancel, save).into_any_element();
        let notice = self.status(cx).map(IntoElement::into_any_element);
        if matches!(editor, EditorDialog::Model { .. }) {
            shared::form_body(title, body, footer, notice, &r)
        } else {
            shared::compact_form_body(title, body, footer, notice, &r)
        }
    }
    /// Delete confirmation: what goes away, a quiet cancel and a destructive confirm.
    pub(super) fn delete_view(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let note = if self.delete_key.is_some() {
            "将删除此密钥；其它密钥、模型和线路保持不变。"
        } else if self.delete_provider.is_some() {
            "将永久删除此服务商及全部线路、模型与密钥。"
        } else {
            "将永久删除此线路及模型与密钥。"
        };
        ui::delete_confirm(
            note,
            &roles(cx),
            cx.listener(|this, _, _, cx| {
                this.delete_provider = None;
                this.delete_endpoint = None;
                this.delete_key = None;
                cx.notify();
            }),
            cx.listener(|this, _, window, cx| {
                this.delete_confirmed(window, cx);
                cx.notify();
            }),
        )
    }
}
