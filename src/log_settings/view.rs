//! ThinkRail-style log settings, wrapping toolbar and chronological trace rows.
use super::*;
use gpui_kit::{
    AnyElement, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled,
    component::{
        Disableable, Sizable,
        button::{Button, ButtonVariants},
        input::Input,
    },
    div,
    prelude::FluentBuilder,
};
use lumakit_design::{
    LumaTheme, Roles,
    components::{label, settings as ui},
    metrics::{size, space, text},
};

const EVENTS: [&str; 7] = [
    "ENTRY",
    "EXIT",
    "ERROR",
    "CANCEL",
    "TIMEOUT",
    "UNFINISHED",
    "全部",
];
const LEVELS: [(LogLevel, &str); 5] = [
    (LogLevel::Error, "ERROR"),
    (LogLevel::Warn, "WARN"),
    (LogLevel::Info, "INFO"),
    (LogLevel::Debug, "DEBUG"),
    (LogLevel::Trace, "TRACE"),
];

impl LogSettings {
    fn filters(&self, cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity();
        let level = ui::select(
            ui::select_button(
                "logs-level",
                "记录级别",
                format!("{:?}", self.handle.level()),
            )
            .small(),
            Some(self.handle.level()),
            LEVELS.map(|(level, name)| (level, name.into())),
            move |level, _, cx| entity.update(cx, |this, cx| this.set_level(level, cx)),
        );
        let entity = cx.entity();
        let event = ui::select(
            ui::select_button("logs-event", "事件 EV", self.event.unwrap_or("全部事件")).small(),
            Some(self.event),
            EVENTS.map(|event| {
                let value = (event != "全部").then_some(event);
                (value, event.into())
            }),
            move |event, _, cx| {
                entity.update(cx, |this, cx| {
                    this.event = event;
                    cx.notify();
                })
            },
        );
        let mut layers: Vec<_> = self
            .entries
            .iter()
            .filter_map(|line| line.layer.as_deref())
            .collect();
        layers.sort_unstable();
        layers.dedup();
        let options: Vec<_> = std::iter::once((None, "全部层级".into()))
            .chain(
                layers
                    .iter()
                    .enumerate()
                    .map(|(index, layer)| (Some(index), (*layer).into())),
            )
            .collect();
        let selected = self
            .layer
            .as_deref()
            .and_then(|value| layers.iter().position(|layer| *layer == value));
        let layers: Vec<String> = layers.into_iter().map(str::to_owned).collect();
        let entity = cx.entity();
        let layer = ui::select(
            ui::select_button(
                "logs-layer",
                "层级 Layer",
                self.layer.clone().unwrap_or_else(|| "全部层级".into()),
            )
            .small(),
            Some(selected),
            options,
            move |index, _, cx| {
                entity.update(cx, |this, cx| {
                    this.layer = index.and_then(|index| layers.get(index).cloned());
                    cx.notify();
                })
            },
        );
        div()
            .flex()
            .flex_wrap()
            .gap(space::S8)
            .min_w_0()
            .child(level)
            .child(event)
            .child(layer)
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .child(Input::new(&self.query).small()),
            )
            .into_any_element()
    }

    fn row(&self, index: usize, line: &LogLine, r: &Roles, cx: &Context<Self>) -> AnyElement {
        let color = match line.level.as_str() {
            "ERROR" => r.feedback_error,
            "WARN" => r.feedback_warning,
            _ => r.text_muted,
        };
        let trace = line.trace_id.clone();
        let mut heading = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(space::S4)
            .min_w_0()
            .child(
                label(text::CODE_SMALL, format!("{} {}", line.time, line.level)).text_color(color),
            );
        if let Some(trace) = trace {
            let selected = self.query.read(cx).value().trim() == trace;
            let filter = trace.clone();
            heading = heading.child(
                Button::new(("trace-filter", index))
                    .link()
                    .xsmall()
                    .label(trace.chars().take(8).collect::<String>())
                    .accessibility_label(format!("只看 Trace {trace} 的日志；再点一次取消筛选"))
                    .tooltip(trace.clone())
                    .toggled(selected)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.filter_trace(&filter, window, cx)
                    })),
            );
            if line.event.as_deref() == Some("ENTRY") {
                heading = heading.child(
                    Button::new(("copy-trace", index))
                        .link()
                        .xsmall()
                        .label("复制诊断 ID")
                        .accessibility_label(format!("复制诊断 ID {trace}"))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(trace.clone()))
                        }),
                );
            }
        }
        let mut summary = format!(
            "{} {} {}",
            line.layer.as_deref().unwrap_or(""),
            line.operation
                .as_deref()
                .or(line.target.as_deref())
                .unwrap_or(""),
            line.event.as_deref().unwrap_or("")
        );
        if let Some(ms) = line.duration_ms {
            summary.push_str(&format!(" {ms}ms"));
        }
        if !line.message.is_empty() {
            summary.push_str(&format!(" {}", line.message));
        }
        div()
            .flex()
            .flex_col()
            .gap(space::S4)
            .min_w_0()
            .w_full()
            .py(space::S8)
            .border_b_1()
            .border_color(r.border_muted)
            .child(heading)
            .child(
                label(text::CODE_SMALL, summary)
                    .text_color(color)
                    .min_w_0()
                    .whitespace_normal(),
            )
            .child(
                Button::new(("log-detail", index))
                    .link()
                    .xsmall()
                    .label(if self.expanded == Some(index) {
                        "收起诊断"
                    } else {
                        "诊断详情"
                    })
                    .accessibility_label(format!("日志第 {} 条诊断详情", index + 1))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.expanded = if this.expanded == Some(index) {
                            None
                        } else {
                            Some(index)
                        };
                        cx.notify();
                    })),
            )
            .when(self.expanded == Some(index), |row| {
                let source = match (&line.source_file, line.source_line) {
                    (Some(file), Some(line)) => format!("Source: {file}:{line}"),
                    (Some(file), None) => format!("Source: {file}"),
                    _ => "Source: unavailable".to_owned(),
                };
                row.child(
                    div()
                        .id(("log-raw", index))
                        .min_w_0()
                        .w_full()
                        .overflow_x_scroll()
                        .child(label(text::CODE_SMALL, source).text_color(r.text_muted))
                        .child(label(text::CODE_SMALL, line.raw.clone()).text_color(r.text_subtle)),
                )
            })
            .into_any_element()
    }
}

