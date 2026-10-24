//! 产品只负责解析已保存模型；会话与绘制由共享库提供。
use crate::model_settings::ModelSettings;
use eframe::egui;
use lumakit_ai::chat::ChatSession;
use lumakit_egui::ChatPanel;
use lumakit_ui::chat::ChatAction;
use std::{path::{Path, PathBuf}, sync::{Arc, mpsc}};

pub struct ChatHostActions {
    pub save_note: Option<Box<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>>,
    pub insert_markdown: Option<Box<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>>,
    pub export_markdown: Option<Box<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>>,
}

pub struct ChatState { session: ChatSession, panel: ChatPanel, host_actions: ChatHostActions, picker: Option<mpsc::Receiver<Result<Option<PathBuf>, String>>> }
impl ChatState {
    pub fn new(root: &Path, ctx: &egui::Context) -> Result<Self, String> {
        let ctx = ctx.clone();
        Ok(Self { session: ChatSession::open(&root.join("chat.db"), Arc::new(move || ctx.request_repaint()))?, panel: ChatPanel::new("lumadesk-chat"), host_actions: ChatHostActions { save_note: None, insert_markdown: None, export_markdown: None }, picker: None })
    }
    pub fn poll(&mut self) {
        self.session.poll();
        let result = self.picker.as_ref().map(|picker| picker.try_recv());
        match result {
            Some(Ok(result)) => {
                self.picker = None;
                match result { Ok(Some(path)) => if let Err(error) = self.session.add_attachment(&path) { self.session.report(error); }, Ok(None) => {}, Err(error) => self.session.report(error) }
            },
            Some(Err(mpsc::TryRecvError::Disconnected)) => { self.picker = None; self.session.report("附件选择器意外退出，请重试。".into()); },
            _ => {},
        }
    }
    pub fn cancel(&mut self) { self.session.cancel(); }
    pub fn can_close(&mut self) -> bool { self.session.can_close() }
    pub fn ui(&mut self, ui: &mut egui::Ui, models: &ModelSettings) -> bool {
        let mut settings = false;
        let image_models = models.image_models();
        let selected_model = if self.session.composer().image_mode { self.session.composer().image_model.as_ref() } else { self.session.history().model.as_ref() };
        let metadata = selected_model.and_then(|model| models.model_settings(model));
        let actions = ui.add_enabled_ui(self.picker.is_none(), |ui| self.panel.show(ui, &self.session, models.available_models(), image_models, metadata)).inner;
        for action in actions {
            match action {
                ChatAction::New => self.session.new_conversation(), ChatAction::Select(id) => self.session.select_conversation(&id), ChatAction::Delete(id) => self.session.delete_conversation(&id), ChatAction::SelectModel(model) => self.session.select_model(model), ChatAction::Draft(text) => self.session.set_draft(text), ChatAction::Stop => self.session.cancel(), ChatAction::Save => self.session.retry_save(),
                ChatAction::SetComposer(settings) => self.session.set_composer(settings),
                ChatAction::UploadAttachment => {
                    if self.picker.is_none() {
                        let (sender, receiver) = mpsc::channel();
                        let ctx = ui.ctx().clone();
                        match std::thread::Builder::new().name("attachment-picker".into()).spawn(move || { let _ = sender.send(lumakit_egui::chat::pick_attachment()); ctx.request_repaint(); }) {
                            Ok(_) => self.picker = Some(receiver),
                            Err(error) => self.session.report(format!("无法打开附件选择器：{error}")),
                        }
                    }
                },
                ChatAction::RemoveAttachment(id) => self.session.remove_attachment(&id),
                ChatAction::OpenMedia(id) => {
                    if let Err(error) = self.session.media_path(&id).and_then(|path| lumakit_egui::chat::open_media(&path)) { self.session.report(error); }
                },
                ChatAction::SaveNote { attempt_id, markdown } => self.run_host_action("save", &attempt_id, &markdown), ChatAction::InsertMarkdown { attempt_id, markdown } => self.run_host_action("insert", &attempt_id, &markdown), ChatAction::ExportMarkdown { attempt_id, markdown } => self.run_host_action("export", &attempt_id, &markdown), ChatAction::OpenSettings => settings = true,
                ChatAction::EditAttempt { .. } => {}, ChatAction::SubmitEdit { conversation_id, turn_id, attempt_id, expected_text, next_text } => { if let Err(error) = self.session.edit_attempt(&conversation_id, &turn_id, &attempt_id, &expected_text, &next_text) { self.session.report(error); } }, ChatAction::DeleteAttempt { conversation_id, turn_id, attempt_id } => { if let Err(error) = self.session.delete_attempt(&conversation_id, &turn_id, &attempt_id) { self.session.report(error); } }, ChatAction::SelectAttempt { conversation_id, turn_id, attempt_id } => { if let Err(error) = self.session.select_attempt(&conversation_id, &turn_id, &attempt_id) { self.session.report(error); } }, ChatAction::ForkAttempt { conversation_id, turn_id, attempt_id } => { if let Err(error) = self.session.fork_attempt(&conversation_id, &turn_id, &attempt_id) { self.session.report(error); } }, ChatAction::CopyConversation { conversation_id } => { if let Err(error) = self.session.copy_conversation(&conversation_id) { self.session.report(error); } },
                ChatAction::Send | ChatAction::Retry => {
                    let retry = matches!(action, ChatAction::Retry);
                    let connection = self.session.request_model(retry).and_then(|model| {
                        if models.model_settings(&model).is_some_and(|settings| settings.model_type == lumakit_ai::model_config::ModelType::Image) { models.image_connection(&model) }
                        else { models.connection(&model) }
                    });
                    match connection { Ok(connection) => if retry { self.session.retry(connection); } else { self.session.send(connection); }, Err(error) => self.session.report(error) }
                },
                ChatAction::RetryWith(model) => match models.connection(&model) { Ok(connection) => { self.session.select_model(model); self.session.retry_with(connection); }, Err(error) => self.session.report(error) }, ChatAction::TranslateAttempt { .. } | ChatAction::SubmitTranslation { .. } => self.session.report("翻译宿主尚未接入模型选择".into()),
            }
        }
        settings
    }
    fn run_host_action(&mut self, kind: &str, attempt_id: &str, markdown: &str) {
        let callback = match kind { "save" => self.host_actions.save_note.as_ref(), "insert" => self.host_actions.insert_markdown.as_ref(), _ => self.host_actions.export_markdown.as_ref() };
        let result = callback.map(|callback| callback(attempt_id, markdown));
        if let Some(Err(error)) = result { self.session.report(error); }
    }
}
