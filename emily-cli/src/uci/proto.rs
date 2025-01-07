use std::cmp::Ordering;
use std::fmt::Display;
use std::time::Duration;

use color_eyre::eyre::{bail, Context, OptionExt};
use color_eyre::Result;
use serde::{Deserialize, Serialize};
use shakmaty::fen::Fen;
use shakmaty::uci::UciMove;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout};
use tracing::{debug, warn};

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

        debug!("UCI send: {}", command.trim());
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
                debug!("UCI recv: {}", line);
                if let Some(msg) = Msg::parse(line) {
                    return Ok(msg);
                }
            }
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn debug(&mut self) -> Result<()> {
        self.send(Command::Debug).await
    }

    pub async fn set_option(&mut self, option: String, value: String) -> Result<()> {
        self.send(Command::SetOption(option, value)).await
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

    pub async fn wait_ready(&mut self) -> Result<()> {
        self.send(Command::IsReady).await?;

        while !matches!(self.recv().await?, Msg::ReadyOk) {}

        Ok(())
    }

    pub async fn new_game(&mut self) -> Result<()> {
        self.send(Command::NewGame).await
    }

    /// Sets the position for analysis
    pub async fn position(
        &mut self,
        fen: impl Into<Option<Fen>>,
        moves: impl IntoIterator<Item = UciMove>,
    ) -> Result<()> {
        self.send(Command::Position {
            fen: fen.into(),
            line: moves.into_iter().collect(),
        })
        .await
    }

    /// Starts the game analysis
    pub async fn go(
        &mut self,
        depth: impl Into<Option<u8>>,
        time: impl Into<Option<Duration>>,
    ) -> Result<InfoStream> {
        self.send(Command::Go {
            depth: depth.into(),
            time: time.into(),
        })
        .await?;

        Ok(InfoStream {
            proto: self,
            best: None,
        })
    }

    pub async fn quit(&mut self) -> Result<()> {
        self.send(Command::Quit).await
    }
}

/// Ongoing engine analysis after the `go` command. It allows to retrieve the `info` position
/// information and waiting for a final best move information.
///
/// It keeps mutable reference to the protocol, so it is impossible to perform any
/// additional communication during analysis (but the `stop` command can be send to finish it early
/// if needed).
pub struct InfoStream<'a> {
    proto: &'a mut Protocol,
    /// Best move if the analysis is complete. If it is `Some` no more `info` are expected and
    /// stdout should not be read.
    best: Option<UciMove>,
}

impl InfoStream<'_> {
    /// Waits for the best move ignoring the info command. After this analysis is fully complete so
    /// it consumes `self`.
    #[allow(unused)]
    pub async fn best(self) -> Result<UciMove> {
        // `bestmove` command was already met, returning cached move.
        if let Some(best) = self.best {
            return Ok(best);
        }

        loop {
            if let Msg::BestMove(best) = self.proto.recv().await? {
                return Ok(best);
            }
        }
    }

    /// Stops the analysis as soon as possible even if stop condidions were not yet met. After
    /// calling this the caller should still wait for `info` function returning `None` or call the
    /// `best` method to ensure the whole analysis is consumed. Alternatievely user can synchronize
    /// with the I/O using the `Protocol::wait_ready`.
    #[allow(unused)]
    pub async fn stop(&mut self) -> Result<()> {
        self.proto.send(Command::Stop).await
    }

    /// Stops the analysis as soon as possible and wait for it finishes leaving the communication
    /// with engine in-sync. Ignores remaining `info` messages.
    #[allow(unused)]
    pub async fn stop_wait(mut self) -> Result<UciMove> {
        self.stop().await?;
        self.best().await
    }

    /// Gets the next `info` message, or `None` if the `bestmove` occured finishing the analysis.
    /// After the `Ok(None)` is returned, the `best` method can be called to retrieve the best move not
    /// affecting the engines output.
    pub async fn info(&mut self) -> Result<Option<Info>> {
        loop {
            match self.proto.recv().await? {
                Msg::BestMove(best) => {
                    self.best = Some(best);
                    return Ok(None);
                }
                Msg::Info(info) => return Ok(Some(info)),
                _ => (),
            }
        }
    }
}

