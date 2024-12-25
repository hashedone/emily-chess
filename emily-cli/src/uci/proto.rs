use std::fmt::Display;

use color_eyre::eyre::{Context, OptionExt};
use color_eyre::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout};
use tracing::debug;

pub struct Protocol {
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    name: String,
}

impl Protocol {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            stdin,
            stdout: BufReader::new(stdout).lines(),
            name: String::new(),
        }
    }

    async fn send(&mut self, command: Command) -> Result<()> {
        let mut command = command.to_string();
        command.push('\n');

        self.stdin
            .write_all(command.as_bytes())
            .await
            .wrap_err("While writting to engine")?;

        debug!(engine = self.name, "UCI send: {}", command.trim());
        Ok(())
    }

    async fn recv(&mut self) -> Result<Msg> {
        loop {
            let line = self
                .stdout
                .next_line()
                .await
                .wrap_err("While reading engine")?
                .ok_or_eyre("Engine stdout closed")?;

            let line = line.trim();
            if !line.is_empty() {
                debug!(engine = self.name, "UCI recv: {}", line);
                if let Some(msg) = Msg::parse(line) {
                    return Ok(msg);
                }
            }
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn init(&mut self) -> Result<()> {
        self.send(Command::Uci).await?;

        loop {
            use Msg::*;

            match self.recv().await? {
                Id { name: Some(n), .. } => self.name = n,
                UciOk => break,
                _ => (),
            }
        }

        Ok(())
    }

    pub async fn debug(&mut self, debug: bool) -> Result<()> {
        self.send(Command::Debug(debug)).await
    }

    pub async fn set_option(&mut self, option: String, value: String) -> Result<()> {
        self.send(Command::SetOption(option, value)).await
    }
}

#[derive(Debug)]
enum Command {
    Uci,
    Debug(bool),
    SetOption(String, String),
}

impl Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Command::*;

        match self {
            Uci => write!(f, "uci"),
            Debug(true) => write!(f, "debug on"),
            Debug(false) => write!(f, "debug off"),
            SetOption(name, value) => write!(f, "setoption name {name} value {value}"),
        }
    }
}

#[derive(Debug)]
enum Msg {
    Id { name: Option<String> },
    UciOk,
}

impl Msg {
    fn parse_id(args: &str) -> Self {
        let name = args.split_once(" name ").map(|(_, name)| name.to_owned());
        Self::Id { name }
    }

    fn parse(line: &str) -> Option<Self> {
        let idx = line.find(' ').unwrap_or(line.len());
        let cmd = line[..idx].trim();
        let args = &line[idx..];

        let msg = match cmd {
            "id" => Self::parse_id(args),
            "uciok" => Self::UciOk,
            _ => return None,
        };

        Some(msg)
    }
}
