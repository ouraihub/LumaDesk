//! Product-owned connection resolution and actions; the session remains UI-neutral.
use crate::model_settings::ModelSettings;
use lumakit_ai::chat::ChatAction;
use lumakit_ai::chat::ChatSession;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

struct Picker {
    receiver: mpsc::Receiver<Result<Option<PathBuf>, String>>,
    thread: std::thread::JoinHandle<()>,
}

pub struct ChatState {
    pub session: ChatSession,
    pub usage_status: lumakit_usage_status::ContextStatus,
    wake: Arc<AtomicBool>,
    picker: Option<Picker>,
}

impl ChatState {
    pub fn new(root: &Path) -> Result<Self, String> {
        let wake = Arc::new(AtomicBool::new(false));
        let signal = wake.clone();
        let session = crate::settings_task::action(
            "chat.load",
            "CHAT_LOAD_FAILED",
            "无法读取或迁移聊天记录；请检查权限、配置版本或备份，原文件未覆盖",
            || {
                ChatSession::open(
                    &root.join("chat.db"),
                    Arc::new(move || {
                        signal.store(true, Ordering::Release);
                    }),
                )
            },
        )?;
        Ok(Self {
            session,
            usage_status: lumakit_usage_status::ContextStatus::default(),
            wake,
            picker: None,
        })
    }

    pub fn is_busy(&self) -> bool {
        self.session.busy() || self.picker.is_some()
    }
    pub fn picking(&self) -> bool {
        self.picker.is_some()
    }
    pub fn cancel(&mut self) {
        self.session.cancel();
    }
    pub fn can_close(&mut self) -> bool {
        if self.picker.is_some() {
            self.session
                .report("请先关闭文件选择窗口，再退出应用。".into());
            return false;
        }
        if !self.session.can_close() {
            return false;
        }
        self.session.close().is_ok()
    }

