use super::*;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable as _};
use std::{cell::RefCell, fs, rc::Rc, thread};

#[test]
fn key_hint_never_reveals_short_keys_or_the_middle() {
    assert_eq!(masked_hint(""), "");
    assert_eq!(masked_hint("abc"), "已输入密钥");
    assert_eq!(masked_hint("synthetic-demo-key"), "synt······-key");
    assert!(!masked_hint("synthetic-demo-key").contains("hetic-demo"));
}

#[test]
fn user_facing_errors_preserve_recovery_guidance() {
    assert_eq!(
        user_error(&switcher_core::Error(
            "Encryption key is missing. Restore vault.key; existing ciphertext was not changed."
        )),
        "本地加密密钥缺失。请恢复 vault.key；工具未修改现有密文。"
    );
    assert_eq!(
        user_error(&switcher_core::Error("Invalid API base URL.")),
        "Base URL 格式无效，请检查协议和主机名。"
    );
}

/// Builds the same immediate-mode controls as the native window with synthetic files.
fn harness_for(
    home: PathBuf,
    dialogs: FileDialogs,
) -> (Harness<'static>, Rc<RefCell<SwitcherApp>>) {
    let paths = match Paths::resolve(Some(home)) {
        Ok(paths) => paths,
        Err(error) => panic!("could not resolve synthetic UI directory: {error}"),
    };
    let app = Rc::new(RefCell::new(SwitcherApp::with_dialogs(
        Store::isolated(paths),
        true,
        dialogs,
    )));
    let ui_app = Rc::clone(&app);
    let harness = Harness::builder()
        .with_size([720.0, 600.0])
        .allow_missing_glyphs()
        .build_ui(move |ui| ui_app.borrow_mut().run_ui(ui));
    (harness, app)
}

