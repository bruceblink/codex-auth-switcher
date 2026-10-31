use super::{DialogFuture, FileDialogs, Switcher};
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    point, px, size,
    test::{TestAppContextExt, TestWindowExt},
};
use std::{fs, path::PathBuf, sync::Arc, time::Duration};
use switcher_core::{Mode, Paths, Store};

fn open(
    cx: &mut TestAppContext,
    home: PathBuf,
    width: f32,
    height: f32,
) -> (AnyWindowHandle, Entity<Switcher>) {
    let paths = Paths::resolve(Some(home))
        .unwrap_or_else(|error| panic!("cannot resolve test directory: {error}"));
    let dialogs = FileDialogs {
        pick_file: Arc::new(|_| Box::pin(async { None }) as DialogFuture),
        pick_folder: Arc::new(|| Box::pin(async { None }) as DialogFuture),
    };
    cx.update(gpui_kit::init);
    cx.update(|app| {
        gpui_kit::component::Theme::set_scrollbar_mode(
            gpui_kit::component::scroll::ScrollbarMode::Always,
            app,
        );
    });
    let (handle, content) = cx.update(|app| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(width), px(height)),
                })),
                ..Default::default()
            },
            app,
            |window, app| {
                app.new(|cx| Switcher::with_dialogs(Store::isolated(paths), dialogs, window, cx))
            },
        )
        .unwrap_or_else(|error| panic!("cannot open GPUI test window: {error}"))
    });
    (handle, content)
}

async fn idle(cx: &mut TestAppContext, window: AnyWindowHandle, app: &Entity<Switcher>) {
    cx.wait_for(window, Duration::from_secs(3), |_, cx| !app.read(cx).busy)
        .await;
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap_or_else(|error| panic!("cannot click {id}: {error}"));
}