    pub fn poll(&mut self) -> bool {
        let changed = self.wake.swap(false, Ordering::AcqRel);
        let previous_phase = self.session.phase();
        self.session.poll();
        let result = self
            .picker
            .as_ref()
            .map(|picker| picker.receiver.try_recv());
        let picker_changed = match result {
            Some(Ok(result)) => {
                if let Some(picker) = self.picker.take()
                    && picker.thread.join().is_err()
                {
                    tracing::warn!(
                        code = "ATTACHMENT_PICKER_PANICKED",
                        "chat.attachment.worker_failed"
                    );
                }
                match result {
                    Ok(Some(path)) => {
                        if self.session.add_attachment(&path).is_err() {
                            self.report_operation_error(
                                "chat.attachment.add",
                                "CHAT_ATTACHMENT_ADD_FAILED",
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(error) => self.session.report(error),
                }
                true
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                if let Some(picker) = self.picker.take()
                    && picker.thread.join().is_err()
                {
                    tracing::warn!(
                        code = "ATTACHMENT_PICKER_PANICKED",
                        "chat.attachment.worker_failed"
                    );
                }
                self.session.report("附件选择器意外退出，请重试。".into());
                true
            }
            _ => false,
        };
        changed || picker_changed || previous_phase != self.session.phase()
    }
    pub fn refresh_usage(&mut self, models: &ModelSettings) {
        let settings = self
            .session
            .history()
            .model
            .as_ref()
            .and_then(|m| models.model_settings(m));
        self.usage_status = crate::chat_usage::status(&self.session, settings);
    }

    pub fn reconcile_models(&mut self, models: &ModelSettings) {
        if let Err(error) = self
            .session
            .reconcile_model_catalog(models.available_models())
        {
            self.session.report(error);
        }
    }

    fn report_operation_error(&mut self, operation: &'static str, code: &'static str) {
        if self.session.save_failed() {
            return;
        }
        let (trace, root) = lumakit_observability::TraceContext::root(operation, "ui");
        root.fail(code);
        self.session.report(format!(
            "操作未完成；请检查已保存的模型、配置或当前记录后重试。\n诊断 TraceId：{}",
            trace.trace_id()
        ));
    }

    pub fn dispatch(&mut self, action: ChatAction, models: &ModelSettings) -> bool {
        match action {
            ChatAction::New => self.session.new_conversation(),
            ChatAction::Select(id) => self.session.select_conversation(&id),
            ChatAction::Delete(id) => self.session.delete_conversation(&id),
            ChatAction::SelectModel(model) => {
                let result = models
                    .available_models()
                    .iter()
                    .find(|option| option.model == model)
                    .ok_or_else(|| "聊天模型已失效，请刷新目录后重选。".to_owned())
                    .and_then(|option| self.session.select_model_pair(option, None));
                if result.is_err() {
                    self.report_operation_error("chat.model.select", "CHAT_MODEL_SELECT_FAILED");
                }
            }
            ChatAction::SelectModelPair { model, effort } => {
                let result = models
                    .available_models()
                    .iter()
                    .find(|option| option.model == model)
                    .ok_or_else(|| "聊天模型已失效，请刷新目录后重选。".to_owned())
                    .and_then(|option| self.session.select_model_pair(option, effort));
                if result.is_err() {
                    self.report_operation_error("chat.model.select", "CHAT_MODEL_SELECT_FAILED");
                }
            }
            ChatAction::ToggleFavorite(model) => {
                if models
                    .available_models()
                    .iter()
                    .any(|option| option.model == model)
                {
                    self.session.toggle_model_favorite(model);
                } else {
                    self.session.report("聊天模型已失效，无法收藏。".into());
                }
            }
            ChatAction::SetDefaultModelPair { model, effort } => {
                let result = models
                    .available_models()
                    .iter()
                    .find(|option| option.model == model)
                    .ok_or_else(|| "聊天模型已失效，无法设为默认。".to_owned())
                    .and_then(|option| self.session.set_default_model_pair(option, effort));
                if result.is_err() {
                    self.report_operation_error("chat.model.default", "CHAT_MODEL_DEFAULT_FAILED");
                }
            }
            ChatAction::RefreshModels { .. } => {
                self.session.report("模型刷新必须由目录宿主执行。".into())
            }
            ChatAction::Draft(text) => self.session.set_draft(text),
            ChatAction::Stop => self.session.cancel(),
            ChatAction::Save => self.session.retry_save(),
            ChatAction::SetComposer(settings) => self.session.set_composer(settings),
            ChatAction::SetToolbar(tools) => self.session.set_toolbar(tools),
            ChatAction::UploadAttachment => {
                if self.picker.is_none() {
                    let (sender, receiver) = mpsc::channel();
                    let wake = self.wake.clone();
                    match std::thread::Builder::new()
                        .name("attachment-picker".into())
                        .spawn(move || {
                            let (trace, root) = lumakit_observability::TraceContext::root(
                                "chat.attachment.pick",
                                "ui",
                            );
                            let _entered = root.span().clone().entered();
                            let result = lumakit_gpui::pick_attachment();
                            let result = match result {
                                Ok(None) => {
                                    root.cancel("user_cancelled");
                                    Ok(None)
                                }
                                Ok(path) => {
                                    root.finish("SELECTED");
                                    Ok(path)
                                }
                                Err(_) => {
                                    root.fail("CHAT_ATTACHMENT_PICK_FAILED");
                                    Err(format!(
                                        "无法选择附件；请检查系统文件选择器。\n诊断 TraceId：{}",
                                        trace.trace_id()
                                    ))
                                }
                            };
                            if sender.send(result).is_err() {
                                tracing::debug!(
                                    reason = "view_closed",
                                    "chat.attachment.discarded"
                                );
                            }
                            wake.store(true, Ordering::Release);
                        }) {
                        Ok(thread) => self.picker = Some(Picker { receiver, thread }),
                        Err(_) => self.report_operation_error(
                            "chat.attachment.pick",
                            "CHAT_ATTACHMENT_WORKER_START_FAILED",
                        ),
                    }
                }
            }
            ChatAction::RemoveAttachment(id) => self.session.remove_attachment(&id),
            ChatAction::OpenMedia(id) => {
                let result = crate::settings_task::action(
                    "chat.media.open",
                    "CHAT_MEDIA_OPEN_FAILED",
                    "无法打开图片；请检查文件是否存在及系统打开方式",
                    || {
                        self.session
                            .media_path(&id)
                            .and_then(|path| lumakit_gpui::open_media(&path))
                    },
                );
                if let Err(error) = result {
                    self.session.report(error);
                }
            }
            ChatAction::OpenSettings => return true,
            ChatAction::SubmitEdit {
                conversation_id,
                turn_id,
                attempt_id,
                expected_text,
                next_text,
            } => {
                if self
                    .session
                    .edit_attempt(
                        &conversation_id,
                        &turn_id,
                        &attempt_id,
                        &expected_text,
                        &next_text,
                    )
                    .is_err()
                {
                    self.report_operation_error("chat.attempt.edit", "CHAT_ATTEMPT_EDIT_FAILED");
                }
            }
            ChatAction::DeleteAttempt {
                conversation_id,
                turn_id,
                attempt_id,
            } => {
                if self
                    .session
                    .delete_attempt(&conversation_id, &turn_id, &attempt_id)
                    .is_err()
                {
                    self.report_operation_error(
                        "chat.attempt.delete",
                        "CHAT_ATTEMPT_DELETE_FAILED",
                    );
                }
            }
            ChatAction::SelectAttempt {
                conversation_id,
                turn_id,
                attempt_id,
            } => {
                if self
                    .session
                    .select_attempt(&conversation_id, &turn_id, &attempt_id)
                    .is_err()
                {
                    self.report_operation_error(
                        "chat.attempt.select",
                        "CHAT_ATTEMPT_SELECT_FAILED",
                    );
                }
            }
            ChatAction::ForkAttempt {
                conversation_id,
                turn_id,
                attempt_id,
            } => {
                if self
                    .session
                    .fork_attempt(&conversation_id, &turn_id, &attempt_id)
                    .is_err()
                {
                    self.report_operation_error("chat.attempt.fork", "CHAT_ATTEMPT_FORK_FAILED");
                }
            }
            ChatAction::CopyConversation { conversation_id } => {
                if self.session.copy_conversation(&conversation_id).is_err() {
                    self.report_operation_error(
                        "chat.conversation.copy",
                        "CHAT_CONVERSATION_COPY_FAILED",
                    );
                }
            }
            ChatAction::Send | ChatAction::Retry => {
                if self.picker.is_some() {
                    return false;
                }
                let retry = matches!(action, ChatAction::Retry);
                let prepared = (|| {
                    let mut chat_model = self.session.request_model(retry)?;
                    let legacy_image = models.model_settings(&chat_model).is_some_and(|settings| {
                        settings.model_type == lumakit_ai::model_config::ModelType::Image
                    });
                    if legacy_image {
                        chat_model = self
                            .session
                            .history()
                            .model
                            .clone()
                            .ok_or("请先选择聊天模型，再重试图片工具。")?;
                    }
                    let chat = models.connection(&chat_model)?;
                    let painting = self
                        .session
                        .request_painting_model(retry)?
                        .map(|model| models.image_connection(&model))
                        .transpose()?;
                    Ok::<_, String>((chat_model, chat, painting, legacy_image))
                })();
                match prepared {
                    Ok((model, chat, Some(painting), true)) => {
                        self.session.select_model(model);
                        self.session.retry_with_model_and_painting(chat, painting);
                    }
                    Ok((_, chat, Some(painting), false)) => {
                        if retry {
                            self.session.retry_with_painting(chat, painting);
                        } else {
                            self.session.send_with_painting(chat, painting);
                        }
                    }
                    Ok((_, chat, None, _)) => {
                        if retry {
                            self.session.retry(chat);
                        } else {
                            self.session.send(chat);
                        }
                    }
                    Err(_) => self.report_operation_error("chat.prepare", "CHAT_PREPARE_FAILED"),
                }
            }
            ChatAction::RetryWith(model) => {
                let prepared = models.connection(&model).and_then(|chat| {
                    self.session
                        .request_painting_model(true)?
                        .map(|painting| models.image_connection(&painting))
                        .transpose()
                        .map(|painting| (chat, painting))
                });
                match prepared {
                    Ok((chat, painting)) => {
                        self.session.select_model(model);
                        if let Some(painting) = painting {
                            self.session.retry_with_model_and_painting(chat, painting);
                        } else {
                            self.session.retry_with(chat);
                        }
                    }
                    Err(_) => self.report_operation_error("chat.prepare", "CHAT_PREPARE_FAILED"),
                }
            }
            ChatAction::EditAttempt { .. } => {
                self.session.report("请在回答编辑窗口提交修改。".into())
            }
            ChatAction::TranslateAttempt { .. } | ChatAction::SubmitTranslation { .. } => {
                self.session.report("翻译宿主尚未接入模型选择".into())
            }
            ChatAction::SaveNote { .. }
            | ChatAction::InsertMarkdown { .. }
            | ChatAction::ExportMarkdown { .. } => {
                self.session.report("该宿主操作尚未接入。".into())
            }
        }
        false
    }
}
