//! Product-owned connection resolution and actions; the session remains UI-neutral.
use crate::model_settings::ModelSettings;
use lumakit_ai::chat::ChatSession;
use lumakit_ai::chat::ChatAction;
use std::{path::{Path, PathBuf}, sync::{Arc, mpsc, atomic::{AtomicBool, Ordering}}};

pub struct ChatState {
    pub session: ChatSession,
    wake: Arc<AtomicBool>,
    picker: Option<mpsc::Receiver<Result<Option<PathBuf>, String>>>,
}

impl ChatState {
    pub fn new(root: &Path) -> Result<Self, String> {
        let wake = Arc::new(AtomicBool::new(false));
        let signal = wake.clone();
        let session = ChatSession::open(&root.join("chat.db"), Arc::new(move || { signal.store(true, Ordering::Release); }))?;
        Ok(Self { session, wake, picker: None })
    }

    pub fn is_busy(&self) -> bool { self.session.busy() || self.picker.is_some() }
    pub fn picking(&self) -> bool { self.picker.is_some() }
    pub fn cancel(&mut self) { self.session.cancel(); }
    pub fn can_close(&mut self) -> bool {
        if self.picker.is_some() {
            self.session.report("请先关闭文件选择窗口，再退出应用。".into());
            return false;
        }
        if !self.session.can_close() { return false; }
        self.session.close().is_ok()
    }

    pub fn poll(&mut self) -> bool {
        let changed = self.wake.swap(false, Ordering::AcqRel);
        let previous_phase = self.session.phase();
        self.session.poll();
        let result = self.picker.as_ref().map(|picker| picker.try_recv());
        let picker_changed = match result {
            Some(Ok(result)) => {
                self.picker = None;
                match result {
                    Ok(Some(path)) => if let Err(error) = self.session.add_attachment(&path) { self.session.report(error); },
                    Ok(None) => {},
                    Err(error) => self.session.report(error),
                }
                true
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.picker = None;
                self.session.report("附件选择器意外退出，请重试。".into());
                true
            }
            _ => false,
        };
        changed || picker_changed || previous_phase != self.session.phase()
    }

    pub fn reconcile_models(&mut self, models: &ModelSettings) {
        if let Err(error) = self.session.reconcile_model_catalog(models.available_models()) { self.session.report(error); }
    }

