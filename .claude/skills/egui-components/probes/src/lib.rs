#![cfg(test)]
use egui::{Context, Event, Key, Modifiers, Pos2, RawInput, Rect, Response, Sense, Ui, Vec2};

// 预研示例，不是已经存在于 lumakit-egui 的 API。
pub fn switch(ui: &mut Ui, checked: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(40.0, 24.0), Sense::click());
    if response.clicked() { *checked = !*checked; response.mark_changed(); }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, ui.is_enabled(), *checked, "Enable service"));
    if ui.is_rect_visible(rect) {
        let fill = if *checked { ui.visuals().selection.bg_fill } else { ui.visuals().widgets.inactive.bg_fill };
        ui.painter().rect_filled(rect.shrink(2.0), 10.0, fill);
        let x = if *checked { rect.right() - 12.0 } else { rect.left() + 12.0 };
        ui.painter().circle_filled(Pos2::new(x, rect.center().y), 8.0, ui.visuals().text_color());
    }
    response
}
fn input(events: Vec<Event>) -> RawInput { RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(860.0,560.0))), events, ..Default::default() } }
fn key(key: Key, pressed: bool) -> Event { Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE } }

#[test]
fn switch_keyboard_disabled_and_semantics() {
    let ctx = Context::default(); ctx.enable_accesskit();
    let mut checked = false;
    let mut id = None;
    let mut changed = false;
    let mut draw = |events, enabled| {
        let mut output = ctx.run_ui(input(events), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let response = ui.add_enabled_ui(enabled, |ui| switch(ui, &mut checked)).inner;
                id = Some(response.id); changed = response.changed();
            });
        });
        output.textures_delta.clear();
        (output, checked, id.unwrap(), changed)
    };
    let (output, _, widget, _) = draw(vec![], true);
    assert!(output.platform_output.accesskit_update.is_some());
    ctx.memory_mut(|m| m.request_focus(widget));
    let (_, state, _, changed) = draw(vec![key(Key::Space, true)], true);
    assert!(state && changed);
    draw(vec![key(Key::Space, false)], true);
    let (_, state, _, changed) = draw(vec![key(Key::Enter, true)], true);
    assert!(!state && changed);
    draw(vec![key(Key::Enter, false)], true);
    let (_, state, _, changed) = draw(vec![key(Key::Space, true)], false);
    assert!(!state && !changed);
}

#[test]
fn panels_reserve_space_for_long_content() {
    for (width,height) in [(860.0,560.0),(480.0,300.0),(1200.0,800.0)] {
        let ctx = Context::default();
        let mut header = Rect::NOTHING; let mut footer = Rect::NOTHING; let mut body = Rect::NOTHING;
        for _ in 0..3 {
            let mut output = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(width,height))), ..Default::default() }, |ui| {
                header = egui::Panel::top("header").exact_size(50.0).show(ui, |ui| { ui.label("Header"); }).response.rect;
                footer = egui::Panel::bottom("composer").exact_size(110.0).show(ui, |ui| {
                    let mut draft = String::from("input"); ui.text_edit_multiline(&mut draft); let _ = ui.button("Send");
                }).response.rect;
                body = egui::CentralPanel::default().show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| { for _ in 0..100 { ui.label("Long transcript"); } });
                }).response.rect;
            });
            output.textures_delta.clear();
        }
        assert!(header.bottom() <= body.top()); assert!(body.bottom() <= footer.top()); assert!(footer.bottom() <= height);
    }
}

#[test]
fn modal_escape_is_cancel_not_confirmation() {
    let ctx=Context::default(); let modal_id=egui::Id::unique("confirm");
    let mut close=false; let mut confirmed=false;
    for events in [vec![],vec![],vec![key(Key::Escape,true)]] {
        let mut output=ctx.run_ui(input(events), |ui| {
            egui::CentralPanel::default().show(ui, |ui| { let _ = ui.button("Background"); });
            let modal=egui::Modal::new(modal_id).show(&ctx,|ui| { ui.heading("Delete?"); if ui.button("Confirm").clicked() { confirmed=true; } });
            close=modal.should_close();
        });
        output.textures_delta.clear();
    }
    assert!(close); assert!(!confirmed);
}