/// Command send to the engine
#[derive(Debug)]
enum Command {
    /// Intialize UCI mode
    Uci,
    /// Set debug mode
    Debug,
    /// Set engine option
    SetOption(String, String),
    /// Sync with engine IO
    IsReady,
    /// Start new game
    NewGame,
    /// Setup the position
    Position {
        /// Position FEN - if missing, startpos will be used
        fen: Option<Fen>,
        /// Moves after the intial FEN
        line: Vec<UciMove>,
    },
    /// Start evaluation
    Go {
        /// Limit depth search
        depth: Option<u8>,
        /// Limit search time
        time: Option<Duration>,
    },
    /// Stop engine evaluation as soon as possible
    #[allow(unused)]
    Stop,
    /// Gracefully quit
    Quit,
}

impl Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Command::*;

        match self {
            Uci => write!(f, "uci"),
            Debug => write!(f, "debug"),
            SetOption(name, value) => write!(f, "setoption name {name} value {value}"),
            IsReady => write!(f, "isready"),
            NewGame => write!(f, "ucinewgame"),
            Position { fen, line } => {
                write!(f, "position fen ")?;
                match fen {
                    Some(fen) => write!(f, "{fen}")?,
                    None => write!(f, "startpos")?,
                }

                if !line.is_empty() {
                    write!(f, " moves")?;
                    for m in line {
                        write!(f, " {m}")?;
                    }
                }

                Ok(())
            }
            Go { depth, time } => {
                write!(f, "go")?;

                if let Some(depth) = &depth {
                    write!(f, " depth {depth}")?;
                }

                if let Some(time) = &time {
                    write!(f, " movetime {}", time.as_millis())?;
                }

                if depth.is_none() && time.is_none() {
                    write!(f, " infinite")?;
                }

                Ok(())
            }
            Stop => write!(f, "stop"),
            Quit => write!(f, "quit"),
        }
    }
}

/// Messages received from engine
#[derive(Debug)]
enum Msg {
    /// Information about engine
    Id { name: Option<String> },
    /// Initialization complete
    UciOk,
    /// IO sync
    ReadyOk,
    /// Analysis complete
    BestMove(UciMove),
    /// Analysis step
    Info(Info),
}

impl Msg {
    fn parse_id(args: &str) -> Option<Self> {
        let name = args.split_once(" name ").map(|(_, name)| name.to_owned());
        Some(Self::Id { name })
    }

    fn parse_bestmove(args: &str) -> Option<Self> {
        let args = args.trim();
        let m = match args.split_once(' ') {
            Some((m, _)) => m,
            None => args,
        };
        match m.parse() {
            Ok(m) => Some(Msg::BestMove(m)),
            Err(err) => {
                warn!(mov = m, ?err, "Invalid best move");
                None
            }
        }
    }

    fn parse(line: &str) -> Option<Self> {
        let idx = line.find(' ').unwrap_or(line.len());
        let cmd = line[..idx].trim();
        let args = &line[idx..];

        match cmd {
            "id" => Self::parse_id(args),
            "uciok" => Some(Self::UciOk),
            "readyok" => Some(Self::ReadyOk),
            "bestmove" => Self::parse_bestmove(args),
            "info" => match Info::parse(args) {
                Ok(info) => Some(Self::Info(info?)),
                Err(err) => {
                    warn!(?err, "Invalid info format");
                    None
                }
            },
            _ => None,
        }
    }
}

/// Engine analysis info
#[derive(Debug)]
pub struct Info {
    /// Line number (1 - best, 2 - second the best, ...). If not send (single-line mode) it will be
    /// defaulted to 1.
    #[allow(unused)]
    pub multipv: u8,
    /// Engine evaluation
    pub score: Score,
    /// The engine line (`pv`)
    pub line: Vec<UciMove>,
    /// Actuall depth the calculation reached
    #[allow(unused)]
    pub depth: u8,
}

