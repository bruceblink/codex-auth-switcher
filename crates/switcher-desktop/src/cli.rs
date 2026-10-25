use clap::{Parser, ValueEnum};
use std::{
    io::{self, Read},
    path::PathBuf,
};
use switcher_core::{ImportSources, Mode, Paths, Store, SwitchOptions};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Default, ValueEnum)]
enum Command {
    #[default]
    Gui,
    Status,
    Initialize,
    Preview,
    Switch,
}
#[derive(Clone, Copy, ValueEnum)]
enum Target {
    Subscription,
    Api,
}
impl From<Target> for Mode {
    fn from(value: Target) -> Self {
        match value {
            Target::Subscription => Self::Subscription,
            Target::Api => Self::Api,
        }
    }
}

#[derive(Parser)]
#[command(
    version,
    about = "Local Codex subscription/API authentication switcher"
)]
struct Args {
    #[arg(long, value_enum, default_value = "gui")]
    command: Command,
    #[arg(long, value_enum)]
    target: Option<Target>,
    #[arg(long)]
    codex_home: Option<PathBuf>,
    /// Create synthetic data in a new directory and ignore host environment.
    #[arg(long)]
    demo: bool,
    /// Ignore host environment credentials, including for an existing demo directory.
    #[arg(long)]
    isolated: bool,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    api_base_url: Option<String>,
    /// Read a key from standard input; never put credentials in arguments.
    #[arg(long)]
    api_key_stdin: bool,
    #[arg(long)]
    subscription_config: Option<PathBuf>,
    #[arg(long)]
    subscription_auth: Option<PathBuf>,
    #[arg(long)]
    api_config: Option<PathBuf>,
    #[arg(long)]
    api_auth: Option<PathBuf>,
}

pub fn run() -> anyhow::Result<()> {
    let args = Args::parse();
    let temporary = if args.demo && args.codex_home.is_none() {
        Some(tempfile::tempdir()?)
    } else {
        None
    };
    let demo_home = temporary.as_ref().map(|d| d.path().join("codex-demo"));
    let paths = Paths::resolve(args.codex_home.or(demo_home))?;
    if args.demo {
        crate::demo::create(&paths.home)?;
    }
    let store = if args.demo || args.isolated {
        Store::isolated(paths)
    } else {
        Store::new(paths)
    };
    if matches!(args.command, Command::Gui) {
        if args.api_key_stdin {
            anyhow::bail!("--api-key-stdin requires preview or switch.");
        }
        crate::app::run(store, args.demo || args.isolated)?;
        return Ok(());
    }
    let result = match args.command {
        Command::Status => serde_json::to_value(store.status()?)?,
        Command::Initialize => serde_json::to_value(store.initialize(ImportSources {
            subscription_config: args.subscription_config,
            subscription_auth: args.subscription_auth,
            api_config: args.api_config,
            api_auth: args.api_auth,
        })?)?,
        Command::Preview | Command::Switch => {
            let target = args
                .target
                .ok_or_else(|| anyhow::anyhow!("--target is required."))?;
            let api_key = if args.api_key_stdin {
                let mut value = Zeroizing::new(String::new());
                io::stdin()
                    .take(16386)
                    .read_to_string(&mut value)
                    .map_err(|_| anyhow::anyhow!("Cannot read API key from stdin."))?;
                if value.len() > 16385 {
                    anyhow::bail!("API key input exceeds size limit.");
                }
                Some(Zeroizing::new(
                    value.trim_end_matches(['\r', '\n']).to_string(),
                ))
            } else {
                None
            };
            let prepared = store.prepare(
                target.into(),
                SwitchOptions {
                    provider: args.provider,
                    base_url: args.api_base_url,
                    api_key,
                },
            )?;
            if matches!(args.command, Command::Preview) {
                serde_json::to_value(&prepared.preview)?
            } else {
                serde_json::to_value(store.apply(prepared)?)?
            }
        }
        Command::Gui => unreachable!(),
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