    pub fn dispatch(&mut self, action: ChatAction, models: &ModelSettings) -> bool {
        match action {
            ChatAction::New => self.session.new_conversation(),
            ChatAction::Select(id) => self.session.select_conversation(&id),
            ChatAction::Delete(id) => self.session.delete_conversation(&id),
            ChatAction::SelectModel(model) => {
                let result = models.available_models().iter().find(|option| option.model == model).ok_or_else(|| "聊天模型已失效，请刷新目录后重选。".to_owned())
                    .and_then(|option| self.session.select_model_pair(option, None));
                if let Err(error) = result { self.session.report(error); }
            }
            ChatAction::SelectModelPair { model, effort } => {
                let result = models.available_models().iter().find(|option| option.model == model).ok_or_else(|| "聊天模型已失效，请刷新目录后重选。".to_owned())
                    .and_then(|option| self.session.select_model_pair(option, effort));
                if let Err(error) = result { self.session.report(error); }
            }
            ChatAction::ToggleFavorite(model) => {
                if models.available_models().iter().any(|option| option.model == model) { self.session.toggle_model_favorite(model); }
                else { self.session.report("聊天模型已失效，无法收藏。".into()); }
            }
            ChatAction::SetDefaultModelPair { model, effort } => {
                let result = models.available_models().iter().find(|option| option.model == model).ok_or_else(|| "聊天模型已失效，无法设为默认。".to_owned())
                    .and_then(|option| self.session.set_default_model_pair(option, effort));
                if let Err(error) = result { self.session.report(error); }
            }
            ChatAction::RefreshModels { .. } => self.session.report("模型刷新必须由目录宿主执行。".into()),
            ChatAction::Draft(text) => self.session.set_draft(text),
            ChatAction::Stop => self.session.cancel(),
            ChatAction::Save => self.session.retry_save(),
            ChatAction::SetComposer(settings) => self.session.set_composer(settings),
            ChatAction::SetToolbar(tools) => self.session.set_toolbar(tools),
            ChatAction::UploadAttachment => {
                if self.picker.is_none() {
                    let (sender, receiver) = mpsc::channel();
                    let wake = self.wake.clone();
                    match std::thread::Builder::new().name("attachment-picker".into()).spawn(move || {
                        let _ = sender.send(lumakit_gpui::pick_attachment());
                        wake.store(true, Ordering::Release);
                    }) {
                        Ok(_) => self.picker = Some(receiver),
                        Err(error) => self.session.report(format!("无法打开附件选择器：{error}")),
                    }
                }
            }
            ChatAction::RemoveAttachment(id) => self.session.remove_attachment(&id),
            ChatAction::OpenMedia(id) => {
                if let Err(error) = self.session.media_path(&id).and_then(|path| lumakit_gpui::open_media(&path)) { self.session.report(error); }
            }
            ChatAction::OpenSettings => return true,
            ChatAction::SubmitEdit { conversation_id, turn_id, attempt_id, expected_text, next_text } => {
                if let Err(error) = self.session.edit_attempt(&conversation_id, &turn_id, &attempt_id, &expected_text, &next_text) { self.session.report(error); }
            }
            ChatAction::DeleteAttempt { conversation_id, turn_id, attempt_id } => {
                if let Err(error) = self.session.delete_attempt(&conversation_id, &turn_id, &attempt_id) { self.session.report(error); }
            }
            ChatAction::SelectAttempt { conversation_id, turn_id, attempt_id } => {
                if let Err(error) = self.session.select_attempt(&conversation_id, &turn_id, &attempt_id) { self.session.report(error); }
            }
            ChatAction::ForkAttempt { conversation_id, turn_id, attempt_id } => {
                if let Err(error) = self.session.fork_attempt(&conversation_id, &turn_id, &attempt_id) { self.session.report(error); }
            }
            ChatAction::CopyConversation { conversation_id } => {
                if let Err(error) = self.session.copy_conversation(&conversation_id) { self.session.report(error); }
            }
            ChatAction::Send | ChatAction::Retry => {
                if self.picker.is_some() { return false; }
                let retry = matches!(action, ChatAction::Retry);
                let prepared = (|| {
                    let mut chat_model = self.session.request_model(retry)?;
                    let legacy_image = models.model_settings(&chat_model).is_some_and(|settings| settings.model_type == lumakit_ai::model_config::ModelType::Image);
                    if legacy_image { chat_model = self.session.history().model.clone().ok_or("请先选择聊天模型，再重试图片工具。")?; }
                    let chat = models.connection(&chat_model)?;
                    let painting = self.session.request_painting_model(retry)?.map(|model| models.image_connection(&model)).transpose()?;
                    Ok::<_, String>((chat_model, chat, painting, legacy_image))
                })();
                match prepared {
                    Ok((model, chat, Some(painting), true)) => { self.session.select_model(model); self.session.retry_with_model_and_painting(chat, painting); }
                    Ok((_, chat, Some(painting), false)) => if retry { self.session.retry_with_painting(chat, painting); } else { self.session.send_with_painting(chat, painting); },
                    Ok((_, chat, None, _)) => if retry { self.session.retry(chat); } else { self.session.send(chat); },
                    Err(error) => self.session.report(error),
                }
            }
            ChatAction::RetryWith(model) => {
                let prepared = models.connection(&model).and_then(|chat| self.session.request_painting_model(true)?.map(|painting| models.image_connection(&painting)).transpose().map(|painting| (chat, painting)));
                match prepared {
                    Ok((chat, painting)) => {
                        self.session.select_model(model);
                        if let Some(painting) = painting { self.session.retry_with_model_and_painting(chat, painting); }
                        else { self.session.retry_with(chat); }
                    }
                    Err(error) => self.session.report(error),
                }
            }
            ChatAction::EditAttempt { .. } => self.session.report("请在回答编辑窗口提交修改。".into()),
            ChatAction::TranslateAttempt { .. } | ChatAction::SubmitTranslation { .. } => self.session.report("翻译宿主尚未接入模型选择".into()),
            ChatAction::SaveNote { .. } | ChatAction::InsertMarkdown { .. } | ChatAction::ExportMarkdown { .. } => self.session.report("该宿主操作尚未接入。".into()),
        }
        false
    }
}
