use super::*;
use lumakit_ai::model_config::{ReasoningDialect, ReasoningEffort};
#[test]
fn invalid_numeric_draft_preserves_saved_value() {
    let mut price = Some(2.0);
    assert!(parse("NaN", &mut price, true, valid_price).is_err());
    assert_eq!(price, Some(2.0));
    assert!(parse("", &mut price, false, valid_price).is_ok());
    assert_eq!(price, None);
    let mut tokens = Some(1024_u32);
    assert!(parse("-1", &mut tokens, true, |v| v >= 1024).is_err());
    assert_eq!(tokens, Some(1024));
    assert!(parse("", &mut tokens, false, |v| v > 0).is_ok());
    assert_eq!(tokens, None);
    assert!(parse("0", &mut tokens, false, |v| v > 0).is_err());
    assert_eq!(tokens, None);
}
#[test]
fn capability_choices_override_catalog_without_selecting_request_effort() {
    let mut model = ModelEntry {
        id: "gpt-6.1-sol".into(),
        capabilities: Default::default(),
        manual: true,
        settings: Default::default(),
    };
    for capability in [
        Capability::Reasoning,
        Capability::Tools,
        Capability::Vision,
        Capability::Audio,
        Capability::Video,
    ] {
        set_capability(&mut model, capability, false);
    }
    let off = model.effective_settings(Protocol::OpenAiChat);
    assert!(
        !off.supports_reasoning()
            && !off.supports_vision
            && !off.supports_audio
            && !off.supports_video
    );
    assert_eq!(off.supports_tools, Some(false));
    for capability in [
        Capability::Reasoning,
        Capability::Tools,
        Capability::Vision,
        Capability::Audio,
        Capability::Video,
    ] {
        set_capability(&mut model, capability, true);
    }
    let restored: ModelEntry =
        serde_json::from_str(&serde_json::to_string(&model).unwrap()).unwrap();
    let on = restored.effective_settings(Protocol::OpenAiChat);
    assert!(
        on.supports_reasoning() && on.supports_vision && on.supports_audio && on.supports_video
    );
    assert_eq!(on.supports_tools, Some(true));
    assert!(on.reasoning_effort.is_none());
    let mut unknown = model;
    unknown.id = "unknown-model".into();
    let declared = unknown.effective_settings(Protocol::OpenAiChat);
    assert!(declared.supports_reasoning());
    assert!(declared.reasoning.is_none()); // A capability declaration does not invent a wire protocol.
}
#[test]
fn declared_efforts_reach_chat_for_uncatalogued_models() {
    let mut model = ModelEntry {
        id: "relay-only-model".into(),
        capabilities: Default::default(),
        manual: true,
        settings: Default::default(),
    };
    set_capability(&mut model, Capability::Reasoning, true);
    for effort in [ReasoningEffort::High, ReasoningEffort::None] {
        toggle_effort(&mut model, Protocol::OpenAiChat, effort);
    }
    let saved = model.settings.reasoning.clone().unwrap();
    assert_eq!(saved.dialect, ReasoningDialect::OpenAi);
    assert_eq!(
        saved.efforts,
        [ReasoningEffort::None, ReasoningEffort::High]
    );
    let effective = model.effective_settings(Protocol::OpenAiChat);
    assert!(effective.validate(Protocol::OpenAiChat).is_ok());
    assert!(
        effective
            .effective_model()
            .supports_effort(ReasoningEffort::High)
    );
    set_capability(&mut model, Capability::Reasoning, false);
    assert!(
        model
            .effective_settings(Protocol::OpenAiChat)
            .effective_model()
            .supported_efforts()
            .is_empty()
    );
    set_capability(&mut model, Capability::Reasoning, true);
    toggle_effort(&mut model, Protocol::OpenAiChat, ReasoningEffort::High);
    toggle_effort(&mut model, Protocol::OpenAiChat, ReasoningEffort::None);
    assert!(model.settings.reasoning.is_none());
    let mut anthropic = ModelEntry {
        id: "relay-claude".into(),
        ..model
    };
    toggle_effort(
        &mut anthropic,
        Protocol::AnthropicMessages,
        ReasoningEffort::Max,
    );
    assert!(
        anthropic
            .effective_settings(Protocol::AnthropicMessages)
            .validate(Protocol::AnthropicMessages)
            .is_ok()
    );
    assert_eq!(
        effort_dialect(&anthropic, Protocol::AnthropicMessages),
        ReasoningDialect::AnthropicAdaptive
    );
    assert!(!offered_efforts(ReasoningDialect::OpenAi).any(|e| e == ReasoningEffort::Auto));
}
#[test]
fn toggling_a_catalogued_model_keeps_its_prefilled_levels() {
    let mut model = ModelEntry {
        id: "gpt-6.1-terra".into(),
        capabilities: Default::default(),
        manual: false,
        settings: Default::default(),
    };
    toggle_effort(&mut model, Protocol::OpenAiResponses, ReasoningEffort::Max);
    assert_eq!(
        model.settings.reasoning.unwrap().efforts,
        [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh
        ]
    );
}