impl Info {
    /// Parses the `info` arguments
    fn parse(args: &str) -> Result<Option<Self>> {
        // `string` denotes debug information up until the end of the line - handing it before
        // splitting tokens to preserve form
        let args = match args.split_once(" string ") {
            Some((args, msg)) => {
                debug!(info = msg, "Engine info");
                // Purely debug info
                if args.trim().is_empty() {
                    return Ok(None);
                }
                args
            }
            None => args,
        };

        let mut args = args.split_whitespace().peekable();

        let mut multipv = 1;
        let mut depth = 0;
        let mut score = None;
        let mut line = vec![];

        while let Some(token) = args.next() {
            match token {
                "multipv" => {
                    multipv = args
                        .next()
                        .ok_or_eyre("Missing multipv value")?
                        .parse()
                        .wrap_err("Invalid multipv value")?;
                }
                "score" => {
                    let sc = match args.next().ok_or_eyre("Missing score type")? {
                        "cp" => {
                            let cp = args
                                .next()
                                .ok_or_eyre("Missing cp value")?
                                .parse()
                                .wrap_err("Invalid cp value")?;
                            Score::Cp(cp)
                        }
                        "mate" => {
                            let mate = args
                                .next()
                                .ok_or_eyre("Missing mate value")?
                                .parse()
                                .wrap_err("Invalid mate value")?;
                            Score::Mate(mate)
                        }
                        _ => bail!("Invalid score type"),
                    };
                    score = Some(sc);
                }
                "depth" => {
                    depth = args
                        .next()
                        .ok_or_eyre("Missing depth value")?
                        .parse()
                        .wrap_err("Invalid depth value")?;
                }
                "pv" => {
                    line.clear();
                    while let Some(mv) = args.peek().and_then(|m| m.parse().ok()) {
                        args.next();
                        line.push(mv);
                    }
                }
                _ => (),
            }
        }

        let score = score.ok_or_eyre("Score missing in info")?;
        if line.is_empty() {
            bail!("Line missing in info");
        }

        Ok(Some(Self {
            multipv,
            score,
            line,
            depth,
        }))
    }
}

/// Engine score evaluation
#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub enum Score {
    /// Centipawns score (from the engine PoV)
    Cp(i16),
    /// Mate in #moves (negative if the engine gets mated)
    Mate(i8),
}

impl Score {
    pub fn rev(self) -> Score {
        match self {
            Score::Cp(cp) => Score::Cp(-cp),
            Score::Mate(m) => Score::Mate(-m),
        }
    }
}

/// It's importat to be able to order the score to decide which line is better:
/// * The best is `Mate(n)` where `n >= 0`.
///   * `Mate(n) > Mate(m)` <=> `n < m` - the less moves to mate the better the move
/// * If there is no mate, `Cp` are ordered: `Cp(n) > Cp(m)` <=> `n > m`
/// * The worst are opponent mates - `Mate(n)` where `n < 0`
///   * `Mate(n) > Mate(m)` <=> `n > m` - if there are more moves to mate, thats better
impl Ord for Score {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use Score::*;

        match (self, other) {
            // Centipawns scores are just compared directly
            (Cp(n), Cp(m)) => n.cmp(m),
            // If we are mating on one side, that is the better side
            (Mate(n), Cp(_)) if *n >= 0 => Ordering::Greater,
            (Cp(_), Mate(m)) if *m >= 0 => Ordering::Less,
            // If they are mating on one side, that is the worse side
            (Mate(_n), Cp(_)) /* if *n < 0 */ => Ordering::Less,
            (Cp(_), Mate(_m)) /* if *m < 0 */ => Ordering::Greater,
            // If there are mates by the different players on both sides, better is side where we
            // are mating
            (Mate(n), Mate(m)) if *n >= 0 && *m < 1 => Ordering::Greater,
            (Mate(n), Mate(m)) if *n < 1 && *m >= 0 => Ordering::Greater,
            // If we are mating on both sides, we prefer the shorter mate (reversing the
            // comparison!)
            (Mate(n), Mate(m)) if *n >= 0 && *m >= 0 => m.cmp(n),
            // If they are mating on boths sides, we prefer the longer mate
            (Mate(n), Mate(m)) /* if *n < 0 && *m < 0 */ => n.cmp(m),
        }
    }
}

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cp(cp) => {
                let h = cp / 100;
                let l = cp.abs() % 100;
                write!(f, "{h}.{l}")
            }
            Self::Mate(m) => {
                write!(f, "#{m}")
            }
        }
    }
}
