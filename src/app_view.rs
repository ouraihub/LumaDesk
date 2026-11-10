//! Desktop shell composition: tabs, settings navigation and the active application.
use super::*;
impl Render for DesktopApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let narrow = window.viewport_size().width < size::NARROW;
        let r = LumaTheme::global(cx).roles;
        // The tab strip is the window title bar. Clickable parts occlude the drag area so a click
        // reaches them; the remaining empty space moves the window.
        let mut tabs = div()
            .id("app-tabs")
            .occlude()
            .flex()
            .items_center()
            .gap(space::S4)
            .min_w_0()
            .overflow_x_scroll();
        for (index, tab) in self.tabs.iter().copied().enumerate() {
            let title = match tab {
                TabKind::Launchpad => "启动台",
                TabKind::App(app) => APPS[app],
            };
            tabs = tabs.child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .child(
                        Button::new(("tab", index))
                            .label(title)
                            .small()
                            .ghost()
                            .selected(self.settings.is_none() && index == self.active)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.leave_settings(window, cx);
                                this.active = index;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("close-tab", index))
                            .icon(IconName::X)
                            .accessibility_label(format!("关闭{title}"))
                            .tooltip(format!("关闭{title}"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_tab(index, window, cx)
                            })),
                    ),
            );
        }
        let top = title_bar(r).child(
            div()
                .flex()
                .items_center()
                .flex_1()
                .min_w_0()
                .h_full()
                .gap(space::S4)
                .child(tabs)
                .child(
                    div().id("new-tab-area").occlude().child(
                        Button::new("new-tab")
                            .icon(IconName::Plus)
                            .accessibility_label("打开启动台")
                            .tooltip("打开启动台")
                            .small()
                            .ghost()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.launchpad(window, cx)),
                            ),
                    ),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("title-actions")
                        .occlude()
                        .flex()
                        .items_center()
                        .gap(space::S4)
                        .child(
                            Button::new("appearance")
                                .icon(IconName::Palette)
                                .label("外观")
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_settings(SettingsPage::Appearance, window, cx)
                                })),
                        )
                        .child(
                            Button::new("settings")
                                .icon(IconName::Settings)
                                .label("设置")
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_settings(SettingsPage::Models, window, cx)
                                })),
                        ),
                ),
        );
        let content = if let Some(page) = self.settings {
            let query = self.settings_query.read(cx).value().trim().to_lowercase();
            // Narrow windows stack: the same items become a wrapping row above the page.
            let mut nav = if narrow { ui::nav_row(&r) } else { ui::nav(&r) }
                .child(ui::nav_item(
                    "settings-return",
                    IconName::ArrowLeft,
                    "返回",
                    false,
                    &r,
                    cx.listener(|this, _, window, cx| {
                        this.leave_settings(window, cx);
                        cx.notify();
                    }),
                ))
                .child(
                    div()
                        .when(narrow, |d| d.w(ui::INLINE_SEARCH))
                        .py(space::S4)
                        .child(Input::new(&self.settings_query).small()),
                );
            let mut found = false;
            for (section, (group, items)) in SETTINGS.iter().enumerate() {
                let matches: Vec<_> = items
                    .iter()
                    .copied()
                    .filter(|(_, _, title, terms)| {
                        query.is_empty() || title.contains(&query) || terms.contains(&query)
                    })
                    .collect();
                if matches.is_empty() {
                    continue;
                }
                if !narrow {
                    nav = nav.child(ui::nav_section(*group, !found, &r));
                }
                found = true;
                for (item, (target, icon, title, _)) in matches.into_iter().enumerate() {
                    nav = nav.child(if target == SettingsPage::Usage {
                        Button::new("settings-usage")
                            .small()
                            .ghost()
                            .icon(icon)
                            .label(title)
                            .accessibility_label("AI 设置：用量统计")
                            .selected(page == target)
                            .toggled(page == target)
                            .when(!narrow, |button| button.w_full())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_settings(target, window, cx)
                            }))
                            .into_any_element()
                    } else {
                        ui::nav_item(
                            ("settings-page", section * 10 + item),
                            icon,
                            title,
                            page == target,
                            &r,
                            cx.listener(move |this, _, window, cx| {
                                this.open_settings(target, window, cx)
                            }),
                        )
                    });
                }
            }
            if !found {
                nav = nav.child(
                    label(text::METADATA, "没有匹配的设置项")
                        .text_color(r.text_subtle)
                        .py(space::S4),
                );
            }
            let page_view = match page {
                SettingsPage::Models => self.models.clone().into_any_element(),
                SettingsPage::Search => self.search.clone().into_any_element(),
                SettingsPage::Mcp => self.mcp.clone().into_any_element(),
                SettingsPage::Usage => self.usage_page(cx),
                SettingsPage::Appearance => self.appearance.clone().into_any_element(),
                SettingsPage::Logs => self.logs.clone().into_any_element(),
            };
            (if narrow {
                ui::stacked(nav, page_view)
            } else {
                ui::shell(nav, page_view)
            })
            .into_any_element()
        } else {
            let page = match self.tabs[self.active] {
                TabKind::App(0) => {
                    let panel = self.panel.clone();
                    self.models.update(cx, |models, cx| {
                        panel.update(cx, |panel, cx| {
                            panel.set_external_busy(self.chat.picking());
                            panel.set_model_refresh_state(
                                models.refreshing_catalog(),
                                models.catalog_refresh_notice().to_owned(),
                                models.catalog_refresh_trace_id().map(str::to_owned),
                                cx,
                            );
                            let metadata = self
                                .chat
                                .session
                                .history()
                                .model
                                .as_ref()
                                .and_then(|model| models.model_settings(model));
                            panel.set_usage_status(self.chat.usage_status.clone(), cx);
                            panel.render_session(
                                &self.chat.session,
                                models.available_models(),
                                models.image_models(),
                                metadata,
                                window,
                                cx,
                            )
                        })
                    })
                }
                TabKind::Launchpad => {
                    let mut tiles = div().flex().flex_wrap().gap(space::S12);
                    for (app, title) in APPS.iter().copied().enumerate() {
                        tiles = tiles.child(
                            Button::new(("launch", app))
                                .label(title)
                                .icon(app_icon(app))
                                .w(TILE_W)
                                .h(TILE_H)
                                .bg(r.container_elevated_bg)
                                .border_1()
                                .border_color(r.border_muted)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.launch(app, window, cx)
                                })),
                        );
                    }
                    div()
                        .id("launchpad")
                        .size_full()
                        .overflow_y_scroll()
                        .p(space::S24)
                        .flex()
                        .flex_col()
                        .gap(space::S24)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(space::S4)
                                .child(label(text::HEADING_SM, "启动台"))
                                .child(
                                    label(
                                        text::METADATA,
                                        "选择应用；已打开的应用会出现在左侧和上方。",
                                    )
                                    .text_color(r.text_subtle),
                                ),
                        )
                        .child(tiles)
                        .into_any_element()
                }
                TabKind::App(app) => div()
                    .p(space::S24)
                    .flex()
                    .flex_col()
                    .gap(space::S4)
                    .child(label(text::HEADING_SM, APPS[app]))
                    .child(label(text::METADATA, "应用业务尚未接入。").text_color(r.text_subtle))
                    .into_any_element(),
            };
            let mut row = div().flex().size_full().min_h_0();
            if !narrow {
                let mut rail = div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(space::S8)
                    .w(RAIL)
                    .flex_shrink_0()
                    .py(space::S8)
                    .bg(r.container_sidebar_bg)
                    .border_r_1()
                    .border_color(r.border_muted)
                    .child(
                        ActivityItem::new("home", IconName::LayoutDashboard, "启动台")
                            .active(self.tabs[self.active] == TabKind::Launchpad)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.launchpad(window, cx)),
                            ),
                    );
                for (index, tab) in self.tabs.iter().copied().enumerate() {
                    if let TabKind::App(app) = tab {
                        rail = rail.child(
                            ActivityItem::new(("rail", index), app_icon(app), APPS[app])
                                .active(index == self.active)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.leave_settings(window, cx);
                                    this.active = index;
                                    cx.notify();
                                })),
                        );
                    }
                }
                rail = rail.child(div().flex_1()).child(
                    ActivityItem::new("rail-settings", IconName::Settings, "设置").on_click(
                        cx.listener(|this, _, window, cx| {
                            this.open_settings(SettingsPage::Models, window, cx)
                        }),
                    ),
                );
                row = row.child(rail);
            }
            row.child(div().flex_1().min_w_0().h_full().min_h_0().child(page))
                .into_any_element()
        };
        root(r)
            .child(top)
            .child(div().flex_1().min_h_0().child(content))
    }
}
