use super::*;
use crate::settings_task::{self as task, Failure};
use lumakit_observability::TraceContext;

impl ModelSettings {
    pub(super) fn open_editor(
        &mut self,
        editor: EditorDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.job.is_some() {
            return;
        }
        self.key_drafts.clear();
        self.set_input("key", "", window, cx);
        self.set_input("key-label", "", window, cx);
        if let Some(view) = &mut self.view {
            view.model = None;
        }
        match &editor {
            EditorDialog::Provider { name, .. } => self.set_input("name", name.clone(), window, cx),
            EditorDialog::Endpoint { name, url, .. } => {
                self.set_input("name", name.clone(), window, cx);
                self.set_input("editor-url", url.clone(), window, cx);
            }
            EditorDialog::Model {
                provider,
                endpoint,
                original,
                value,
                ..
            } => {
                let protocol = self
                    .draft
                    .providers
                    .iter()
                    .find(|p| p.id == *provider)
                    .and_then(|p| p.endpoints.iter().find(|e| e.id == *endpoint))
                    .map_or(Protocol::OpenAiChat, |e| e.protocol);
                let model = cx.new(|cx| {
                    ModelEditor::new(value.clone(), protocol, original.is_some(), window, cx)
                });
                self.view.as_mut().unwrap().model = Some(model);
            }
            EditorDialog::Keys { .. } => {}
        }
        self.editor = Some(editor);
        self.notice.clear();
        self.error = false;
    }
    pub(super) fn edit_endpoint(
        &mut self,
        p: usize,
        e: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_endpoint(p, e);
        let editor = self.editor.take().unwrap();
        self.open_editor(editor, window, cx);
    }
    pub(super) fn close_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        self.view.as_mut().unwrap().model = None;
        self.key_drafts.clear();
        self.set_input("key", "", window, cx);
        self.set_input("key-label", "", window, cx);
    }
    pub(super) fn save_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let (trace, root) = TraceContext::root(
            match &editor {
                EditorDialog::Provider { .. } => "settings.provider.save",
                EditorDialog::Endpoint { .. } => "settings.endpoint.save",
                EditorDialog::Model { .. } => "settings.model.save",
                EditorDialog::Keys { .. } => "settings.keys.save",
            },
            "ui",
        );
        let _entered = root.span().clone().entered();
        if let EditorDialog::Model { value, .. } = &mut editor
            && let Some(model) = &self.view.as_ref().unwrap().model
        {
            if let Err(error) = model.read(cx).validate() {
                let result = Err::<(), _>(Failure::failed("SETTINGS_INPUT_INVALID", error));
                self.report_task(&trace, &result, "");
                self.editor = Some(editor);
                task::end(root, &result);
                return;
            }
            *value = model.read(cx).value.clone();
        }
        let before = self.draft.clone();
        let provider = self.provider.clone();
        let endpoint = self.endpoint.clone();
        let result = self
            .apply_editor(&editor)
            .map_err(|message| Failure::failed("SETTINGS_INPUT_INVALID", message))
            .and_then(|_| self.save_in(&trace));
        if self.report_task(&trace, &result, "已保存") {
            self.close_editor(window, cx);
            self.refresh_endpoint(window, cx);
        } else {
            self.draft = before;
            self.provider = provider;
            self.endpoint = endpoint;
            self.editor = Some(editor);
        }
        task::end(root, &result);
    }
    pub(super) fn save_key_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submit_key() {
            self.set_input("key", "", window, cx);
            self.set_input("key-label", "", window, cx);
        }
    }
    pub(super) fn delete_confirmed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (trace, root) = TraceContext::root(
            if self.delete_key.is_some() {
                "settings.keys.delete"
            } else if self.delete_endpoint.is_some() {
                "settings.endpoint.delete"
            } else {
                "settings.provider.delete"
            },
            "ui",
        );
        let _entered = root.span().clone().entered();
        let before = self.draft.clone();
        if let Some(id) = &self.delete_provider {
            self.draft.providers.retain(|p| p.id != *id);
        }
        if let Some(id) = &self.delete_endpoint {
            for p in &mut self.draft.providers {
                p.endpoints.retain(|e| e.id != *id);
            }
        }
        if let Some((provider, endpoint, key)) = &self.delete_key
            && let Some(endpoint) = self
                .draft
                .providers
                .iter_mut()
                .find(|p| &p.id == provider)
                .and_then(|p| p.endpoints.iter_mut().find(|e| &e.id == endpoint))
        {
            endpoint.keys.retain(|k| &k.id != key);
            if endpoint.selected_key.as_ref() == Some(key) {
                endpoint.selected_key = endpoint.keys.first().map(|k| k.id.clone());
            }
        }
        let result = if let Some((provider, endpoint, _)) = &self.delete_key {
            let indices = self
                .draft
                .providers
                .iter()
                .position(|p| &p.id == provider)
                .and_then(|p| {
                    self.draft.providers[p]
                        .endpoints
                        .iter()
                        .position(|e| &e.id == endpoint)
                        .map(|e| (p, e))
                });
            match indices {
                Some((p, e)) => self.persist_keys(&trace, p, e),
                None => Err(Failure::failed("SETTINGS_TARGET_MISSING", "线路已不存在")),
            }
        } else {
            self.save_in(&trace)
        };
        if self.report_task(&trace, &result, "已删除") {
            self.delete_provider = None;
            self.delete_endpoint = None;
            self.delete_key = None;
            self.repair_selection();
            self.refresh_endpoint(window, cx);
        } else {
            self.draft = before;
        }
        task::end(root, &result);
    }
    pub(super) fn edit_models(
        &mut self,
        p: usize,
        e: usize,
        selected: Option<String>,
        removed: &[String],
    ) {
        let before = self.draft.providers[p].clone();
        if self.job.is_some() {
            return;
        }
        let (trace, root) = TraceContext::root("settings.models.edit", "ui");
        let _entered = root.span().clone().entered();
        let endpoint = &mut self.draft.providers[p].endpoints[e];
        if let Some(id) = selected {
            endpoint.selected_model = Some(id);
        }
        endpoint.models.retain(|m| !removed.contains(&m.id));
        repair_model_selection(endpoint);
        let result = self.save_in(&trace);
        if !self.report_task(&trace, &result, "已保存") {
            self.draft.providers[p] = before;
        }
        task::end(root, &result);
    }
}
