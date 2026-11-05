//! Maps existing context estimates and request accounting; called on session changes, not render.
use lumakit_ai::{chat::{ChatPhase, ChatSession}, model_config::ModelSettings};
use lumakit_usage_status::{ContextStatus, Money, StatusState};

pub(super) fn status(session: &ChatSession, settings: Option<&ModelSettings>) -> ContextStatus {
    let defaults = ModelSettings::default();
    let context = session.context_usage(settings.unwrap_or(&defaults));
    let mut status = ContextStatus {
        estimate: true,
        state: match session.phase() {
            ChatPhase::Idle => StatusState::Ready,
            ChatPhase::Running | ChatPhase::Waiting => StatusState::Streaming,
            ChatPhase::Closed => StatusState::Unknown,
        },
        ..ContextStatus::default()
    };
    match context {
        Ok(context) => {
            status.used_tokens = Some(context.tokens);
            status.context_window = context.context_window.map(u64::from);
            status.percent = context.context_window.filter(|n| *n > 0)
                .map(|n| context.tokens as f32 / n as f32 * 100.);
            status.detail = if context.anchored {
                "上下文：匹配请求的已报告输入 + 当前尾部估算。"
            } else { "上下文：本地保守估算，非精确 tokenizer。" }.into();
        }
        Err(error) => status.detail = format!("上下文未知：{error}"),
    }
    if let Some(conversation) = session.selected() {
        match session.usage_report(Some(&conversation.id), None) {
            Ok(report) => {
                let total = report.total;
                if total.missing_usage == 0 {
                    status.cumulative_tokens = Some(total.input_tokens.saturating_add(total.output_tokens));
                }
                if total.configured_cost_requests == total.requests && total.configured_costs.len() == 1 {
                    status.cumulative_cost = total.configured_costs.iter().next().map(|(currency, amount)| Money {
                        currency: currency.clone(), amount: *amount, estimated: true, partial: false,
                    });
                }
                status.detail.push_str(&format!("\n当前对话累计：输入 {}（含缓存），输出 {} token。缓存读取 {}、写入 {} 均为输入子集，推理为输出子集。\n{} 次请求；{} 次缺用量、{} 次缺价格。累计包含重试和工具请求，不是当前上下文占用。", total.input_tokens, total.output_tokens, total.cache_read_tokens, total.cache_write_tokens, total.requests, total.missing_usage, total.missing_cost));
                if status.cumulative_cost.is_none() {
                    status.detail.push_str("\n未完整配置请求单价或涉及多币种，摘要显示 token，不套用目录参考价。已配置的历史请求不按当前单价重算。");
                }
            }
            Err(error) => status.detail.push_str(&format!("\n累计用量未知：{error}")),
        }
    } else { status.cumulative_tokens = Some(0); }
    status.detail.push_str("\n费用仅按请求时保存的用户模型单价估算，不是账单；多币种不换算。魔杖为上下文状态标记，自动压缩是否启用单独说明。");
    status
}
