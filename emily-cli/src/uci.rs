//! UCI protocol implementation and engine interface

use derivative::Derivative;
use shakmaty::fen::Fen;
use shakmaty::uci::UciMove;
use shakmaty::{Chess, EnPassantMode, Move};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process;

use color_eyre::eyre::{Context, OptionExt};
use color_eyre::Result;
use tokio::spawn;
use tracing::{error, info, instrument, trace, warn};

use self::proto::{InfoStream, Protocol};
use crate::adapters::debug::{DFenExt, FlatOptExt, LineExt};

pub use self::proto::Score;

mod proto;

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Engine {
    #[derivative(Debug = "ignore")]
    task: tokio::task::JoinHandle<()>,
    #[derivative(Debug = "ignore")]
    proto: Protocol,
    name: String,
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Engine {
    #[instrument(skip(config))]
    async fn configure(&mut self, config: crate::config::Engine) {
        if config.debug {
            trace!("Enabling debug engine mode");
            if let Err(err) = self.proto.debug().await {
                warn!(%err, "While setting engine debug mode");
            }
        }

        for (option, value) in config.options {
            if let Err(err) = self.proto.set_option(option, value).await {
                warn!(%err, "While setting engine option");
            }
        }

        trace!("Engine configured");
    }

    #[instrument(skip(config), err)]
    pub async fn run(config: crate::config::Engine) -> Result<Engine> {
        trace!(?config, "Starting engine");

        let mut command = process::Command::new(&config.command);
        command
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(pwd) = &config.pwd {
            command.current_dir(pwd);
        }

        trace!(?command, "Execution command prepared");
        let mut process = command.spawn().wrap_err("While starting engine")?;

        let stdin = process
            .stdin
            .take()
            .ok_or_eyre("Cannot open engine stdin")?;

        let stdout = process
            .stdout
            .take()
            .ok_or_eyre("Cannot open engine stdout")?;

        match process.stderr.take() {
            Some(stderr) => {
                spawn(async move {
                    let mut stderr = BufReader::new(stderr).lines();
                    loop {
                        match stderr.next_line().await {
                            Err(err) => {
                                error!("While reading from engine stderr: {err}");
                                break;
                            }
                            Ok(None) => break,
                            Ok(Some(line)) => {
                                warn!(err = line, "Engine stderr")
                            }
                        }
                    }
                });
            }
            None => warn!("Cannot open engine stderr"),
        }
        info!(pid = process.id(), "Engine started");

        let proto = Protocol::new(stdin, stdout);

        let task = spawn(async move {
            match process.wait().await {
                Ok(code) => info!(%code, "Engine exited"),
                Err(err) => error!(?err, "While running engine"),
            }
        });

        let mut engine = Self {
            task,
            proto,
            name: config.name.clone(),
        };

        engine.proto.init().await?;
        trace!("Engine initialized");

        engine.configure(config).await;
        Ok(engine)
    }

    #[instrument(err)]
    pub async fn new_game(&mut self) -> Result<()> {
        self.proto.new_game().await?;
        self.proto.wait_ready().await
    }

    #[instrument(skip(fen, moves, depth, time), fields(fen=?fen.d_fen(), moves=?moves.d_line(), depth=?depth.d_opt(), time=?time.d_opt()), err)]
    pub async fn go(
        &mut self,
        fen: Chess,
        moves: &[Move],
        depth: Option<u8>,
        time: Option<Duration>,
    ) -> Result<InfoStream> {
        let fen = Fen::from_position(fen, EnPassantMode::Always);
        let moves = moves.iter().map(UciMove::from_standard).collect();
        self.proto.position(Some(fen), moves).await?;
        self.proto.go(depth, time).await
    }

    #[instrument(err)]
    pub async fn quit(mut self) -> Result<()> {
        self.proto.quit().await
    }
}
