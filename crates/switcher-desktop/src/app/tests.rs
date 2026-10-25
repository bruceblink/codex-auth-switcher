use super::{DialogFuture, FileDialogs, Switcher};
use crate::demo;
use gpui_kit::{
    AppContext as _, Bounds, ElementId, InputEvent as _, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use switcher_core::{Mode, Paths, Store};

fn test_dialogs(
    files: Vec<(bool, Option<PathBuf>)>,
    folders: Vec<PathBuf>,
) -> (FileDialogs, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let files = Arc::new(files);
    let folders = Arc::new(folders);
    let file_index = Arc::new(AtomicUsize::new(0));
    let folder_index = Arc::new(AtomicUsize::new(0));
    let file_calls = Arc::new(AtomicUsize::new(0));
    let folder_calls = Arc::new(AtomicUsize::new(0));
    let pick_file = {
        let files = Arc::clone(&files);
        let file_index = Arc::clone(&file_index);
        let file_calls = Arc::clone(&file_calls);
        Arc::new(move |auth| {
            file_calls.fetch_add(1, Ordering::SeqCst);
            let index = file_index.fetch_add(1, Ordering::SeqCst);
            let selected = files
                .get(index)
                .filter(|(expected_auth, _)| *expected_auth == auth)
                .and_then(|(_, path)| path.clone());
            Box::pin(async move { selected }) as DialogFuture
        })
    };
    let pick_folder = {
        let folders = Arc::clone(&folders);
        let folder_index = Arc::clone(&folder_index);
        let folder_calls = Arc::clone(&folder_calls);
        Arc::new(move || {
            folder_calls.fetch_add(1, Ordering::SeqCst);
            let index = folder_index.fetch_add(1, Ordering::SeqCst);
            let folder = folders.get(index).cloned();
            Box::pin(async move { folder }) as DialogFuture
        })
    };
    (
        FileDialogs {
            pick_file,
            pick_folder,
        },
        file_calls,
        folder_calls,
    )
}

fn click(cx: &mut TestAppContext, window_handle: gpui_kit::AnyWindowHandle, id: &'static str) {
    let result = cx.update_window(window_handle, |_, window, cx| {
        window.draw(cx).clear(cx);
        let element_id: ElementId = id.into();
        let target = gpui_base::test_support::find(window, &[], &element_id);
        let Some(target) = target else {
            panic!("button '{id}' is missing from the headless view");
        };
        assert!(target.visible(), "button '{id}' is not visible");
        let position = target.bounds().center();
        window.dispatch_event(
            MouseMoveEvent {
                position,
                modifiers: Modifiers::default(),
                pressed_button: None,
            }
            .to_platform_input(),
            cx,
        );
        window.draw(cx).clear(cx);
        window.dispatch_event(
            MouseDownEvent {
                position,
                modifiers: Modifiers::default(),
                button: MouseButton::Left,
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.draw(cx).clear(cx);
        window.dispatch_event(
            MouseUpEvent {
                position,
                modifiers: Modifiers::default(),
                button: MouseButton::Left,
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.draw(cx).clear(cx);
    });
    assert!(
        result.is_ok(),
        "button '{id}' could not be clicked: {result:?}"
    );
    cx.run_until_parked();
}

fn profile(store: &Store, mode: Mode) -> switcher_core::ModeStatus {
    let status = match store.status() {
        Ok(status) => status,
        Err(error) => panic!("headless UI test could not read status: {error}"),
    };
    match status
        .modes
        .into_iter()
        .find(|profile| profile.mode == mode)
    {
        Some(profile) => profile,
        None => panic!("headless UI test did not find the {mode:?} profile"),
    }
}

#[gpui_test::test]
fn every_switcher_button_dispatches_its_action(cx: &mut TestAppContext) {
    let temp = match tempfile::tempdir() {
        Ok(temp) => temp,
        Err(error) => panic!("could not create isolated UI test directory: {error}"),
    };
    let home = temp.path().join("codex-home");
    if let Err(error) = demo::create(&home) {
        panic!("could not create synthetic Codex data: {error}");
    }

    let api_config = temp.path().join("ui-api.toml");
    let api_auth = temp.path().join("ui-api-auth.json");
    let subscription_config = temp.path().join("ui-subscription.toml");
    let subscription_auth = temp.path().join("ui-subscription-auth.json");
    let synthetic_api_config = r#"model_provider = "CustomApi"
model = "ui-imported-api-model"
model_reasoning_effort = "high"
[model_providers.CustomApi]
name = "Synthetic gateway"
base_url = "https://ui-import.example.invalid/v1"
wire_api = "responses"
requires_openai_auth = true
"#;
    let synthetic_subscription_config = r#"model_provider = "openai"
model = "ui-imported-subscription-model"
model_reasoning_effort = "medium"
cli_auth_credentials_store = "file"
"#;
    let synthetic_api_auth = r#"{"OPENAI_API_KEY":"synthetic-import-api-key"}"#;
    let synthetic_subscription_auth = r#"{"auth_mode":"chatgpt","tokens":{"access_token":"synthetic-access-token","refresh_token":"synthetic-refresh-token","id_token":"synthetic-id-token","account_id":"synthetic-account"}}"#;
    for (path, contents) in [
        (&api_config, synthetic_api_config),
        (&api_auth, synthetic_api_auth),
        (&subscription_config, synthetic_subscription_config),
        (&subscription_auth, synthetic_subscription_auth),
    ] {
        if let Err(error) = fs::write(path, contents) {
            panic!("could not write synthetic import source: {error}");
        }
    }

    let browsed_home = temp.path().join("alternate-codex-home");
    if let Err(error) = demo::create(&browsed_home) {
        panic!("could not create alternate synthetic Codex data: {error}");
    }
    let browsed_config = temp.path().join("browsed-subscription.toml");
    if let Err(error) = fs::write(
        &browsed_config,
        "model_provider = \"openai\"\nmodel = \"ui-browsed-subscription-model\"\nmodel_reasoning_effort = \"medium\"\n",
    ) {
        panic!("could not write alternate synthetic config: {error}");
    }
    let browsed_paths = match Paths::resolve(Some(browsed_home.clone())) {
        Ok(paths) => paths,
        Err(error) => panic!("could not resolve alternate Codex directory: {error}"),
    };
    let browsed_store = Store::isolated(browsed_paths);
    if let Err(error) = browsed_store.initialize(switcher_core::ImportSources {
        subscription_config: Some(browsed_config),
        ..Default::default()
    }) {
        panic!("could not initialize alternate synthetic Codex data: {error}");
    }

    let (dialogs, file_calls, folder_calls) = test_dialogs(
        vec![
            (false, Some(api_config)),
            (true, Some(api_auth)),
            (false, Some(subscription_config)),
            (true, Some(subscription_auth)),
            (true, None),
        ],
        vec![browsed_home.clone(), home.clone()],
    );
    let paths = match Paths::resolve(Some(home.clone())) {
        Ok(paths) => paths,
        Err(error) => panic!("could not resolve synthetic Codex directory: {error}"),
    };
    let store = Store::isolated(paths);
    cx.update(gpui_kit::init);
    let (window_handle, content) = cx.update(|app| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Default::default(),
                size: size(px(1050.0), px(830.0)),
            })),
            ..Default::default()
        };
        match gpui_kit::open_window(options, app, |window, app| {
            let store = store.clone();
            let dialogs = dialogs.clone();
            app.new(|cx| Switcher::with_dialogs(store, true, dialogs, window, cx))
        }) {
            Ok(result) => result,
            Err(error) => panic!("could not open headless GPUI window: {error}"),
        }
    });
    cx.run_until_parked();

    click(cx, window_handle, "initialize");
    assert!(profile(&store, Mode::Api).ready);
    assert!(profile(&store, Mode::Subscription).ready);

    click(cx, window_handle, "api");
    assert_eq!(content.read_with(cx, |view, _| view.target), Mode::Api);
    click(cx, window_handle, "import-config");
    let imported_api = profile(&store, Mode::Api);
    assert_eq!(imported_api.provider, "CustomApi");
    assert_eq!(imported_api.model, "ui-imported-api-model");
    assert_eq!(
        imported_api.base_url,
        "https://ui-import.example.invalid/v1"
    );
    click(cx, window_handle, "import-auth");
    assert!(profile(&store, Mode::Api).ready);
    click(cx, window_handle, "browse");
    assert_eq!(folder_calls.load(Ordering::SeqCst), 1);
    let browsed_state = content.read_with(cx, |view, cx| {
        (
            view.directory.read(cx).value().to_string(),
            view.status.as_ref().and_then(|status| {
                status
                    .modes
                    .iter()
                    .find(|profile| profile.mode == Mode::Subscription)
                    .map(|profile| profile.model.clone())
            }),
        )
    });
    assert_eq!(browsed_state.0, browsed_home.to_string_lossy());
    assert_eq!(
        browsed_state.1.as_deref(),
        Some("ui-browsed-subscription-model")
    );
    click(cx, window_handle, "browse");
    assert_eq!(folder_calls.load(Ordering::SeqCst), 2);
    let restored_home = content.read_with(cx, |view, cx| {
        (
            view.directory.read(cx).value().to_string(),
            view.status.as_ref().and_then(|status| {
                status
                    .modes
                    .iter()
                    .find(|profile| profile.mode == Mode::Api)
                    .map(|profile| profile.model.clone())
            }),
        )
    });
    assert_eq!(restored_home.0, home.to_string_lossy());
    assert_eq!(restored_home.1.as_deref(), Some("ui-imported-api-model"));
    click(cx, window_handle, "refresh");
    assert!(profile(&store, Mode::Api).ready);

    let key = content.read_with(cx, |view, _| view.key.clone());
    let update_key = cx.update_window(window_handle, |_, window, cx| {
        key.update(cx, |state, cx| {
            state.set_value("synthetic-ui-key", window, cx)
        });
    });
    assert!(
        update_key.is_ok(),
        "API key input could not be updated in the headless window"
    );
    click(cx, window_handle, "preview");
    let preview = content.read_with(cx, |view, _| {
        view.prepared
            .as_ref()
            .map(|prepared| prepared.preview.clone())
    });
    let preview = match preview {
        Some(preview) => preview,
        None => panic!("preview button did not prepare a switch"),
    };
    assert_eq!(preview.target, Mode::Api);
    assert!(preview.auth_changes);
    assert!(
        preview
            .changes
            .iter()
            .any(|change| { change.field == "model_provider" && change.after == "CustomApi" })
    );
    click(cx, window_handle, "apply");
    let api_status = match store.status() {
        Ok(status) => status,
        Err(error) => panic!("API apply status could not be read: {error}"),
    };
    assert_eq!(api_status.mode, Some(Mode::Api));
    assert_eq!(api_status.auth_mode, Some(Mode::Api));
    assert!(content.read_with(cx, |view, cx| view.key.read(cx).value().is_empty()));

    click(cx, window_handle, "subscription");
    assert_eq!(
        content.read_with(cx, |view, _| view.target),
        Mode::Subscription
    );
    click(cx, window_handle, "import-config");
    assert_eq!(
        profile(&store, Mode::Subscription).model,
        "ui-imported-subscription-model"
    );
    click(cx, window_handle, "import-auth");
    assert!(profile(&store, Mode::Subscription).ready);
    assert_eq!(file_calls.load(Ordering::SeqCst), 4);
    click(cx, window_handle, "preview");
    let subscription_preview = content.read_with(cx, |view, _| {
        view.prepared
            .as_ref()
            .map(|prepared| prepared.preview.clone())
    });
    let subscription_preview = match subscription_preview {
        Some(preview) => preview,
        None => panic!("subscription preview button did not prepare a switch"),
    };
    assert_eq!(subscription_preview.target, Mode::Subscription);
    assert!(subscription_preview.auth_changes);
    assert!(subscription_preview.changes.iter().any(|change| {
        change.field == "model" && change.after == "ui-imported-subscription-model"
    }));
    click(cx, window_handle, "apply");
    let subscription_status = match store.status() {
        Ok(status) => status,
        Err(error) => panic!("subscription apply status could not be read: {error}"),
    };
    assert_eq!(subscription_status.mode, Some(Mode::Subscription));
    assert_eq!(subscription_status.auth_mode, Some(Mode::Subscription));
    assert!(subscription_status.initialized);
    assert_eq!(
        profile(&store, Mode::Subscription).model,
        "ui-imported-subscription-model"
    );
    let subscription_auth = match fs::read_to_string(home.join("auth.json")) {
        Ok(auth) => auth,
        Err(error) => panic!("subscription auth was not written: {error}"),
    };
    assert!(subscription_auth.contains("synthetic-access-token"));

    click(cx, window_handle, "api");
    click(cx, window_handle, "preview");
    click(cx, window_handle, "apply");
    let api_status = match store.status() {
        Ok(status) => status,
        Err(error) => panic!("API restore status could not be read: {error}"),
    };
    assert_eq!(api_status.mode, Some(Mode::Api));
    assert_eq!(api_status.auth_mode, Some(Mode::Api));
    let api_config = match fs::read_to_string(home.join("config.toml")) {
        Ok(config) => config,
        Err(error) => panic!("API config was not restored: {error}"),
    };
    assert!(api_config.contains("ui-imported-api-model"));
    let api_auth = match fs::read_to_string(home.join("auth.json")) {
        Ok(auth) => auth,
        Err(error) => panic!("API auth was not restored: {error}"),
    };
    assert!(api_auth.contains("synthetic-ui-key"));

    let before_cancel = [
        match fs::read(store.paths.config()) {
            Ok(contents) => contents,
            Err(error) => panic!("config could not be read before picker cancellation: {error}"),
        },
        match fs::read(store.paths.auth()) {
            Ok(contents) => contents,
            Err(error) => panic!("auth could not be read before picker cancellation: {error}"),
        },
        match fs::read(store.paths.vault()) {
            Ok(contents) => contents,
            Err(error) => {
                panic!("mode state could not be read before picker cancellation: {error}")
            }
        },
        match fs::read(store.paths.key()) {
            Ok(contents) => contents,
            Err(error) => panic!("vault key could not be read before picker cancellation: {error}"),
        },
    ];
    click(cx, window_handle, "import-auth");
    assert_eq!(file_calls.load(Ordering::SeqCst), 5);
    assert!(!content.read_with(cx, |view, _| view.busy));
    assert!(content.read_with(cx, |view, _| view.prepared.is_none()));
    let after_cancel = [
        fs::read(store.paths.config()),
        fs::read(store.paths.auth()),
        fs::read(store.paths.vault()),
        fs::read(store.paths.key()),
    ];
    for (index, result) in after_cancel.into_iter().enumerate() {
        let contents = match result {
            Ok(contents) => contents,
            Err(error) => {
                panic!("local file {index} could not be read after cancellation: {error}")
            }
        };
        assert_eq!(
            contents, before_cancel[index],
            "local file {index} changed after cancel"
        );
    }
    click(cx, window_handle, "refresh");
    let refreshed = match store.status() {
        Ok(status) => status,
        Err(error) => panic!("status could not be read after picker cancellation: {error}"),
    };
    assert_eq!(refreshed.mode, Some(Mode::Api));
    assert!(profile(&store, Mode::Api).ready);
}

#[test]
fn encrypted_state_errors_are_localized_without_dropping_recovery_guidance() {
    assert_eq!(
        super::user_error(&switcher_core::Error(
            "Encryption key is missing. Restore vault.key; existing ciphertext was not changed."
        )),
        "本地加密密钥缺失。请恢复 vault.key；工具未修改现有密文。"
    );
    assert_eq!(
        super::user_error(&switcher_core::Error(
            "Cannot authenticate encrypted state. Check the key and ciphertext."
        )),
        "加密存档验证失败。请检查 vault.key 与密文；工具未修改现有存档。"
    );
    assert_eq!(
        super::user_error(&switcher_core::Error("Invalid API base URL.")),
        "Base URL 格式无效，请检查协议和主机名。"
    );
}
