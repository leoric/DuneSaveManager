//! Settings live in one settings.toml; older JSON files are migrated silently on first load.
//! Single test function: it points the config dir at a temp folder through XDG_CONFIG_HOME (Linux/macOS-style
//! lookup used by `dirs`), which is process-global.
#![cfg(target_os = "linux")]

use dune_save_manager::*;
use std::{fs, path::PathBuf, sync::{Mutex, MutexGuard}};

/// Tests share the process environment, so they run one at a time.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn fresh_dir(tag: &str) -> (MutexGuard<'static, ()>, PathBuf) {
    let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let base = std::env::temp_dir().join(format!("dsm-test-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("DuneSaveManager")).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &base);
    (guard, base.join("DuneSaveManager"))
}

#[test]
fn migrates_v1_0_0_json_files() {
    let (_g, d) = fresh_dir("v100");
    // exactly what v1.0.0 wrote (config.json + window.json)
    fs::write(
        d.join("config.json"),
        r#"{
  "saves_root": "C:\\Users\\me\\AppData\\Local\\DuneSandbox\\Saved\\Cloud\\PlayerClientStorage\\FLS_retail",
  "profile": "76561198000000000",
  "vault": "D:\\DuneVault",
  "screenshot_dirs": ["D:\\Shots", "E:\\More Shots"],
  "name_sql": "SELECT name FROM t LIMIT 1",
  "level_sql": null,
  "log_path": null
}"#,
    )
    .unwrap();
    fs::write(d.join("window.json"), r#"{"x":120.0,"y":80.0,"w":958.0,"h":600.0,"maximized":false}"#).unwrap();

    let c = load_config(); // triggers migration, no prompts
    assert_eq!(c.profile.as_deref(), Some("76561198000000000"));
    assert_eq!(c.vault, Some(PathBuf::from("D:\\DuneVault")));
    assert_eq!(c.screenshot_dirs.len(), 2);
    assert_eq!(c.name_sql.as_deref(), Some("SELECT name FROM t LIMIT 1"));
    assert_eq!(c.level_sql, None);
    let w = c.window.clone().unwrap();
    assert_eq!((w.x, w.y, w.w, w.h, w.maximized), (Some(120.0), Some(80.0), 958.0, 600.0, false));
    assert_eq!(c.ui.theme, ThemePref::Auto);

    assert!(d.join("settings.toml").exists());
    assert!(!d.join("config.json").exists() && d.join("config.json.migrated").exists());
    assert!(!d.join("window.json").exists() && d.join("window.json.migrated").exists());

    // idempotent: a second load must not touch anything, even if stale json files reappear
    fs::write(d.join("config.json"), r#"{"profile":"other"}"#).unwrap();
    assert_eq!(load_config().profile.as_deref(), Some("76561198000000000"));
    assert!(d.join("config.json").exists());
}

#[test]
fn migrates_prerelease_ui_json_and_partial_sets() {
    let (_g, d) = fresh_dir("ui");
    fs::write(d.join("ui.json"), r#"{"theme":"light"}"#).unwrap();
    let c = load_config();
    assert_eq!(c.ui.theme, ThemePref::Light);
    assert!(c.window.is_none() && c.vault.is_none());
    assert!(d.join("ui.json.migrated").exists());
}

#[test]
fn leaves_unparsable_legacy_file_and_fresh_install_untouched() {
    let (_g, d) = fresh_dir("bad");
    fs::write(d.join("config.json"), "{ not json").unwrap();
    let c = load_config();
    assert!(c.vault.is_none());
    assert!(d.join("config.json").exists(), "unparsable legacy file must not be renamed");
    assert!(!d.join("settings.toml").exists(), "nothing to migrate -> no file created");
}

#[test]
fn existing_toml_wins_and_update_config_merges() {
    let (_g, d) = fresh_dir("toml");
    fs::write(d.join("config.json"), r#"{"profile":"old"}"#).unwrap();
    update_config(|c| c.profile = Some("new".into())).unwrap(); // creates settings.toml (after migrating)
    update_config(|c| c.ui.theme = ThemePref::Dark).unwrap();
    update_config(|c| c.window = Some(WindowState { x: None, y: None, w: 900.0, h: 700.0, maximized: true })).unwrap();
    let c = load_config();
    assert_eq!(c.profile.as_deref(), Some("new"));
    assert_eq!(c.ui.theme, ThemePref::Dark);
    assert_eq!(c.window.unwrap().w, 900.0);
    let text = fs::read_to_string(d.join("settings.toml")).unwrap();
    assert!(text.contains("[ui]") && text.contains("theme = \"dark\"") && text.contains("[window]"), "{text}");
    assert!(!d.join("settings.toml.tmp").exists());
}
