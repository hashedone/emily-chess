//! UCI protocol implementation and engine interface

use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process;

use color_eyre::eyre::{Context, OptionExt};
use color_eyre::Result;
use tokio::spawn;
use tracing::{error, info, warn};

use self::proto::Protocol;

mod proto;

pub struct Engine {
    task: tokio::task::JoinHandle<()>,
    proto: Protocol,
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Engine {
    async fn configure(&mut self, config: crate::config::Engine) -> Result<()> {
        self.proto.debug(config.debug).await?;

        for (option, value) in config.options {
            if let Err(err) = self.proto.set_option(option, value).await {
                warn!(
                    engine = self.proto.name(),
                    "While setting engine option: {err}"
                );
            }
        }

        info!(engine = self.proto.name(), "Engine configured");
        Ok(())
    }

    pub async fn run(config: crate::config::Engine) -> Result<Engine> {
        info!(cmd = ?config.command, args = ?config.args, pwd = ?config.pwd, "Starting engine");

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
                                warn!("Engine: {line}")
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
                Ok(code) => info!("Engine exited with code {code}"),
                Err(err) => error!("While running engine: {err}"),
            }
        });

        let mut engine = Self { task, proto };

        engine.proto.init().await?;
        info!(engine = engine.proto.name(), "Engine initialized");

        engine.configure(config).await?;

        Ok(engine)
    }
}
