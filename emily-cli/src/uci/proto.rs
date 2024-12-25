use std::fmt::Display;
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, ChildStdout};

use color_eyre::eyre::Context;
use color_eyre::Result;
use tracing::debug;

pub struct Protocol {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    buf: String,
    name: String,
}

impl Protocol {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            stdin,
            stdout: BufReader::new(stdout),
            buf: String::new(),
            name: String::new(),
        }
    }

    fn send(&mut self, command: Command) -> Result<()> {
        let command = command.to_string();
        debug!(engine = self.name, "UCI send: {}", command);

        self.stdin
            .write_all(command.as_bytes())
            .wrap_err("While writting to engine")?;

        self.stdin
            .write_all(b"\n")
            .wrap_err("While writting to engine")
    }

    fn recv(&mut self) -> Result<Msg> {
        loop {
            self.stdout
                .read_line(&mut self.buf)
                .wrap_err("While reading engine")?;

            let line = self.buf.trim();
            if !line.is_empty() {
                debug!(engine = self.name, "UCI recv: {}", line);
            }

            let msg = Msg::parse(line);
            self.buf.clear();

            if let Some(msg) = msg {
                return Ok(msg);
            }
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn init(&mut self) -> Result<Headers> {
        self.send(Command::Uci)?;

        loop {
            use Msg::*;

            match self.recv()? {
                Id { name: Some(n), .. } => self.name = n,
                UciOk => break,
                _ => (),
            }
        }

        Ok(Headers)
    }

    pub fn debug(&mut self, debug: bool) -> Result<()> {
        self.send(Command::Debug(debug))
    }

    pub fn set_option(&mut self, option: String, value: String) -> Result<()> {
        self.send(Command::SetOption(option, value))
    }
}

#[derive(Debug)]
pub struct Headers;

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
