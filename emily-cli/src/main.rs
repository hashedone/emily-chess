use std::path::{Path, PathBuf};

use color_eyre::Result;
use serde::Deserialize;
use structopt::StructOpt;
use tracing::{debug, error, warn};

mod rev;

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
    fn run(self, config: Config) -> Result<()> {
        use Command::*;

        match self {
            Rev(rev) => rev.run(config),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct Config;

fn read_config(path: &Path) -> Config {
    let config = match std::fs::read_to_string(path) {
        Err(err) => {
            warn!(?err, ?path, "Error while reading config, using defaults");
            return Config;
        }
        Ok(config) => config,
    };

    match toml::from_str(&config) {
        Err(err) => {
            error!(?err, ?path, "Error parsing config, using defaults");
            Config
        }
        Ok(config) => config,
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let opt = Opt::from_args();
    debug!(?opt, "Emily CLI started");

    let config = read_config(&opt.config);
    debug!(?config, "Emily config loaded");

    opt.cmd.run(config)?;

    Ok(())
}
