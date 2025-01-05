use std::path::{Path, PathBuf};

use color_eyre::Result;
use structopt::StructOpt;
use tracing::{debug, error, warn};

use config::Config;

mod adapters;
mod config;
mod knowledge;
mod rev;
mod uci;

#[derive(Debug, StructOpt)]
#[structopt(name = "emily-cli", about = "Chess assistant application")]
struct Opt {
    /// Config file
    #[structopt(short, long, default_value = "config.toml")]
    config: PathBuf,

    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(Debug, StructOpt)]
enum Command {
    // Position analysis and review
    Rev(rev::Rev),
}

impl Command {
    async fn run(self, config: Config) -> Result<()> {
        use Command::*;

        match self {
            Rev(rev) => rev.run(config).await,
        }
    }
}

async fn read_config(path: &Path) -> Config {
    let config = match tokio::fs::read_to_string(path).await {
        Err(err) => {
            warn!(?err, ?path, "Error while reading config, using defaults");
            return Config::default();
        }
        Ok(config) => config,
    };

    match toml::from_str(&config) {
        Err(err) => {
            error!(?err, ?path, "Error parsing config, using defaults");
            Config::default()
        }
        Ok(config) => config,
    }
}

fn setup_tracing() {
    use tracing_error::ErrorLayer;
    use tracing_subscriber::filter::EnvFilter;
    use tracing_subscriber::fmt;
    use tracing_subscriber::prelude::*;

    let fmt_layer = fmt::layer().with_target(false).pretty();
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .with(ErrorLayer::default())
        .init();
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    setup_tracing();
    color_eyre::install()?;

    let opt = Opt::from_args();
    debug!(?opt, "Emily CLI started");

    let config = read_config(&opt.config).await;
    debug!(?config, "Emily config loaded");

    opt.cmd.run(config).await
}
