use std::path::PathBuf;

use color_eyre::Result;
use serde::Deserialize;
use structopt::StructOpt;
use tracing::{debug, error};

#[derive(Debug, StructOpt)]
#[structopt(name = "emily-cli", about = "Chess assistant application")]
struct Opt {
    /// Config file
    #[structopt(short, long, default_value = "config.toml")]
    config: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Config;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let opt = Opt::from_args();
    debug!(?opt, "Emily CLI started");

    let config = match std::fs::read_to_string(&opt.config) {
        Err(err) => {
            error!(?err, path = ?opt.config, "Error while reading config");
            String::new()
        }
        Ok(config) => config,
    };

    debug!(?config, "Emily config loaded");

    Ok(())
}
