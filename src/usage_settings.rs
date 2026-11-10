//! Global request-ledger statistics. Queries run only on opening, changing range or refreshing.
use crate::DesktopApp;
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{AnyElement, Context, Div, IntoElement, ParentElement, Styled, div};
use lumakit_ai::usage::{UsageGroup, UsageReport};
use lumakit_design::{
    LumaTheme, Roles,
    components::{label, settings as ui},
    metrics::{size, space, text},
};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Range {
    #[default]
    All,
    Week,
    Month,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Grouping {
    #[default]
    Model,
    Provider,
    Day,
}

#[derive(Default)]
pub(super) struct UsageSettings {
    range: Range,
    grouping: Grouping,
    report: Option<Result<UsageReport, String>>,
}

fn since_ms(range: Range, now: u64) -> Option<u64> {
    match range {
        Range::All => None,
        Range::Week => Some(now.saturating_sub(7 * 24 * 60 * 60 * 1000)),
        Range::Month => Some(now.saturating_sub(30 * 24 * 60 * 60 * 1000)),
    }
}

impl DesktopApp {
    pub(super) fn refresh_usage(&mut self) {
        self.usage.report = Some(crate::settings_task::action(
            "settings.usage.read",
            "SETTINGS_USAGE_READ_FAILED",
            "无法读取用量统计；请检查聊天记录数据库与系统时间",
            || {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| "无法读取当前时间".to_owned())?
                    .as_millis()
                    .try_into()
                    .map_err(|_| "系统时间超出统计范围".to_owned())?;
                self.chat
                    .session
                    .usage_report(None, since_ms(self.usage.range, now))
            },
        ));
    }

    pub(super) fn usage_page(&self, cx: &Context<Self>) -> AnyElement {
        let r = LumaTheme::global(cx).roles;
        let mut ranges = div().flex().flex_wrap().gap(space::S4).min_w_0();
        for (index, (range, title)) in [
            (Range::All, "全部"),
            (Range::Week, "近 7 天"),
            (Range::Month, "近 30 天"),
        ]
        .into_iter()
        .enumerate()
        {
            ranges = ranges.child(
                Button::new(("usage-range", index))
                    .small()
                    .ghost()
                    .label(title)
                    .accessibility_label(format!("统计时间范围：{title}"))
                    .toggled(self.usage.range == range)
                    .selected(self.usage.range == range)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.usage.range = range;
                        this.refresh_usage();
                        cx.notify();
                    })),
            );
        }
        ranges = ranges.child(
            Button::new("usage-refresh")
                .small()
                .ghost()
                .label("刷新")
                .accessibility_label("刷新全局用量统计")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.refresh_usage();
                    cx.notify();
                })),
        );
        let mut page = div()
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .max_w(size::CONTENT_MAX)
            .gap(space::S16)
            .child(ui::page_header(
                "用量统计",
                Some("本地请求账本；删除对话不会清除统计。输入包含缓存，推理是输出子集。".into()),
                &r,
            ))
            .child(ranges);
        match &self.usage.report {
            Some(Ok(report)) => {
                page = page.child(ui::group("累计用量", None, &r)
                    .child(lumakit_gpui::chat::usage_totals(&report.total, r)))
                    .child(label(text::METADATA, format!(
                        "账本覆盖：{} 个请求缺少时间，{} 个为历史导入（历史副本无法可靠去重）。{}",
                        report.undated_requests, report.legacy_requests,
                        if self.usage.range == Range::All { "按天分组不包含无时间记录。" }
                        else { "当前时间范围排除无时间记录；天分组使用 UTC 日期。" })));
                let mut groups = div().flex().flex_wrap().gap(space::S4).min_w_0();
                for (index, (grouping, title)) in [
                    (Grouping::Model, "按模型"),
                    (Grouping::Provider, "按服务商"),
                    (Grouping::Day, "按天（UTC）"),
                ]
                .into_iter()
                .enumerate()
                {
                    groups = groups.child(
                        Button::new(("usage-group", index))
                            .small()
                            .ghost()
                            .label(title)
                            .accessibility_label(format!("用量分组：{title}"))
                            .toggled(self.usage.grouping == grouping)
                            .selected(self.usage.grouping == grouping)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.usage.grouping = grouping;
                                cx.notify();
                            })),
                    );
                }
                page = page.child(groups);
                let entries = match self.usage.grouping {
                    Grouping::Model => &report.by_model,
                    Grouping::Provider => &report.by_provider,
                    Grouping::Day => &report.by_day,
                };
                if entries.is_empty() {
                    page = page.child(ui::empty("此范围没有可分组的请求。", &r));
                }
                for entry in entries {
                    page = page.child(group(entry, r));
                }
            }
            Some(Err(_)) => {
                page = page.child(ui::notice(
                    "读取用量统计失败，请刷新重试；原账本未被修改。",
                    ui::Tone::Error,
                    &r,
                ));
            }
            None => {
                page = page.child(ui::empty("统计尚未读取，请刷新。", &r));
            }
        }
        page.into_any_element()
    }
}

fn group(entry: &UsageGroup, r: Roles) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(space::S8)
        .py(space::S12)
        .border_b_1()
        .border_color(r.border_muted)
        .child(label(text::TITLE_COMPACT, entry.label.clone()).whitespace_normal())
        .child(lumakit_gpui::chat::usage_totals(&entry.totals, r))
}
