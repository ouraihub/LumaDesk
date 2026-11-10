//! Saved-target discovery selection and immediate model persistence.
use super::*;
use gpui_kit::component::scroll::ScrollableElement;
const SYNC_LIST: Pixels = px(440.);

impl ModelSettings {
    /// "同步模型" picker, as ThinkRail's `ModelPicker`: search with bulk add/remove, type tabs and a
    /// fixed-height scrolling list grouped by id prefix, so the dialog keeps its size (and its close
    /// button) however many models are listed or added. Every + / − saves at once.
    pub(super) fn sync_view(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let dialog = self.sync_dialog.as_ref().unwrap();
        let query = dialog.search.trim().to_lowercase();
        let kind = |id: &str| dialog.types.get(id).copied().unwrap_or(ModelType::Unknown);
        let searched: Vec<&String> = dialog
            .ids
            .iter()
            .filter(|id| id.to_lowercase().contains(&query))
            .collect();
        let shown: Vec<String> = searched
            .iter()
            .filter(|id| dialog.filter.is_none_or(|f| kind(id) == f))
            .map(|id| (*id).clone())
            .collect();
        let all_picked = !shown.is_empty() && shown.iter().all(|id| dialog.selected.contains(id));
        let ids = shown.clone();
        let bulk = self
            .button(
                "sync-all",
                if all_picked {
                    "全部移除"
                } else {
                    "全部添加"
                },
                cx,
                move |this, _, _| {
                    if let Some(dialog) = &this.sync_dialog {
                        let mut selected = dialog.selected.clone();
                        for id in &ids {
                            if all_picked {
                                selected.remove(id);
                            } else {
                                selected.insert(id.clone());
                            }
                        }
                        let types = dialog.types.clone();
                        this.save_sync(selected, types);
                    }
                },
            )
            .outline()
            .disabled(shown.is_empty());
        let tabs: Vec<_> = std::iter::once(None)
            .chain(MODEL_TYPES.map(Some))
            .enumerate()
            .map(|(index, value)| {
                let hits = searched
                    .iter()
                    .filter(|id| value.is_none_or(|t| kind(id) == t))
                    .count();
                let current = dialog.filter == value;
                self.button(
                    ("sync-filter", index),
                    format!("{} {hits}", value.map_or("全部", ModelType::label)),
                    cx,
                    move |this, _, _| {
                        if let Some(dialog) = &mut this.sync_dialog {
                            dialog.filter = value;
                        }
                    },
                )
                .small()
                .when(current, |b| b.outline())
                .when(!current, |b| b.ghost())
                .selected(current)
                .disabled(hits == 0 && value.is_some())
            })
            .collect();
        let mut groups: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for id in shown {
            let group = id.split(['-', '/', ':']).next().unwrap_or_default();
            let group = if group.is_empty() {
                "其他".to_owned()
            } else {
                group.to_owned()
            };
            groups.entry(group).or_default().push(id);
        }
        let searching = !query.is_empty();
        let list: Vec<_> = groups
            .into_iter()
            .enumerate()
            .map(|(index, (group, ids))| {
                let folded = !searching && dialog.folded.contains(&group);
                let name = group.clone();
                v_flex()
                    .child(shared::model_group(
                        self.on(
                            Button::new(format!("sync-group-{}-{group}", dialog.endpoint))
                                .accessibility_label(format!("{group}，{} 个", ids.len()))
                                .ghost()
                                .min_w(px(0.))
                                .flex_1()
                                .child(
                                    h_flex()
                                        .w_full()
                                        .min_w(px(0.))
                                        .justify_start()
                                        .gap(space::S8)
                                        .child(
                                            Icon::new(if folded {
                                                IconName::ChevronRight
                                            } else {
                                                IconName::ChevronDown
                                            })
                                            .small()
                                            .text_color(r.text_muted),
                                        )
                                        .child(
                                            label(text::UI, group.clone()).min_w(px(0.)).truncate(),
                                        )
                                        .child(
                                            label(text::METADATA, ids.len().to_string())
                                                .text_color(r.text_subtle),
                                        ),
                                ),
                            cx,
                            move |this, _, _| {
                                if let Some(dialog) = &mut this.sync_dialog
                                    && !dialog.folded.remove(&name)
                                {
                                    dialog.folded.insert(name.clone());
                                }
                            },
                        )
                        .into_any_element(),
                        div().into_any_element(),
                        index == 0,
                        &r,
                    ))
                    .when(!folded, |d| {
                        d.children(ids.iter().map(|id| {
                            self.sync_row(id, dialog.selected.contains(id), kind(id), cx)
                        }))
                    })
            })
            .collect();
        let list = if list.is_empty() {
            div()
                .h(SYNC_LIST)
                .child(shared::empty("没有匹配的模型", &r))
                .into_any_element()
        } else {
            v_flex()
                .id("sync-list")
                .h(SYNC_LIST)
                .gap(space::S4)
                .overflow_y_scrollbar()
                .children(list)
                .into_any_element()
        };
        let close = self.on(
            Button::new("sync-close")
                .ghost()
                .small()
                .icon(IconName::X)
                .accessibility_label("关闭")
                .tooltip("关闭"),
            cx,
            |this, _, _| this.sync_dialog = None,
        );
        let count = dialog
            .ids
            .iter()
            .filter(|id| dialog.selected.contains(*id))
            .count();
        let body = shared::form_fields([
            hint(
                format!(
                    "发现 {} 个；点 + 添加，点 − 移除；即时保存，不改动本次目录之外的模型。",
                    dialog.ids.len()
                ),
                &r,
            )
            .into_any_element(),
            h_flex()
                .gap(space::S8)
                .child(
                    self.input("sync-search")
                        .prefix(Icon::new(IconName::Search).small())
                        .flex_1(),
                )
                .child(bulk)
                .into_any_element(),
            h_flex()
                .gap(space::S4)
                .flex_wrap()
                .children(tabs)
                .into_any_element(),
            list,
        ]);
        shared::form_body(
            "挑选模型",
            body.into_any_element(),
            shared::form_footer(None, close.into_any_element(), None)
                .child(hint(format!("已选 {count} 个"), &r))
                .into_any_element(),
            self.status(cx).map(IntoElement::into_any_element),
            &r,
        )
    }
    /// One listed model (ThinkRail `render_pick`): id, type menu, 已选 mark, and + / − that saves at once.
    fn sync_row(
        &self,
        id: &str,
        selected: bool,
        kind: ModelType,
        cx: &Context<Self>,
    ) -> gpui_kit::Div {
        let r = roles(cx);
        let (page, model) = (cx.entity(), id.to_owned());
        let types = ui::select(
            Button::new(format!("sync-type-{id}"))
                .ghost()
                .small()
                .label(kind.label())
                .icon(IconName::ChevronDown)
                .accessibility_label(format!("{id} 的类型：{}", kind.label())),
            Some(kind),
            MODEL_TYPES.map(|value| (value, value.label().into())),
            move |value, _, cx| {
                page.update(cx, |this, cx| {
                    if let Some(dialog) = &mut this.sync_dialog {
                        if dialog.selected.contains(&model) {
                            let selected = dialog.selected.clone();
                            let mut types = dialog.types.clone();
                            types.insert(model.clone(), value);
                            this.save_sync(selected, types);
                        } else {
                            dialog.types.insert(model.clone(), value);
                        }
                    }
                    cx.notify();
                })
            },
        );
        let model = id.to_owned();
        let toggle = Button::new(format!("sync-select-{id}"))
            .ghost()
            .small()
            .icon(if selected {
                IconName::Minus
            } else {
                IconName::Plus
            })
            .accessibility_label(format!(
                "{} {id}",
                if selected {
                    "移除模型"
                } else {
                    "添加模型"
                }
            ))
            .tooltip(if selected {
                "移除模型"
            } else {
                "添加模型"
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(dialog) = &this.sync_dialog {
                    let mut selected = dialog.selected.clone();
                    if !selected.remove(&model) {
                        selected.insert(model.clone());
                    }
                    let types = dialog.types.clone();
                    this.save_sync(selected, types);
                }
                cx.notify();
            }));
        shared::picker_row(
            h_flex()
                .gap(space::S8)
                .min_w(px(0.))
                .flex_1()
                .child(
                    label(text::CODE_TEXT, id.to_owned())
                        .flex_1()
                        .min_w(px(0.))
                        .truncate(),
                )
                .child(types)
                .child(
                    label(text::METADATA, if selected { "已选" } else { "" })
                        .w(px(32.))
                        .text_color(r.feedback_success),
                )
                .into_any_element(),
            toggle.into_any_element(),
            selected,
            &r,
        )
    }
}
