//! LumaDesk's appearance storage: `lumakit_design::appearance::Preferences` (schema 2) in
//! `settings.db`, migrated from LumaDesk's schema 1 palette; the page itself is Kit's
//! [`AppearancePage`].
use crate::settings_store;
use gpui_kit::{App, Context, Render, Window, prelude::*};
use lumakit_design::ThemeCatalog;
use lumakit_design::appearance::{Accent, AppearancePage, Density, Preferences};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Schema 1 (LumaDesk's own palette), read only to migrate.
mod legacy {
    use serde::Deserialize;
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
    pub(super) enum Theme {
        Light,
        Dark,
        System,
        NativeLight,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
    pub(super) enum Accent {
        Red,
        Blue,
        Green,
        Yellow,
        Cyan,
        Purple,
        Gray,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
    pub(super) enum Background {
        Warm,
        White,
    }
    #[derive(Clone, Copy, Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct Preferences {
        pub theme: Theme,
        pub accent: Accent,
        pub background: Background,
        pub density: super::Density,
    }
}

/// Light/Dark × Warm/White pick the matching catalog theme; NativeLight was Kit's light
/// controls, now `light`; System follows with the background's pair. Accent and density
/// carry over unchanged (the old palette always had an accent, so it stays an override).
fn migrate(old: legacy::Preferences) -> Preferences {
    use legacy::{Background::*, Theme::*};
    let (light, dark) = match old.background {
        Warm => ("warm-light", "warm-dark"),
        White => ("light", "dark"),
    };
    let theme_id = match old.theme {
        Light | System => light,
        Dark => dark,
        NativeLight => "light",
    };
    let accent = match old.accent {
        legacy::Accent::Red => Accent::Red,
        legacy::Accent::Blue => Accent::Blue,
        legacy::Accent::Green => Accent::Green,
        legacy::Accent::Yellow => Accent::Yellow,
        legacy::Accent::Cyan => Accent::Cyan,
        legacy::Accent::Purple => Accent::Purple,
        legacy::Accent::Gray => Accent::Gray,
    };
    Preferences {
        theme_id: theme_id.to_owned(),
        follow_system: old.theme == System,
        light_theme: light.to_owned(),
        dark_theme: dark.to_owned(),
        accent,
        density: old.density,
    }
}

/// Either schema, as stored; schema 1 migrates in memory and is rewritten on the next save.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Current(Preferences),
    Legacy(legacy::Preferences),
}

/// Decodes stored preferences of either schema.
fn decode(value: serde_json::Value) -> Result<Preferences, String> {
    match serde_json::from_value::<Stored>(value)
        .map_err(|_| "外观设置 JSON 无效；原文件未修改".to_owned())?
    {
        Stored::Current(p) => Ok(p),
        Stored::Legacy(old) => Ok(migrate(old)),
    }
}

pub(crate) struct Appearance {
    root: PathBuf,
    catalog: &'static ThemeCatalog,
    saved: Preferences,
    error: Option<String>,
}

impl Appearance {
    pub(crate) fn load(root: &Path) -> Result<Self, String> {
        let catalog =
            ThemeCatalog::bundled().map_err(|error| format!("内置主题无法加载：{error}"))?;
        let saved = match settings_store::load_appearance(root)? {
            Some(value) => decode(value)?,
            None => {
                let value = match lumakit_support::load_versioned::<serde_json::Value>(
                    &root.join("appearance.json"),
                    1,
                )
                .map_err(|_| {
                    "无法读取 appearance.json；请检查版本、格式或权限，原文件未修改".to_owned()
                })? {
                    None => Preferences::default(),
                    Some((1, value)) => decode(value)?,
                    Some(_) => return Err("外观设置版本不受支持；原文件未修改".into()),
                };
                settings_store::save_appearance(
                    root,
                    &serde_json::to_value(&value).map_err(|_| "外观设置序列化失败".to_owned())?,
                )?;
                value
            }
        };
        Ok(Self {
            root: root.to_owned(),
            catalog,
            saved,
            error: None,
        })
    }
    fn save(&mut self, next: Preferences) -> Result<(), String> {
        settings_store::save_appearance(
            &self.root,
            &serde_json::to_value(&next).map_err(|_| "外观设置序列化失败".to_owned())?,
        )
        .map_err(|_| {
            "保存外观失败；请检查目录权限或磁盘空间。原设置保持不变，可重新选择重试".to_owned()
        })?;
        self.saved = next;
        Ok(())
    }

    /// Installs the saved appearance; the app calls this again when the window's appearance changes.
    pub(crate) fn apply(&self, window: &mut Window, cx: &mut App) {
        self.saved.apply(self.catalog, window, cx);
    }

    fn choose(&mut self, next: Preferences, window: &mut Window, cx: &mut Context<Self>) {
        self.error = self.save(next).err();
        if self.error.is_none() {
            self.apply(window, cx);
            cx.refresh_windows();
        }
        cx.notify();
    }
}

impl Render for Appearance {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        AppearancePage::new(self.catalog, self.saved.clone(), move |next, window, cx| {
            let _ = this.update(cx, |this, cx| this.choose(next, window, cx));
        })
        .error(self.error.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::WindowAppearance;
    use serde_json::json;

    fn catalog() -> &'static ThemeCatalog {
        ThemeCatalog::bundled().unwrap()
    }
    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "lumadesk-{name}-{}",
            lumakit_support::new_id().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
    fn legacy(theme: &str, background: &str) -> serde_json::Value {
        json!({ "theme": theme, "accent": "Cyan", "background": background, "density": "Compact" })
    }

    #[test]
    fn every_schema_1_combination_migrates_to_a_catalog_theme() {
        // (old theme, old background) -> (theme_id, follow_system, light slot, dark slot)
        let table = [
            (
                "Light",
                "Warm",
                "warm-light",
                false,
                "warm-light",
                "warm-dark",
            ),
            ("Light", "White", "light", false, "light", "dark"),
            (
                "Dark",
                "Warm",
                "warm-dark",
                false,
                "warm-light",
                "warm-dark",
            ),
            ("Dark", "White", "dark", false, "light", "dark"),
            (
                "NativeLight",
                "Warm",
                "light",
                false,
                "warm-light",
                "warm-dark",
            ),
            ("NativeLight", "White", "light", false, "light", "dark"),
            (
                "System",
                "Warm",
                "warm-light",
                true,
                "warm-light",
                "warm-dark",
            ),
            ("System", "White", "light", true, "light", "dark"),
        ];
        for (theme, background, id, follow, light, dark) in table {
            let p = decode(legacy(theme, background)).unwrap();
            assert_eq!(
                p,
                Preferences {
                    theme_id: id.into(),
                    follow_system: follow,
                    light_theme: light.into(),
                    dark_theme: dark.into(),
                    accent: Accent::Cyan,
                    density: Density::Compact
                },
                "{theme}+{background}"
            );
            for slot in [&p.theme_id, &p.light_theme, &p.dark_theme] {
                assert!(catalog().get(slot).is_some(), "{slot} must be bundled");
            }
        }
        for (old, new) in [
            ("Red", Accent::Red),
            ("Blue", Accent::Blue),
            ("Green", Accent::Green),
            ("Yellow", Accent::Yellow),
            ("Cyan", Accent::Cyan),
            ("Purple", Accent::Purple),
            ("Gray", Accent::Gray),
        ] {
            let value = json!({ "theme": "Dark", "accent": old, "background": "White", "density": "Comfortable" });
            assert_eq!(decode(value).unwrap().accent, new);
        }
        assert!(decode(json!({ "theme": "Dark", "accent": "Theme", "background": "White", "density": "Comfortable" })).is_err());
        assert!(
            decode(json!({ "theme_id": "dark" })).is_err(),
            "a partial schema 2 is corrupt, not a default"
        );
    }

    #[test]
    fn fresh_install_defaults_to_dark_with_the_theme_accent() {
        let root = temp_root("appearance-fresh");
        let appearance = Appearance::load(&root).unwrap();
        assert_eq!(appearance.saved, Preferences::default());
        assert_eq!(
            appearance
                .saved
                .resolve(catalog(), WindowAppearance::Light)
                .id,
            "dark"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn schema_1_in_the_database_loads_migrated_and_is_rewritten_only_on_save() {
        let root = temp_root("appearance-db-v1");
        settings_store::save_appearance(&root, &legacy("Dark", "Warm")).unwrap();
        let mut appearance = Appearance::load(&root).unwrap();
        assert_eq!(appearance.saved.theme_id, "warm-dark");
        assert_eq!(
            settings_store::load_appearance(&root).unwrap(),
            Some(legacy("Dark", "Warm")),
            "loading must not rewrite"
        );
        let next = Preferences {
            density: Density::Comfortable,
            ..appearance.saved.clone()
        };
        appearance.save(next.clone()).unwrap();
        assert_eq!(
            settings_store::load_appearance(&root).unwrap(),
            Some(serde_json::to_value(&next).unwrap())
        );
        assert_eq!(Appearance::load(&root).unwrap().saved, next);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persistence_failure_restart_and_corruption_preserve_preferences() {
        let root = temp_root("appearance");
        let mut appearance = Appearance::load(&root).unwrap();
        let next = Preferences {
            theme_id: "minimal-light".into(),
            accent: Accent::Purple,
            density: Density::Compact,
            ..Default::default()
        };
        appearance.save(next.clone()).unwrap();
        assert_eq!(Appearance::load(&root).unwrap().saved, next);
        let path = root.join("settings.db");
        let bytes = std::fs::read(&path).unwrap();
        let real_root = appearance.root.clone();
        appearance.root = path.clone();
        assert!(appearance.save(Preferences::default()).is_err());
        assert_eq!(appearance.saved, next);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        appearance.root = real_root;
        std::fs::write(&path, b"broken").unwrap();
        assert!(Appearance::load(&root).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_import_is_once_and_invalid_input_is_preserved() {
        let root = temp_root("appearance-import");
        let path = root.join("appearance.json");
        std::fs::write(&path, b"invalid").unwrap();
        assert!(Appearance::load(&root).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid");
        let connection = rusqlite::Connection::open(root.join("settings.db")).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM settings_import_markers", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        lumakit_support::save_versioned(&path, 1, &legacy("Dark", "White")).unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut appearance = Appearance::load(&root).unwrap();
        assert_eq!(
            appearance.saved,
            Preferences {
                theme_id: "dark".into(),
                accent: Accent::Cyan,
                density: Density::Compact,
                ..Default::default()
            }
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        appearance.save(Preferences::default()).unwrap();
        std::fs::write(&path, b"invalid but no longer a source").unwrap();
        assert_eq!(
            Appearance::load(&root).unwrap().saved,
            Preferences::default()
        );
        connection
            .execute("DELETE FROM prefs WHERE key='appearance'", [])
            .unwrap();
        assert!(Appearance::load(&root).is_err());
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}
