//! Declared reasoning levels a model offers in chat; dialect follows the route, never the user.
use lumakit_ai::{
    Protocol,
    catalog::ModelEntry,
    model_config::{ReasoningConfig, ReasoningDialect, ReasoningEffort},
};
/// Display and storage order for declared efforts.
const EFFORTS: [ReasoningEffort; 8] = [
    ReasoningEffort::None,
    ReasoningEffort::Minimal,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::Xhigh,
    ReasoningEffort::Max,
    ReasoningEffort::Auto,
];
/// Efforts the editor offers: `auto` only for on/off dialects that can encode it.
pub(super) fn offered_efforts(dialect: ReasoningDialect) -> impl Iterator<Item = ReasoningEffort> {
    let auto = matches!(
        dialect,
        ReasoningDialect::ThinkingType | ReasoningDialect::Qwen
    );
    EFFORTS
        .into_iter()
        .filter(move |effort| auto || *effort != ReasoningEffort::Auto)
}
/// The dialect saved efforts are encoded with: saved, then catalog, then the protocol's own.
pub(super) fn effort_dialect(value: &ModelEntry, protocol: Protocol) -> ReasoningDialect {
    value
        .effective_settings(protocol)
        .reasoning
        .map_or_else(|| ReasoningDialect::for_protocol(protocol), |r| r.dialect)
}
/// Toggles one declared effort, starting from the effective (catalog-filled) list so a prefill is
/// kept rather than discarded. Clearing every level returns to the catalog's declaration.
pub(super) fn toggle_effort(value: &mut ModelEntry, protocol: Protocol, effort: ReasoningEffort) {
    let effective = value.effective_settings(protocol).reasoning;
    let mut config = value
        .settings
        .reasoning
        .clone()
        .or(effective)
        .unwrap_or(ReasoningConfig {
            dialect: ReasoningDialect::for_protocol(protocol),
            efforts: Vec::new(),
            budget_tokens: None,
        });
    if config.efforts.contains(&effort) {
        config.efforts.retain(|e| *e != effort);
    } else {
        config.efforts.push(effort);
        config
            .efforts
            .sort_by_key(|e| EFFORTS.iter().position(|o| o == e));
    }
    value.settings.reasoning = (!config.efforts.is_empty()).then_some(config);
}