/// Lets worker-thread file operations finish before the next UI action is asserted.
fn wait_until_idle(harness: &mut Harness<'static>, app: &Rc<RefCell<SwitcherApp>>) {
    for _ in 0..200 {
        harness.run();
        if !app.borrow().busy {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("synthetic UI operation did not finish");
}

fn click_button(harness: &mut Harness, label: &str) {
    let button = harness.get_by_role_and_label(Role::Button, label);
    button.scroll_to_me();
    harness.run();
    let button = harness.get_by_role_and_label(Role::Button, label);
    button.click();
    harness.run();
}

#[test]
fn controls_initialize_import_preview_and_switch_both_modes() {
    let temp = match tempfile::tempdir() {
        Ok(temp) => temp,
        Err(error) => panic!("could not create synthetic UI directory: {error}"),
    };
    let home = temp.path().join("codex-home");
    if let Err(error) = crate::demo::create(&home) {
        panic!("could not create synthetic UI configuration: {error}");
    }
    let api_config = temp.path().join("custom-api.toml");
    let api_auth = temp.path().join("custom-api-auth.json");
    if let Err(error) = fs::write(
        &api_config,
        "model_provider = \"CustomApi\"\nmodel = \"imported-ui-model\"\nmodel_reasoning_effort = \"medium\"\n[model_providers.CustomApi]\nname = \"Synthetic gateway\"\nbase_url = \"https://gateway.example.invalid/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n",
    ) {
        panic!("could not create synthetic API configuration: {error}");
    }
    if let Err(error) = fs::write(&api_auth, r#"{"OPENAI_API_KEY":"synthetic-ui-key"}"#) {
        panic!("could not create synthetic API authentication: {error}");
    }
    let config_for_dialog = api_config.clone();
    let auth_for_dialog = api_auth.clone();
    let (mut harness, app) = harness_for(
        home.clone(),
        FileDialogs {
            pick_file: Arc::new(move |auth| {
                Some(if auth {
                    auth_for_dialog.clone()
                } else {
                    config_for_dialog.clone()
                })
            }),
            pick_folder: Arc::new(|| None),
        },
    );
    wait_until_idle(&mut harness, &app);

    assert!(app.borrow().status.is_some());
    click_button(&mut harness, "初始化模式");
    wait_until_idle(&mut harness, &app);
    assert!(
        app.borrow()
            .status
            .as_ref()
            .is_some_and(|status| status.initialized)
    );

    click_button(&mut harness, "导入配置 TOML");
    wait_until_idle(&mut harness, &app);
    click_button(&mut harness, "导入认证 JSON");
    wait_until_idle(&mut harness, &app);
    let api_profile = app
        .borrow()
        .status
        .as_ref()
        .and_then(|status| {
            status
                .modes
                .iter()
                .find(|profile| profile.mode == Mode::Api)
        })
        .cloned();
    assert!(api_profile.is_some_and(|profile| {
        profile.ready
            && profile.provider == "CustomApi"
            && profile.model == "imported-ui-model"
            && profile.config_source == "custom-api.toml"
            && profile.auth_source == "custom-api-auth.json"
    }));

    click_button(&mut harness, "API key");
    app.borrow_mut().key = Zeroizing::new("synthetic-input-ui-key".into());
    click_button(&mut harness, "预览变更");
    wait_until_idle(&mut harness, &app);
    assert!(app.borrow().prepared.is_some());
    assert_eq!(
        app.borrow()
            .prepared
            .as_ref()
            .map(|prepared| prepared.preview.auth_source.as_str()),
        Some("user input")
    );
    assert_eq!(
        app.borrow().status.as_ref().and_then(|status| status.mode),
        Some(Mode::Subscription)
    );
    click_button(&mut harness, "应用切换");
    assert!(
        app.borrow().busy,
        "apply button did not start work: {}",
        app.borrow().message
    );
    wait_until_idle(&mut harness, &app);
    assert_eq!(
        app.borrow().status.as_ref().and_then(|status| status.mode),
        Some(Mode::Api),
        "{}",
        app.borrow().message
    );
    assert!(app.borrow().message.contains("AUTOCODE_API_KEY"));
    let active_config = fs::read_to_string(home.join("config.toml"));
    assert!(active_config.is_ok_and(|text| {
        text.contains("model = \"imported-ui-model\"")
            && text.contains("[features]")
            && text.contains("[projects.\"F:/example\"]")
    }));
    let active_auth = fs::read_to_string(home.join("auth.json"));
    assert!(active_auth.is_ok_and(|text| text.contains("synthetic-input-ui-key")));
    assert!(app.borrow().key.is_empty());

    click_button(&mut harness, "订阅");
    click_button(&mut harness, "预览变更");
    wait_until_idle(&mut harness, &app);
    click_button(&mut harness, "应用切换");
    wait_until_idle(&mut harness, &app);
    assert_eq!(
        app.borrow().status.as_ref().and_then(|status| status.mode),
        Some(Mode::Subscription)
    );
    let active_config = fs::read_to_string(home.join("config.toml"));
    assert!(active_config.is_ok_and(|text| text.contains("model = \"demo-subscription-model\"")));
    let active_auth = fs::read_to_string(home.join("auth.json"));
    assert!(active_auth.is_ok_and(|text| text.contains("synthetic-access")));
}

#[test]
fn cancelled_file_and_folder_pickers_keep_controls_responsive() {
    let temp = match tempfile::tempdir() {
        Ok(temp) => temp,
        Err(error) => panic!("could not create synthetic UI directory: {error}"),
    };
    let home = temp.path().join("codex-home");
    if let Err(error) = crate::demo::create(&home) {
        panic!("could not create synthetic UI configuration: {error}");
    }
    let paths_before =
        ["config.toml", "auth.json"].map(|name| fs::read(home.join(name)).unwrap_or_default());
    let (mut harness, app) = harness_for(
        home,
        FileDialogs {
            pick_file: Arc::new(|_| None),
            pick_folder: Arc::new(|| None),
        },
    );
    wait_until_idle(&mut harness, &app);

    click_button(&mut harness, "导入配置 TOML");
    wait_until_idle(&mut harness, &app);
    assert!(app.borrow().message.contains("文件选择已取消"));
    click_button(&mut harness, "选择目录");
    wait_until_idle(&mut harness, &app);
    assert!(
        app.borrow().message.contains("目录选择已取消"),
        "{}",
        app.borrow().message
    );
    assert!(!app.borrow().busy);
    assert_eq!(
        paths_before,
        ["config.toml", "auth.json"]
            .map(|name| { fs::read(app.borrow().store.paths.home.join(name)).unwrap_or_default() })
    );
    click_button(&mut harness, "刷新状态");
    wait_until_idle(&mut harness, &app);
    assert!(app.borrow().status.is_some());
}
