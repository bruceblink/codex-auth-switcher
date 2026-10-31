#![cfg(windows)]

use switcher_core::{ImportSources, Mode, Paths, Store, SwitchOptions};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};
use zeroize::Zeroizing;

#[test]
fn isolated_store_does_not_write_the_user_environment() -> anyhow::Result<()> {
    let environment = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Environment")?;
    let before = environment.get_raw_value("AUTOCODE_API_KEY").ok();
    let temp = tempfile::tempdir()?;
    let store = Store::isolated(Paths {
        home: temp.path().join("codex-home"),
    });
    store.initialize(ImportSources::default())?;
    let prepared = store.prepare(
        Mode::Api,
        SwitchOptions {
            api_key: Some(Zeroizing::new("synthetic-isolation-test".into())),
            ..Default::default()
        },
    )?;
    let result = store.apply(prepared)?;
    assert_eq!(result.environment_persisted, None);
    let after = environment.get_raw_value("AUTOCODE_API_KEY").ok();
    assert!(
        before == after,
        "isolated store changed the user environment"
    );
    Ok(())
}
