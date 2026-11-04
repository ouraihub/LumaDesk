//! Model editor, deletion confirmation and discovery picker views.
use super::*;
use gpui_kit::component::scroll::ScrollableElement;

const PICK_GROUP: Pixels = px(32.);
const SYNC_LIST: Pixels = px(440.);
const SYNC_ROW: Pixels = px(36.);

impl ModelSettings {
    pub(super) fn editor_view(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let editor = self.editor.as_ref().unwrap();
        let title = match editor { EditorDialog::Provider { .. } => "服务商设置", EditorDialog::Endpoint { .. } => "线路设置", EditorDialog::Model { .. } => "模型设置", EditorDialog::Keys { .. } => "密钥管理" };
        let mut body = v_flex().w_full().gap(space::S16).child(dialog_title(title));
        let mut destructive = None;
        match editor {
            EditorDialog::Provider { id, .. } => {
                body = body.child(ui::field("名称", self.input("name"), &r));
                if let Some(id) = id { let id = id.clone(); destructive = Some(self.button("delete-provider", "删除服务商…", cx, move |this, window, cx| { this.delete_provider = Some(id.clone()); this.close_editor(window, cx); }).danger()); }
            }
            EditorDialog::Endpoint { id, protocol, url, .. } => {
                let (page, current) = (cx.entity(), *protocol);
                let protocols = ui::select(ui::select_button("endpoint-protocol", "协议", protocol_name(current)).w_full(), Some(current),
                    [Protocol::OpenAiChat, Protocol::OpenAiResponses, Protocol::AnthropicMessages].map(|p| (p, protocol_name(p).into())),
                    move |value, _, cx| page.update(cx, |this, cx| { if let Some(EditorDialog::Endpoint { protocol, .. }) = &mut this.editor { *protocol = value; } cx.notify(); }));
                let request = match model_url(url, *protocol) { Ok(url) => format!("GET {url}"), Err(error) => error };
                body = body.child(ui::field("线路名称", self.input("name"), &r)).child(ui::field("API 地址", self.input("editor-url"), &r))
                    .child(ui::field("协议", v_flex().gap(space::S8).child(protocols).child(ui::request_line(request, &r)), &r));
                if let Some(id) = id { let id = id.clone(); destructive = Some(self.button("delete-endpoint", "删除线路…", cx, move |this, window, cx| { this.delete_endpoint = Some(id.clone()); this.close_editor(window, cx); }).danger()); }
            }
            EditorDialog::Model { .. } => {
                if let Some(model) = &self.view.as_ref().unwrap().model { body = body.child(model.clone()); }
                body = body.child(hint("保存后从下一次调用生效；正在生成的回复不受影响。", &r));
            }
            EditorDialog::Keys { provider, endpoint } => {
                if let Some(p) = self.draft.providers.iter().position(|p| p.id == *provider) { if let Some(e) = self.draft.providers[p].endpoints.iter().position(|e| e.id == *endpoint) {
                    let keys = &self.draft.providers[p].endpoints[e].keys;
                    body = body.child(hint("每条线路最多 16 个，加密保存。", &r));
                    if !keys.is_empty() { body = body.child(v_flex().gap(space::S6).children(keys.iter().map(|key| self.key_row(p, e, key, cx)))); }
                    let editing = self.key_drafts.get(endpoint).is_some_and(|draft| draft.editing.is_some());
                    let mut actions = h_flex().justify_end().gap(space::S8);
                    if editing { actions = actions.child(self.button("cancel-key-edit", "取消修改", cx, |this, window, cx| { this.key_drafts.clear(); this.set_input("key", "", window, cx); }).outline()); }
                    actions = actions.child(self.button("save-key", if editing { "保存修改后的密钥" } else { "保存新密钥" }, cx, |this, window, cx| this.save_key_input(window, cx)).primary());
                    let field = ui::field(if editing { "修改密钥" } else { "添加密钥" }, self.input("key"), &r).when(editing, |d| d.child(hint("空草稿不会清空原密钥", &r)));
                    body = body.child(field).child(actions);
                } }
            }
        }
        let keys = matches!(editor, EditorDialog::Keys { .. });
        let mut footer = h_flex().gap(space::S8).children(destructive).child(div().flex_1())
            .child(self.button("close-editor", if keys { "完成" } else { "取消" }, cx, |this, window, cx| this.close_editor(window, cx)).outline());
        if !keys { footer = footer.child(self.button("save-editor", "保存", cx, |this, window, cx| this.save_editor(window, cx)).primary()); }
        body.child(footer)
    }
    /// Delete confirmation: what goes away, a quiet cancel and a destructive confirm.
    pub(super) fn delete_view(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let note = if self.delete_key.is_some() { "将删除此密钥；其它密钥、模型和线路保持不变。" } else if self.delete_provider.is_some() { "将永久删除此服务商及全部线路、模型与密钥。" } else { "将永久删除此线路及模型与密钥。" };
        v_flex().w_full().gap(space::S16)
            .child(v_flex().gap(space::S4).child(dialog_title("确认删除")).child(label(text::UI, note).text_color(r.text_muted)))
            .child(h_flex().justify_end().gap(space::S8)
                .child(self.button("cancel-delete", "取消", cx, |this, _, _| { this.delete_provider = None; this.delete_endpoint = None; this.delete_key = None; }).outline())
                .child(self.button("confirm-delete", "确认删除", cx, |this, window, cx| this.delete_confirmed(window, cx)).danger()))
    }
    /// "同步模型" picker, as ThinkRail's `ModelPicker`: search with bulk add/remove, type tabs and a
    /// fixed-height scrolling list grouped by id prefix, so the dialog keeps its size (and its close
    /// button) however many models are listed or added. Every + / − saves at once.
    pub(super) fn sync_view(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let dialog = self.sync_dialog.as_ref().unwrap(); let query = dialog.search.trim().to_lowercase();
        let kind = |id: &str| dialog.types.get(id).copied().unwrap_or(ModelType::Unknown);
        let searched: Vec<&String> = dialog.ids.iter().filter(|id| id.to_lowercase().contains(&query)).collect();
        let shown: Vec<String> = searched.iter().filter(|id| dialog.filter.is_none_or(|f| kind(id) == f)).map(|id| (*id).clone()).collect();
        let all_picked = !shown.is_empty() && shown.iter().all(|id| dialog.selected.contains(id)); let ids = shown.clone();
        let bulk = self.button("sync-all", if all_picked { "全部移除" } else { "全部添加" }, cx, move |this, _, _| {
            if let Some(dialog) = &this.sync_dialog { let mut selected = dialog.selected.clone(); for id in &ids { if all_picked { selected.remove(id); } else { selected.insert(id.clone()); } } let types = dialog.types.clone(); this.save_sync(selected, types); }
        }).outline().disabled(shown.is_empty());
        let tabs: Vec<_> = std::iter::once(None).chain(MODEL_TYPES.map(Some)).enumerate().map(|(index, value)| {
            let hits = searched.iter().filter(|id| value.is_none_or(|t| kind(id) == t)).count();
            let current = dialog.filter == value;
            self.button(("sync-filter", index), format!("{} {hits}", value.map_or("全部", ModelType::label)), cx, move |this, _, _| { if let Some(dialog) = &mut this.sync_dialog { dialog.filter = value; } })
                .small().when(current, |b| b.outline()).when(!current, |b| b.ghost()).selected(current).disabled(hits == 0 && value.is_some())
        }).collect();
        let mut groups: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        for id in shown { let group = id.split(['-', '/', ':']).next().unwrap_or_default(); let group = if group.is_empty() { "其他".to_owned() } else { group.to_owned() }; groups.entry(group).or_default().push(id); }
        let searching = !query.is_empty();
        let list: Vec<_> = groups.into_iter().enumerate().map(|(index, (group, ids))| {
            let folded = !searching && dialog.folded.contains(&group); let name = group.clone();
            v_flex().child(group_header(("sync-group", index), folded, &group, ids.len(), &r).h(PICK_GROUP).px(space::S8).rounded(radius::MD).bg(r.container_elevated_bg)
                    .on_click(cx.listener(move |this, _, _, cx| { if let Some(dialog) = &mut this.sync_dialog { if !dialog.folded.remove(&name) { dialog.folded.insert(name.clone()); } } cx.notify(); })))
                .when(!folded, |d| d.children(ids.iter().map(|id| self.sync_row(id, dialog.selected.contains(id), kind(id), cx))))
        }).collect();
        let list = if list.is_empty() { div().h(SYNC_LIST).child(ui::empty("没有匹配的模型", &r)).into_any_element() }
            else { v_flex().id("sync-list").h(SYNC_LIST).gap(space::S4).overflow_y_scrollbar().children(list).into_any_element() };
        let close = self.on(Button::new("sync-close").ghost().small().icon(IconName::X).accessibility_label("关闭").tooltip("关闭"), cx, |this, _, _| this.sync_dialog = None);
        let count = dialog.ids.iter().filter(|id| dialog.selected.contains(*id)).count();
        v_flex().w_full().gap(space::S8)
            .child(h_flex().gap(space::S8).child(dialog_title("挑选模型"))
                .child(label(text::METADATA, dialog.ids.len().to_string()).px(space::S6).rounded(radius::SM).bg(r.control_bg)).child(div().flex_1()).child(close))
            .child(hint("点 + 添加模型，点 − 移除；即时保存，不改动本次目录之外的模型。", &r))
            .child(h_flex().gap(space::S8).child(self.input("sync-search").prefix(Icon::new(IconName::Search).small()).flex_1()).child(bulk))
            .child(h_flex().gap(space::S4).flex_wrap().children(tabs))
            .child(list)
            .child(label(text::METADATA, format!("已选 {count} 个")).text_color(r.text_muted))
    }
    /// One listed model (ThinkRail `render_pick`): id, type menu, 已选 mark, and + / − that saves at once.
    fn sync_row(&self, id: &str, selected: bool, kind: ModelType, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let (page, model) = (cx.entity(), id.to_owned());
        let types = ui::select(Button::new(format!("sync-type-{id}")).ghost().small().label(kind.label()).icon(IconName::ChevronDown).accessibility_label(format!("{id} 的类型：{}", kind.label())), Some(kind),
            MODEL_TYPES.map(|value| (value, value.label().into())),
            move |value, _, cx| page.update(cx, |this, cx| {
                if let Some(dialog) = &mut this.sync_dialog {
                    if dialog.selected.contains(&model) { let selected = dialog.selected.clone(); let mut types = dialog.types.clone(); types.insert(model.clone(), value); this.save_sync(selected, types); }
                    else { dialog.types.insert(model.clone(), value); }
                }
                cx.notify();
            }));
        let model = id.to_owned();
        let toggle = Button::new(format!("sync-select-{id}")).ghost().small().icon(if selected { IconName::Minus } else { IconName::Plus })
            .accessibility_label(format!("{} {id}", if selected { "移除模型" } else { "添加模型" })).tooltip(if selected { "移除模型" } else { "添加模型" })
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(dialog) = &this.sync_dialog { let mut selected = dialog.selected.clone(); if !selected.remove(&model) { selected.insert(model.clone()); } let types = dialog.types.clone(); this.save_sync(selected, types); }
                cx.notify();
            }));
        h_flex().h(SYNC_ROW).pl(space::S16).pr(space::S4).gap(space::S8).rounded(radius::MD).hover(|s| s.bg(r.control_bg_hovered))
            .child(label(text::CODE_TEXT, id.to_owned()).flex_1().min_w(px(0.)).truncate()).child(types)
            .child(label(text::METADATA, if selected { "已选" } else { "" }).w(px(32.)).text_color(r.feedback_success)).child(toggle)
    }
}
