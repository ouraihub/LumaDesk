//! Explicit pricing drafts; reference tariffs remain read-only until adopted.
use super::*;
use gpui_kit::component::{Disableable, Selectable};
use lumakit_ai::{model_config::ModelPricing, model_reference};
use lumakit_design::{LumaTheme, metrics::radius};

impl ModelEditor {
    pub(super) fn pricing_card(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = LumaTheme::global(cx).roles;
        let mut card = lumakit_gpui::providers::form_fields(std::iter::empty())
            .child(group_label("价格 · 每百万 Token"));
        if let Some(pricing) = &self.value.settings.pricing {
            card = card.child(
                label(styles::METADATA, "已设置自定义价格；清除后恢复来源参考。")
                    .text_color(r.text_subtle),
            );
            let mut currencies = div().flex().flex_wrap().gap(space::S4);
            for currency in ["USD", "CNY"] {
                currencies = currencies.child(
                    self.action(
                        format!("currency-{currency}"),
                        currency,
                        pricing.currency == currency,
                        cx,
                        move |this, _, _| {
                            if let Some(pricing) = &mut this.value.settings.pricing {
                                pricing.currency = currency.into();
                            }
                        },
                    )
                    .selected(pricing.currency == currency)
                    .toggled(pricing.currency == currency)
                    .accessibility_label(format!("价格币种 {currency}")),
                );
            }
            card = card.child(group_label("币种")).child(currencies);
            for (index, _) in pricing.tiers.iter().enumerate() {
                let mut row = div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .gap(space::S8)
                    .pt(space::S12)
                    .border_t_1()
                    .border_color(r.border_muted)
                    .child(group_label(format!("第 {} 档", index + 1)));
                for (field, name) in [
                    ("start", "起点（输入 Token）"),
                    ("input", "输入价格"),
                    ("output", "输出价格"),
                    ("read", "缓存读取（留空同输入）"),
                    ("write", "缓存写入（留空同输入）"),
                ] {
                    row = row.child(self.field(
                        &format!("pricing.{index}.{field}"),
                        &format!("第 {} 档 · {name}", index + 1),
                        index == 0 && field == "start",
                        cx,
                    ));
                }
                row = row.child(
                    div().flex().justify_end().child(
                        self.action(
                            ("remove-tier", index),
                            "删除此档",
                            false,
                            cx,
                            move |this, window, cx| {
                                if let Some(pricing) = &mut this.value.settings.pricing {
                                    pricing.tiers.remove(index);
                                    if let Some(first) = pricing.tiers.first_mut() {
                                        first.min_input_tokens = 0;
                                    } else {
                                        this.value.settings.pricing = None;
                                    }
                                }
                                this.state.remove_tier(index);
                                this.inputs.retain(|key, _| !key.starts_with("pricing."));
                                this.prepare_inputs(window, cx);
                            },
                        )
                        .danger()
                        .accessibility_label(format!("删除第 {} 价格档", index + 1)),
                    ),
                );
                card = card.child(row);
            }
            let next = pricing
                .tiers
                .last()
                .map_or(Some(0), |p| p.min_input_tokens.checked_add(200000));
            card = card.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(space::S8)
                    .child(
                        self.action(
                            "add-tier",
                            "添加价格档",
                            false,
                            cx,
                            move |this, window, cx| {
                                if let (Some(pricing), Some(next)) =
                                    (&mut this.value.settings.pricing, next)
                                    && pricing.tiers.len() < 8
                                {
                                    pricing.tiers.push(tier(next));
                                }
                                this.prepare_inputs(window, cx);
                            },
                        )
                        .disabled(pricing.tiers.len() >= 8 || next.is_none()),
                    )
                    .child(self.action(
                        "clear-price",
                        "恢复自动参考价格",
                        false,
                        cx,
                        |this, window, cx| {
                            this.value.settings.pricing = None;
                            this.reset("pricing", window, cx);
                        },
                    )),
            );
        } else {
            let reference =
                model_reference::resolve(&self.value.id).and_then(|model| model.pricing.as_ref());
            if let Some(pricing) = reference {
                card = card.child(
                    label(
                        styles::METADATA,
                        "来源参考 · 公开标价，仅用于估算，不代表服务商实际账单。未保存为覆盖值。",
                    )
                    .text_color(r.text_subtle),
                );
                for (index, tier) in pricing.tiers.iter().enumerate() {
                    let amount = |value: Option<f64>| {
                        value.map_or_else(|| "同输入".into(), |value| value.to_string())
                    };
                    card = card.child(label(styles::METADATA, format!("第 {} 档 · ≥ {} 输入 Token\n输入 {} · 输出 {} {} / 百万\n缓存读取 {} · 写入 {}", index + 1, tier.min_input_tokens, tier.input, tier.output, pricing.currency, amount(tier.cache_read), amount(tier.cache_write))).text_color(r.text_muted));
                }
            } else {
                card = card.child(
                    label(styles::METADATA, "暂无可用来源价格；可设置自定义估算价格。")
                        .text_color(r.text_subtle),
                );
            }
            card = card.child(
                div().flex().child(
                    self.action(
                        "add-price",
                        "设置自定义价格",
                        false,
                        cx,
                        |this, window, cx| {
                            this.value.settings.pricing = Some(
                                model_reference::resolve(&this.value.id)
                                    .and_then(|model| model.pricing.clone())
                                    .unwrap_or_else(|| ModelPricing {
                                        currency: "USD".into(),
                                        tiers: vec![tier(0)],
                                    }),
                            );
                            this.reset("pricing", window, cx);
                        },
                    )
                    .rounded(radius::MD),
                ),
            );
        }
        card
    }
}