#[gpui_kit::test]
async fn switches_modes_and_clears_the_api_input(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let (window, app) = open(cx, home.clone(), 1200., 1100.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;

    let old_key = app.read_with(cx, |view, _| view.key.entity_id());
    cx.update_window(window, |_, window, cx| {
        let key = app.read(cx).key.clone();
        key.update(cx, |state, cx| {
            state.set_value("synthetic-gpui-input", window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot edit test input: {error}"));
    click(cx, window, "preview");
    idle(cx, window, &app).await;
    assert!(app.read_with(cx, |view, _| {
        view.prepared
            .as_ref()
            .is_some_and(|item| item.preview.changed)
    }));
    click(cx, window, "apply");
    idle(cx, window, &app).await;
    assert_eq!(
        app.read_with(cx, |view, _| view
            .status
            .as_ref()
            .and_then(|status| status.mode)),
        Some(Mode::Api)
    );
    assert!(app.read_with(cx, |view, cx| view.key.read(cx).value().is_empty()));
    assert_ne!(old_key, app.read_with(cx, |view, _| view.key.entity_id()));

    click(cx, window, "subscription");
    click(cx, window, "preview");
    idle(cx, window, &app).await;
    click(cx, window, "apply");
    idle(cx, window, &app).await;
    assert_eq!(
        app.read_with(cx, |view, _| view
            .status
            .as_ref()
            .and_then(|status| status.mode)),
        Some(Mode::Subscription)
    );
    let config = fs::read_to_string(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    assert!(config.contains("[projects.\"F:/example\"]"));
    assert!(config.contains("[features]"));
}

#[gpui_kit::test]
async fn first_run_creates_defaults_without_fake_authentication(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    let (window, app) = open(cx, home.clone(), 1040., 740.);
    idle(cx, window, &app).await;
    assert!(!home.join("config.toml").exists());
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    let config = fs::read_to_string(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    assert!(config.contains("cli_auth_credentials_store"));
    assert!(!config.contains("gateway"));
    assert!(!home.join("auth.json").exists());
    assert_eq!(
        app.read_with(cx, |view, cx| view.endpoint.read(cx).value().to_string()),
        "https://api.openai.com/v1"
    );
}

#[gpui_kit::test]
async fn changing_directory_clears_unsaved_key_but_refreshing_same_directory_keeps_it(
    cx: &mut TestAppContext,
) {
    let temporary = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let first_home = temporary.path().join("first-codex-home");
    let second_home = temporary.path().join("second-codex-home");
    let (window, app) = open(cx, first_home, 1040., 740.);
    idle(cx, window, &app).await;

    let initial_key = "synthetic-key-for-first-directory";
    cx.update_window(window, |_, window, cx| {
        let key = app.read(cx).key.clone();
        key.update(cx, |state, cx| {
            state.set_value(initial_key, window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot set test API key: {error}"));
    let original_key_entity = app.read_with(cx, |view, _| view.key.entity_id());

    click(cx, window, "refresh");
    idle(cx, window, &app).await;
    assert_eq!(
        app.read_with(cx, |view, cx| view.key.read(cx).value().to_string()),
        initial_key,
        "refreshing the same directory must retain the unsaved key"
    );
    assert_eq!(
        app.read_with(cx, |view, _| view.key.entity_id()),
        original_key_entity,
        "refreshing the same directory must retain the input history"
    );

    cx.update_window(window, |_, window, cx| {
        let directory = app.read(cx).directory.clone();
        directory.update(cx, |state, cx| {
            state.set_value(second_home.to_string_lossy(), window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot set test Codex directory: {error}"));
    click(cx, window, "refresh");
    idle(cx, window, &app).await;

    assert!(app.read_with(cx, |view, cx| view.key.read(cx).value().is_empty()));
    assert_ne!(
        app.read_with(cx, |view, _| view.key.entity_id()),
        original_key_entity,
        "changing directories must replace the key input and revoke its edit history"
    );
}

#[gpui_kit::test]
async fn changing_directory_resets_provider_and_endpoint_without_profiles(cx: &mut TestAppContext) {
    let temporary = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let first_home = temporary.path().join("first-codex-home");
    let second_home = temporary.path().join("second-codex-home");
    crate::demo::create(&first_home).unwrap_or_else(|error| panic!("test data: {error}"));
    let (window, app) = open(cx, first_home, 1040., 740.);
    idle(cx, window, &app).await;

    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .api_provider
            .read(cx)
            .value()
            .to_string()),
        "OpenAI"
    );
    assert_eq!(
        app.read_with(cx, |view, cx| view.endpoint.read(cx).value().to_string()),
        "https://gateway.example.invalid/v1"
    );

    cx.update_window(window, |_, window, cx| {
        let directory = app.read(cx).directory.clone();
        directory.update(cx, |state, cx| {
            state.set_value(second_home.to_string_lossy(), window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot set test Codex directory: {error}"));
    click(cx, window, "refresh");
    idle(cx, window, &app).await;

    assert!(app.read_with(cx, |view, _| {
        view.status
            .as_ref()
            .is_some_and(|status| status.modes.is_empty())
    }));
    assert_eq!(
        app.read_with(cx, |view, cx| view
            .api_provider
            .read(cx)
            .value()
            .to_string()),
        "openai"
    );
    assert_eq!(
        app.read_with(cx, |view, cx| view.endpoint.read(cx).value().to_string()),
        "https://api.openai.com/v1"
    );
}

#[gpui_kit::test]
async fn controls_stay_visible_and_picker_cancellation_is_safe(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let (window, app) = open(cx, home.clone(), 720., 560.);
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        for id in ["preview", "apply", "restart"] {
            let button = window.find(id);
            assert!(button.visible(), "{id} is outside the window");
            assert!(button.bounds().bottom() <= window.viewport_size().height);
        }
    })
    .unwrap_or_else(|error| panic!("cannot inspect test window: {error}"));
    let before = fs::read(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    click(cx, window, "browse");
    idle(cx, window, &app).await;
    let after = fs::read(home.join("config.toml"))
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    assert_eq!(before, after);
    assert!(app.read_with(cx, |view, _| view.message.contains("取消")));
}

#[gpui_kit::test]
async fn scrollbars_reach_overflowing_switch_and_model_content(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("temp directory: {error}"));
    let home = temp.path().join("codex-home");
    crate::demo::create(&home).unwrap_or_else(|error| panic!("test data: {error}"));
    let catalog_path = home.join("models-codex.json");
    let models = (0..48)
        .map(|index| {
            format!(
                r#"{{"slug":"model-{index}","display_name":"Model {index}","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{{"effort":"high","description":"Synthetic high"}}],"priority":{index}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    fs::write(
        &catalog_path,
        format!(r#"{{"catalog_version":3,"models":[{models}]}}"#),
    )
    .unwrap_or_else(|error| panic!("cannot write test model catalog: {error}"));
    let catalog_path = catalog_path.to_string_lossy().replace('\\', "/");
    let config_path = home.join("config.toml");
    let config = fs::read_to_string(&config_path)
        .unwrap_or_else(|error| panic!("cannot read test config: {error}"));
    fs::write(
        &config_path,
        format!("model_catalog_json = \"{catalog_path}\"\n{config}"),
    )
    .unwrap_or_else(|error| panic!("cannot configure test model catalog: {error}"));

    let (window, app) = open(cx, home, 720., 560.);
    idle(cx, window, &app).await;
    click(cx, window, "initialize");
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        let key = app.read(cx).key.clone();
        key.update(cx, |state, cx| {
            state.set_value("synthetic-scroll-test-key", window, cx);
        });
    })
    .unwrap_or_else(|error| panic!("cannot seed preview input: {error}"));
    click(cx, window, "preview");
    idle(cx, window, &app).await;

    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let scroll = app.read(cx).switch_scroll.clone();
        assert!(
            scroll.max_offset().y > px(0.),
            "switch page content does not overflow"
        );
        let bounds = scroll.bounds();
        let start = point(bounds.right() - px(8.), bounds.top() + px(20.));
        let end = point(bounds.right() - px(8.), bounds.bottom() - px(20.));
        let offset_before = scroll.offset().y;
        window.drag(start, end, cx);
        assert!(
            scroll.offset().y < offset_before,
            "dragging the switch page scrollbar did not move its content"
        );
        for id in ["preview", "apply", "restart"] {
            let button = window.find(id);
            assert!(button.visible(), "{id} is outside the window");
            assert!(button.bounds().bottom() <= window.viewport_size().height);
        }
    })
    .unwrap_or_else(|error| panic!("cannot inspect switch page scrolling: {error}"));

    click(cx, window, "models-tab");
    click(cx, window, "catalog-load");
    idle(cx, window, &app).await;
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let model_list_scroll = app.read(cx).model_list_scroll.clone();
        assert!(
            model_list_scroll.max_offset().y > px(0.),
            "model list content does not overflow"
        );
        let list_bounds = model_list_scroll.bounds();
        let start = point(list_bounds.right() - px(8.), list_bounds.top() + px(20.));
        let end = point(list_bounds.right() - px(8.), list_bounds.bottom() - px(20.));
        let list_offset_before = model_list_scroll.offset().y;
        window.drag(start, end, cx);
        assert!(
            model_list_scroll.offset().y < list_offset_before,
            "dragging the model list scrollbar did not move the list"
        );
        let catalog_scroll = app.read(cx).catalog_scroll.clone();
        assert!(
            catalog_scroll.max_offset().y > px(0.),
            "model catalog page content does not overflow"
        );
        let catalog_bounds = catalog_scroll.bounds();
        let start = point(
            catalog_bounds.right() - px(8.),
            catalog_bounds.top() + px(20.),
        );
        let end = point(
            catalog_bounds.right() - px(8.),
            catalog_bounds.bottom() - px(20.),
        );
        let catalog_offset_before = catalog_scroll.offset().y;
        window.drag(start, end, cx);
        assert!(
            catalog_scroll.offset().y < catalog_offset_before,
            "dragging the model catalog page scrollbar did not move its content"
        );
    })
    .unwrap_or_else(|error| panic!("cannot inspect model list scrolling: {error}"));
}
