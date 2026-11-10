use super::*;

impl ModelEditor {
    pub(super) fn limits_card(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = LumaTheme::global(cx).roles;
        let reference = model_reference::resolve(&self.value.id);
        let effective = model_reference::effective_settings(
            &self.value.id,
            self.protocol,
            &self.value.settings,
        );
        let mut section = lumakit_gpui::providers::form_fields(std::iter::empty())
            .child(group_label("Token 容量与限制"));
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
                lumakit_gpui::providers::field(
                    title,
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .gap(space::S6)
                        .child(self.presets(key, presets, cx))
                        .child(
                            Input::new(&self.inputs[key].0)
                                .aria_label(title)
                                .w_full()
                                .min_w_0(),
                        ),
                    &r,
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
