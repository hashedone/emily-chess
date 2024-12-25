//! UCI protocol implementation and engine interface

use std::process::{self, Child, Stdio};

use color_eyre::eyre::{Context, OptionExt};
use color_eyre::Result;
use tracing::info;

use self::proto::Protocol;

mod proto;

pub struct Engine {
    process: Child,
    proto: Protocol,
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.process.kill().ok();
    }
}

impl Engine {
    pub fn run(config: crate::config::Engine) -> Result<Engine> {
        info!(cmd = ?config.command, args = ?config.args, pwd = ?config.pwd, "Starting engine");

        let mut command = process::Command::new(config.command);

        command
            .args(config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        if let Some(pwd) = config.pwd {
            command.current_dir(pwd);
        }

        let mut process = command.spawn().wrap_err("While starting engine")?;

        let io = || -> Result<_> {
            let stdin = process
                .stdin
                .take()
                .ok_or_eyre("Cannot open engine stdin")?;
            let stdout = process
                .stdout
                .take()
                .ok_or_eyre("Cannot open engine stdout")?;

            Ok((stdin, stdout))
        }();

        let mut engine = match io {
            Ok((stdin, stdout)) => Engine {
                process,
                proto: Protocol::new(stdin, stdout),
            },
            Err(err) => {
                process.kill().ok();
                return Err(err);
            }
        };

        info!(pid = engine.process.id(), "Engine started");
        engine.proto.init()?;

        Ok(engine)
    }
}