impl Render for LogSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = LumaTheme::global(cx).roles;
        let visible = self.visible(cx);
        let count = visible.len();
        let rows: Vec<_> = visible
            .into_iter()
            .map(|(index, line)| self.row(index, line, &r, cx))
            .collect();
        let filters = self.filters(cx);
        div().flex().flex_col().min_w_0().w_full().max_w(size::CONTENT_MAX).gap(space::S12)
            .child(ui::page_header("后台日志", Some("按 TraceId 还原操作链路。默认 INFO；调试需提前启用 DEBUG，过去被过滤的日志不会补录。".into()), &r))
            .child(filters)
            .child(div().flex().flex_wrap().gap(space::S8)
                .child(Button::new("logs-refresh").small().outline().label("刷新日志")
                    .on_click(cx.listener(|this, _, _, cx| this.set_active(true, cx))))
                .child(Button::new("logs-copy").small().outline().label("复制筛选结果").disabled(count == 0)
                    .on_click(cx.listener(|this, _, _, cx| this.copy(cx))))
                .child(Button::new("logs-directory").small().outline().label("打开日志目录")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(dir) = this.handle.status().path.parent() { cx.reveal_path(dir); }
                    }))))
            .child(label(text::METADATA, format!("最近 1000 条中匹配 {count} 条；按时间顺序显示" )).text_color(r.text_subtle))
            .when(self.copied, |page| page.child(label(text::METADATA, "已复制筛选结果").text_color(r.feedback_success)))
            .when_some(self.level_notice.clone(), |page, notice| page.child(label(text::METADATA, notice).text_color(r.feedback_info)))
            .when_some(self.read_error.or(self.level_error), |page, error| page.child(ui::notice(error, ui::Tone::Error, &r)))
            .when(count == 0, |page| page.child(ui::empty("没有匹配的日志；可清空筛选或等待新的操作。", &r)))
            .children(rows)
    }
}
