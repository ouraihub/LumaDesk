use super::*;
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
