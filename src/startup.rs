//! Product composition and configuration loading; failure owners live at each domain boundary.
use super::*;

impl DesktopApp {
    /// `font_error`: why the bundled fonts could not be registered (logged once logging runs).
    pub(super) fn create(
        font_error: Option<String>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Entity<Self>, String> {
        let root = match std::env::var_os("LUMADESK_DATA_DIR") {
            Some(path) => PathBuf::from(path),
            None => std::env::current_exe()
                .map_err(|_| "无法定位程序目录".to_owned())?
                .parent()
                .ok_or("无法定位程序目录")?
                .join("data-gpui"),
        };
        std::fs::create_dir_all(&root)
            .map_err(|_| "无法创建配置目录，请将程序移动到可写目录".to_owned())?;
        let data_lock = lumakit_support::FileLock::acquire(&root.join("lumadesk.lock"))
            .map_err(|_| "此数据目录已在使用，或无法加锁；请关闭另一个实例后重试".to_owned())?;
        let logs = lumakit_observability::LogHandle::init(
            &root.join("logs"),
            lumakit_observability::LogLevel::Info,
            5 * 1024 * 1024,
        )
        .map_err(|_| "无法初始化诊断日志；请检查日志配置、目录权限或磁盘空间".to_owned())?;
        if font_error.is_some() {
            tracing::warn!(code = "BUNDLED_FONTS_UNAVAILABLE", "app.fonts.fallback");
        }
        let appearance = theme::Appearance::load(&root)?;
        appearance.apply(window, cx);
        let appearance = cx.new(|_| appearance);
        let models = model_settings::ModelSettings::create(&root, window, cx)?;
        let search = search_settings::SearchSettings::create(&root, window, cx)?;
        let mcp = mcp_settings::McpSettings::create(&root, window, cx)?;
        let logs = log_settings::LogSettings::create(logs, window, cx);
        let mut chat = chat::ChatState::new(&root)?;
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            ui = "gpui-kit",
            "app.started"
        );
        chat.reconcile_models(models.read(cx));
        chat.refresh_usage(models.read(cx));
        let panel = cx.new(|cx| ChatPanel::new(window, cx));
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("搜索设置"));
        let app = cx.new(|cx: &mut Context<Self>| {
            let subscriptions = vec![
                cx.subscribe_in(
                    &panel,
                    window,
                    |this, _, action: &ChatAction, window, cx| {
                        if let ChatAction::RefreshModels { force } = action {
                            this.models.update(cx, |models, cx| {
                                models.refresh_catalog(*force);
                                cx.notify();
                            });
                        } else {
                            let open_settings =
                                this.chat.dispatch(action.clone(), this.models.read(cx));
                            if open_settings {
                                this.open_settings(SettingsPage::Models, window, cx);
                            }
                        }
                        this.chat.refresh_usage(this.models.read(cx));
                        this.ensure_background(window, cx);
                        cx.notify();
                    },
                ),
                cx.observe(&panel, |_, _, cx| cx.notify()),
                cx.observe_in(&models, window, |this, _, window, cx| {
                    this.chat.reconcile_models(this.models.read(cx));
                    this.chat.refresh_usage(this.models.read(cx));
                    this.ensure_background(window, cx);
                    cx.notify();
                }),
                cx.observe_in(&search, window, |this, _, window, cx| {
                    this.ensure_background(window, cx);
                    cx.notify();
                }),
                cx.observe(&appearance, |_, _, cx| cx.notify()),
                cx.observe(&logs, |_, _, cx| cx.notify()),
                cx.subscribe(&query, |_, _, _: &InputEvent, cx| cx.notify()),
                cx.observe_window_appearance(window, |this, window, cx| {
                    this.appearance
                        .update(cx, |appearance, cx| appearance.apply(window, cx));
                    cx.notify();
                }),
            ];
            Self {
                _data_lock: data_lock,
                logs,
                models,
                search,
                mcp,
                appearance,
                chat,
                panel,
                usage: usage_settings::UsageSettings::default(),
                tabs: vec![TabKind::Launchpad],
                active: 0,
                settings: None,
                settings_query: query,
                _subscriptions: subscriptions,
                background: None,
            }
        });
        let weak = app.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| {
                if this.chat.can_close() {
                    this.leave_settings(window, cx);
                    tracing::info!("app.closed");
                    true
                } else {
                    this.launch(0, window, cx);
                    this.ensure_background(window, cx);
                    cx.notify();
                    false
                }
            })
            .unwrap_or(true)
        });
        Ok(app)
    }
}
