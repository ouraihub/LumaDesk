//! Product log viewer; parsing and log persistence remain owned by lumakit-observability.
use gpui_kit::{
    AppContext, ClipboardItem, Context, Entity, Subscription, Task, Window,
    component::input::{InputEvent, InputState},
};
use lumakit_observability::{LogHandle, LogLevel, LogLine, parse_line};
use std::time::Duration;

mod view;

pub(super) struct LogSettings {
    handle: LogHandle,
    entries: Vec<LogLine>,
    query: Entity<InputState>,
    event: Option<&'static str>,
    layer: Option<String>,
    expanded: Option<usize>,
    read_error: Option<&'static str>,
    level_error: Option<&'static str>,
    level_notice: Option<String>,
    copied: bool,
    poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl LogSettings {
    pub(super) fn create(
        handle: LogHandle,
        window: &mut Window,
        cx: &mut gpui_kit::App,
    ) -> Entity<Self> {
        cx.new(|cx: &mut Context<Self>| {
            let query =
                cx.new(|cx| InputState::new(window, cx).placeholder("搜索 TraceId、ME 或模块"));
            let subscription = cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.expanded = None;
                    this.copied = false;
                    cx.notify();
                }
            });
            Self {
                handle,
                entries: Vec::new(),
                query,
                event: None,
                layer: None,
                expanded: None,
                read_error: None,
                level_error: None,
                level_notice: None,
                copied: false,
                poll: None,
                _subscriptions: vec![subscription],
            }
        })
    }

    /// Reading is intentionally untraced: viewing logs must not generate polling noise.
    pub(super) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        self.poll = None;
        if !active {
            return;
        }
        let handle = self.handle.clone();
        self.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let handle = handle.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        handle
                            .tail(1000)
                            .map(|lines| lines.iter().map(|line| parse_line(line)).collect())
                    })
                    .await;
                if this
                    .update(cx, |this, cx| {
                        match result {
                            Ok(entries) => {
                                if this.entries != entries {
                                    this.expanded = None;
                                }
                                this.entries = entries;
                                this.read_error = None;
                            }
                            Err(_) => {
                                this.read_error =
                                    Some("读取日志失败，请检查目录权限后刷新；原文件未修改。")
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        }));
    }

    fn set_level(&mut self, level: LogLevel, cx: &mut Context<Self>) {
        self.level_error = match self.handle.set_level(level) {
            Ok(_) => {
                self.level_notice = Some(format!(
                    "已切换为 {level:?}；请重新执行操作。历史日志不会补录。"
                ));
                self.set_active(true, cx);
                None
            }
            Err(_) => Some("日志级别保存失败；当前会话可能已切换，重启后不保证恢复，请重试。"),
        };
        cx.notify();
    }

    fn visible(&self, cx: &Context<Self>) -> Vec<(usize, &LogLine)> {
        let query = self.query.read(cx).value().trim().to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, line)| keeps(line, self.event, self.layer.as_deref(), &query))
            .collect()
    }

    fn filter_trace(&mut self, trace: &str, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.query.read(cx).value().trim() == trace {
            ""
        } else {
            trace
        };
        self.query
            .update(cx, |input, cx| input.set_value(next.to_owned(), window, cx));
        // Selecting a chain must not hide its other layers or terminal states.
        self.event = None;
        self.layer = None;
        self.expanded = None;
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let text = self
            .visible(cx)
            .iter()
            .map(|(_, line)| line.raw.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = true;
        cx.notify();
    }
}

fn keeps(line: &LogLine, event: Option<&str>, layer: Option<&str>, query: &str) -> bool {
    event.is_none_or(|event| line.event.as_deref() == Some(event))
        && layer.is_none_or(|layer| line.layer.as_deref() == Some(layer))
        && (query.is_empty() || line.raw.to_lowercase().contains(query))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_intersect_without_conflating_event_layer_or_business_fields() {
        let line = parse_line(
            "2026-11-09T10:00:00Z WARN TraceId:abc|Layer:external|ME:catalog.fetch|EV:TIMEOUT|RT:21|Limit:20",
        );
        assert!(keeps(&line, Some("TIMEOUT"), Some("external"), "abc"));
        assert!(keeps(&line, None, None, "me:catalog.fetch"));
        assert!(!keeps(&line, Some("ERROR"), None, "abc"));
        assert!(!keeps(&line, None, Some("ui"), "abc"));
        assert!(!keeps(&line, None, None, "other-trace"));
        let business =
            parse_line("2026-11-09T10:00:00Z INFO Layer:external|event:TIMEOUT|Msg:ordinary event");
        assert!(!keeps(&business, Some("TIMEOUT"), None, ""));
    }
}
