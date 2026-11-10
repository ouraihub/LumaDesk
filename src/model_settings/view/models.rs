use super::*;

impl ModelSettings {
    pub(super) fn models_view(&self, p: usize, e: usize, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let endpoint = &self.draft.providers[p].endpoints[e];
        let query = self.model_query.trim().to_lowercase();
        let mut groups: Vec<(String, Vec<&ModelEntry>)> = Vec::new();
        for model in &endpoint.models {
            if !(model.id.to_lowercase().contains(&query)
                || model
                    .settings
                    .display_name(&model.id)
                    .to_lowercase()
                    .contains(&query))
            {
                continue;
            }
            let effective = model.effective_settings(endpoint.protocol);
            if !match self.capability_filter {
                1 => effective.supports_vision,
                2 => effective.supports_reasoning(),
                3 => effective.supports_tools == Some(true),
                4 => effective.native_web_search,
                _ => true,
            } {
                continue;
            }
            let group = model.settings.group_name(&model.id);
            if let Some((_, models)) = groups.iter_mut().find(|(key, _)| key == group) {
                models.push(model);
            } else {
                groups.push((group.into(), vec![model]));
            }
        }
        let names: Vec<_> = groups.iter().map(|(g, _)| g.clone()).collect();
        let body = if groups.is_empty() {
            shared::empty(
                if endpoint.models.is_empty() {
                    "未选择模型"
                } else {
                    "没有匹配的模型"
                },
                &r,
            )
        } else {
            let rows = groups
                .into_iter()
                .enumerate()
                .flat_map(|(index, (group, models))| {
                    self.model_group(p, e, index, group, models, cx)
                });
            shared::model_list(rows, &r).into_any_element()
        };
        v_flex()
            .gap(space::S8)
            .child(self.models_toolbar(p, e, &names, cx))
            .child(body)
    }
    /// 模型, fold-all, inline search, capability menu; 同步模型 and manual add on the right.
    pub(super) fn models_toolbar(
        &self,
        p: usize,
        e: usize,
        names: &[String],
        cx: &Context<Self>,
    ) -> gpui_kit::Div {
        let all_folded = !names.is_empty() && names.iter().all(|group| self.folded.contains(group));
        let names = names.to_vec();
        let fold = self
            .icon(
                "fold-all",
                if all_folded {
                    IconName::ChevronsUpDown
                } else {
                    IconName::ChevronsDownUp
                },
                if all_folded {
                    "展开全部分组"
                } else {
                    "收起全部分组"
                },
                cx,
                move |this, _, _| {
                    if all_folded {
                        this.folded.clear();
                    } else {
                        this.folded.extend(names.iter().cloned());
                    }
                },
            )
            .disabled(self.draft.providers[p].endpoints[e].models.is_empty());
        let page = cx.entity();
        let current = self.capability_filter;
        let title = CAPABILITY_FILTERS
            .iter()
            .find(|(v, _)| *v == current)
            .map_or("全部", |(_, t)| *t);
        let capability = Button::new("cap-filter")
            .ghost()
            .small()
            .label(title)
            .icon(IconName::ChevronDown)
            .accessibility_label(format!("按能力筛选：{title}"))
            .dropdown_menu(move |menu, _, _| {
                CAPABILITY_FILTERS
                    .into_iter()
                    .fold(menu, |menu, (value, title)| {
                        let page = page.clone();
                        menu.item(
                            PopupMenuItem::new(title)
                                .checked(current == value)
                                .on_click(move |_, _, cx| {
                                    page.update(cx, |this, cx| {
                                        this.capability_filter = value;
                                        cx.notify();
                                    })
                                }),
                        )
                    })
            });
        let add = self.on(
            Button::new("new-model")
                .outline()
                .icon(IconName::Plus)
                .accessibility_label("手动添加模型")
                .tooltip("手动添加模型"),
            cx,
            move |this, window, cx| {
                this.open_editor(
                    EditorDialog::Model {
                        provider: this.draft.providers[p].id.clone(),
                        endpoint: this.draft.providers[p].endpoints[e].id.clone(),
                        original: None,
                        value: ModelEntry {
                            id: String::new(),
                            capabilities: unknown_capabilities(),
                            manual: true,
                            settings: Default::default(),
                        },
                    },
                    window,
                    cx,
                )
            },
        );
        shared::models_toolbar(
            fold.into_any_element(),
            Some(
                self.input("model-search")
                    .prefix(Icon::new(IconName::Search).small())
                    .small()
                    .w(ui::INLINE_SEARCH)
                    .max_w_full()
                    .min_w(px(0.))
                    .into_any_element(),
            ),
            capability.into_any_element(),
            self.button("sync-models", "同步模型", cx, |this, window, cx| {
                this.set_input("sync-search", "", window, cx);
                this.start_job(true);
            })
            .outline()
            .icon(IconName::RefreshCw)
            .disabled(self.dirty || self.job.is_some())
            .into_any_element(),
            add.disabled(self.job.is_some()).into_any_element(),
        )
    }
    /// One model group: a fold header with its count and remove-all, then a row per model.
    pub(super) fn model_group(
        &self,
        p: usize,
        e: usize,
        index: usize,
        group: String,
        models: Vec<&ModelEntry>,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let r = roles(cx);
        let folded = self.folded.contains(&group);
        let name = group.clone();
        let removed: Vec<_> = models.iter().map(|m| m.id.clone()).collect();
        let title = if group.is_empty() {
            "未分组"
        } else {
            &group
        };
        let toggle = self.on(
            Button::new(format!(
                "model-group-{}-{group}",
                self.draft.providers[p].endpoints[e].id
            ))
            .accessibility_label(format!("{title}，{} 个", models.len()))
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
                    .child(label(text::UI, title.to_owned()).min_w(px(0.)).truncate())
                    .child(
                        label(text::METADATA, models.len().to_string()).text_color(r.text_subtle),
                    ),
            ),
            cx,
            move |this, _, _| {
                if !this.folded.remove(&name) {
                    this.folded.insert(name.clone());
                }
            },
        );
        let remove = self
            .icon(
                ("remove-group", index),
                IconName::Minus,
                "移除分组模型",
                cx,
                move |this, _, _| this.edit_models(p, e, None, &removed),
            )
            .disabled(self.job.is_some());
        let header = shared::model_group(
            toggle.into_any_element(),
            remove.into_any_element(),
            index == 0,
            &r,
        );
        let mut rows = vec![header.into_any_element()];
        if !folded {
            rows.extend(
                models
                    .into_iter()
                    .map(|model| self.model_row(p, e, model, cx).into_any_element()),
            );
        }
        rows
    }
    /// A model: name, capability tags and id · type (click selects it); settings and remove on the right.
    pub(super) fn model_row(
        &self,
        p: usize,
        e: usize,
        model: &ModelEntry,
        cx: &Context<Self>,
    ) -> gpui_kit::Div {
        let r = roles(cx);
        let endpoint = &self.draft.providers[p].endpoints[e];
        let selected = endpoint.selected_model.as_ref() == Some(&model.id);
        let name = model.settings.display_name(&model.id).to_owned();
        let id = model.id.clone();
        let effective = model.effective_settings(endpoint.protocol);
        let select = self.on(
            Button::new(format!("select-model-{}-{id}", endpoint.id))
                .accessibility_label(name.clone())
                .ghost()
                .disabled(self.job.is_some())
                .justify_start()
                .h_auto()
                .min_w(px(0.))
                .flex_1()
                .child(
                    h_flex()
                        .gap(space::S8)
                        .min_w(px(0.))
                        .flex_1()
                        .child(ui::avatar(&name, &r))
                        .child(
                            v_flex()
                                .min_w(px(0.))
                                .flex_1()
                                .child(label(text::UI, name).min_w(px(0.)).truncate())
                                .child(
                                    hint(
                                        format!(
                                            "{} · {}",
                                            model.id,
                                            model.settings.model_type.label()
                                        ),
                                        &r,
                                    )
                                    .min_w(px(0.))
                                    .truncate(),
                                ),
                        )
                        .child(lumakit_gpui::model_capability_badges(&effective, r)),
                ),
            cx,
            move |this, _, _| this.edit_models(p, e, Some(id.clone()), &[]),
        );
        let value = model.clone();
        let id = model.id.clone();
        shared::model_row(
            select.into_any_element(),
            [
                self.on(
                    Button::new(format!("edit-model-{}-{id}", endpoint.id))
                        .small()
                        .ghost()
                        .disabled(self.job.is_some())
                        .icon(IconName::Settings2)
                        .accessibility_label(format!("编辑模型 {id}"))
                        .tooltip("模型设置"),
                    cx,
                    move |this, window, cx| {
                        this.open_editor(
                            EditorDialog::Model {
                                provider: this.draft.providers[p].id.clone(),
                                endpoint: this.draft.providers[p].endpoints[e].id.clone(),
                                original: Some(value.id.clone()),
                                value: value.clone(),
                            },
                            window,
                            cx,
                        )
                    },
                )
                .into_any_element(),
                self.icon(
                    format!("remove-model-{}-{id}", endpoint.id),
                    IconName::Minus,
                    "移除模型",
                    cx,
                    move |this, _, _| this.edit_models(p, e, None, std::slice::from_ref(&id)),
                )
                .disabled(self.job.is_some())
                .into_any_element(),
            ],
            selected,
            &r,
        )
    }
}
