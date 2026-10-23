//! 产品只负责解析已保存模型；会话与绘制由共享库提供。
use crate::model_settings::ModelSettings;
use eframe::egui;
use lumakit_ai::chat::ChatSession;
use lumakit_egui::ChatPanel;
use lumakit_ui::chat::ChatAction;
use std::{path::Path, sync::Arc};

pub struct ChatState { session: ChatSession, panel: ChatPanel }
impl ChatState {
    pub fn new(root: &Path, ctx: &egui::Context) -> Result<Self, String> {
        let ctx = ctx.clone();
        Ok(Self { session: ChatSession::open(&root.join("chat-history.json"), Arc::new(move || ctx.request_repaint()))?, panel: ChatPanel::new("lumadesk-chat") })
    }
    pub fn poll(&mut self) { self.session.poll(); }
    pub fn cancel(&mut self) { self.session.cancel(); }
    pub fn can_close(&mut self) -> bool { self.session.can_close() }
    pub fn ui(&mut self, ui: &mut egui::Ui, models: &ModelSettings) -> bool {
        let mut settings = false;
        for action in self.panel.show(ui, &self.session, models.available_models()) {
            match action {
                ChatAction::New => self.session.new_conversation(),
                ChatAction::Select(id) => self.session.select_conversation(&id),
                ChatAction::Delete(id) => self.session.delete_conversation(&id),
                ChatAction::SelectModel(model) => self.session.select_model(model),
                ChatAction::Draft(text) => self.session.set_draft(text),
                ChatAction::Stop => self.session.cancel(),
                ChatAction::Save => self.session.retry_save(),
                ChatAction::OpenSettings => settings = true,
                ChatAction::Send | ChatAction::Retry => {
                    let retry = matches!(action, ChatAction::Retry);
                    match self.session.request_model(retry).and_then(|model| models.connection(&model)) {
                        Ok(connection) => if retry { self.session.retry(connection); } else { self.session.send(connection); },
                        Err(error) => self.session.report(error),
                    }
                }
                ChatAction::RetryWith(model) => {
                    match models.connection(&model) {
                        Ok(connection) => { self.session.select_model(model); self.session.retry_with(connection); }
                        Err(error) => self.session.report(error),
                    }
                }
            }
        }
        settings
    }
}
