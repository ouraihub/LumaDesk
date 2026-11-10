//! Search startup loading and one-time legacy import share one root operation.
use super::*;
use crate::settings_task::{self as task, Failure};
use lumakit_observability::TraceContext;

impl SearchSettings {
    pub(super) fn load_traced(root: &Path, trace: &TraceContext) -> Result<Self, Failure> {
        let data = task::sync(
            trace,
            "settings.search.store.load",
            "SETTINGS_STORE_READ_FAILED",
            || settings_store::load_search(root),
        )?;
        let mut stored = match data {
            Some(data) => Stored::from_database(data)
                .map_err(|message| Failure::failed("SETTINGS_STORE_INVALID_DATA", message))?,
            None => {
                let value = task::sync(
                    trace,
                    "settings.search.legacy.load",
                    "SETTINGS_IMPORT_FAILED",
                    || match lumakit_support::load_versioned::<Stored>(&root.join(FILE), 1)
                        .map_err(|_| "无法读取网络搜索设置；原文件未修改".to_owned())?
                    {
                        Some((1, value)) => Ok(value),
                        Some(_) => Err("网络搜索设置版本不受支持；未覆盖原文件".into()),
                        None => Ok(Stored::default()),
                    },
                )?;
                value.save_traced(root, trace)?;
                value
            }
        };
        let mut added = false;
        for provider in PROVIDERS {
            if !stored
                .configs
                .iter()
                .any(|config| config.provider == provider)
            {
                stored.configs.push(Config {
                    provider,
                    endpoint: provider.endpoint().into(),
                    keys: Vec::new(),
                    username: String::new(),
                    selected_key: None,
                });
                added = true;
            }
        }
        if added {
            stored.save_traced(root, trace)?;
        }
        let search_index = stored
            .configs
            .iter()
            .position(|c| c.provider == stored.default_search)
            .ok_or_else(|| Failure::failed("SETTINGS_STORE_INVALID_DATA", "缺少默认搜索配置"))?;
        let fetch_index = stored
            .configs
            .iter()
            .position(|c| c.provider == stored.default_fetch)
            .ok_or_else(|| Failure::failed("SETTINGS_STORE_INVALID_DATA", "缺少默认读取配置"))?;
        let count = stored.configs.len();
        Ok(Self {
            root: root.into(),
            drafts: stored.configs.iter().map(Draft::new).collect(),
            preferences: stored.preferences.clone(),
            compression: stored.compression,
            blacklist: stored.preferences.blacklist.join("\n"),
            stored,
            search_index,
            fetch_index,
            query: String::new(),
            target: String::new(),
            result: None,
            page: None,
            status: String::new(),
            job: None,
            diagnostic_trace: None,
            card_diagnostic_trace: vec![None; count],
            inputs: None,
            advanced: false,
            delete_key: None,
            key_dialog: None,
            card_status: vec![String::new(); count],
            card_error: vec![false; count],
            previous_focus: None,
        })
    }
}
