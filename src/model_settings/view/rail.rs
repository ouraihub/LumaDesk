use super::*;

impl ModelSettings {
    /// The provider rail: search with a status filter menu, one row per provider, then 新建服务商.
    pub(super) fn providers_view(&self, cx: &Context<Self>) -> gpui_kit::Div {
        let r = roles(cx);
        let page = cx.entity();
        let current = self.provider_filter;
        let filter = Button::new("provider-filter")
            .outline()
            .icon(IconName::SlidersHorizontal)
            .accessibility_label("筛选平台")
            .tooltip("筛选平台")
            .dropdown_menu(move |menu, _, _| {
                PROVIDER_FILTERS
                    .into_iter()
                    .fold(menu, |menu, (value, title)| {
                        let page = page.clone();
                        menu.item(
                            PopupMenuItem::new(title)
                                .checked(current == value)
                                .on_click(move |_, _, cx| {
                                    page.update(cx, |this, cx| {
                                        this.provider_filter = value;
                                        cx.notify();
                                    })
                                }),
                        )
                    })
            });
        let search = self
            .input("provider-search")
            .prefix(Icon::new(IconName::Search).small())
            .flex_1();
        let query = self.search.trim().to_lowercase();
        let rows: Vec<AnyElement> = self
            .draft
            .providers
            .iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&query)
                    && (self.provider_filter == 0 || p.enabled == (self.provider_filter == 1))
            })
            .map(|provider| self.provider_row(provider, &r, cx))
            .collect();
        let mut rows = rows;
        if rows.is_empty() {
            rows.push(shared::empty("没有匹配的平台", &r));
        }
        shared::rail(
            search.into_any_element(),
            filter.into_any_element(),
            rows,
            self.button("new-provider", "新建服务商", cx, |this, window, cx| {
                this.open_editor(
                    EditorDialog::Provider {
                        id: None,
                        name: String::new(),
                    },
                    window,
                    cx,
                )
            })
            .w_full()
            .disabled(self.job.is_some())
            .icon(IconName::Plus)
            .into_any_element(),
        )
    }
    /// Avatar, name (muted when disabled), status dot (green with a saved key, amber without, none when disabled), ··· menu.
    pub(super) fn provider_row(
        &self,
        provider: &ProviderEntry,
        r: &Roles,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = provider.id.clone();
        let selected = self.provider.as_ref() == Some(&provider.id);
        let has_key = provider.endpoints.iter().any(|e| !e.keys.is_empty());
        let page = cx.entity();
        let selection = page.clone();
        let (edit, remove) = (provider.id.clone(), provider.id.clone());
        let menu = ui::row_menu_button(
            format!("provider-menu-{id}"),
            format!("{} 的操作", provider.name),
        )
        .disabled(self.job.is_some())
        .dropdown_menu(move |menu, _, _| {
            let (page_edit, page_remove, edit, remove) =
                (page.clone(), page.clone(), edit.clone(), remove.clone());
            menu.item(
                PopupMenuItem::new("服务商设置")
                    .icon(IconName::Pencil)
                    .on_click(move |_, window, cx| {
                        page_edit.update(cx, |this, cx| {
                            if let Some(p) = this.draft.providers.iter().find(|p| p.id == edit) {
                                let name = p.name.clone();
                                this.open_editor(
                                    EditorDialog::Provider {
                                        id: Some(edit.clone()),
                                        name,
                                    },
                                    window,
                                    cx,
                                );
                            }
                            cx.notify();
                        })
                    }),
            )
            .item(
                PopupMenuItem::new("删除服务商…")
                    .icon(IconName::Trash)
                    .on_click(move |_, _, cx| {
                        page_remove.update(cx, |this, cx| {
                            this.delete_provider = Some(remove.clone());
                            cx.notify();
                        })
                    }),
            )
        });
        shared::provider_row(
            format!("provider-{id}"),
            provider.name.clone(),
            selected,
            provider.enabled,
            has_key,
            menu.into_any_element(),
            r,
        )
        .disabled(self.job.is_some())
        .on_select(move |window, cx| {
            selection.update(cx, |this, cx| {
                if this.job.is_some() {
                    return;
                }
                if this.provider.as_ref() == Some(&id) {
                    return;
                }
                if this.dirty {
                    this.report(
                        Err("当前地址尚未保存，请修正或重试保存后再切换平台".into()),
                        "",
                    );
                    cx.notify();
                    return;
                }
                this.provider = Some(id.clone());
                this.repair_selection();
                this.model_query.clear();
                this.set_input("model-search", "", window, cx);
                this.notice.clear();
                this.refresh_endpoint(window, cx);
                cx.notify();
            })
        })
        .into_any_element()
    }
}
