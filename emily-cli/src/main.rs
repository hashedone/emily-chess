use std::path::{Path, PathBuf};

use color_eyre::Result;
use structopt::StructOpt;
use tracing::{debug, error, warn};

use config::Config;

mod config;
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

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    let opt = Opt::from_args();
    debug!(?opt, "Emily CLI started");

    let config = read_config(&opt.config).await;
    debug!(?config, "Emily config loaded");

    opt.cmd.run(config).await
}
