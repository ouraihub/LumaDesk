fn main() {
    let ctx = egui::Context::default();
    let mut draft = String::from("A long draft stays in the composer, not in the transcript.");
    let mut measured = [egui::Rect::NOTHING; 3];
    let mut frames = 0;
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(860.0,560.0))), ..Default::default() }, |ui| {
            measured[0] = egui::Panel::top("header").exact_size(50.0).show(ui, |ui| { ui.heading("Component research"); }).response.rect;
            measured[2] = egui::Panel::bottom("composer").exact_size(110.0).show(ui, |ui| { ui.text_edit_multiline(&mut draft); let _ = ui.button("Send"); }).response.rect;
            measured[1] = egui::CentralPanel::default().show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| { for n in 0..100 { ui.label(format!("Message {n}: body is independently scrolled.")); } });
            }).response.rect;
        });
        let meshes = ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
        assert!(!meshes.is_empty());
        // Only this CPU smoke has no GPU renderer. Production integrations must apply the deltas.
        output.textures_delta.clear();
        frames += 1;
    }
    assert!(measured[0].bottom() <= measured[1].top());
    assert!(measured[1].bottom() <= measured[2].top());
    println!("SMOKE PASS: {frames} CPU-rendered/tessellated frames; header={:?}; transcript={:?}; composer={:?}", measured[0],measured[1],measured[2]);
}
