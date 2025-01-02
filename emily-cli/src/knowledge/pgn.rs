use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU32;

use chrono::Local;
use shakmaty::fen::Fen;
use shakmaty::san::San;
use shakmaty::{Chess, Color, EnPassantMode, Move, Outcome, Position};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tracing::{debug, instrument, warn};

use crate::uci::Score;

use super::Knowledge;
use crate::Result;

/// [Knowledge] preprocessed for `PGN` storage
#[derive(Debug)]
pub struct Pgn {
    /// Starting position
    root: Chess,
    /// Game outcome
    outcome: Option<Outcome>,
    /// Positions in game
    data: HashMap<Chess, PosInfo>,
}

#[derive(Debug, Clone, Copy)]
struct MoveNo(NonZeroU32, Color);

impl MoveNo {
    fn new(pos: &impl Position) -> Self {
        let mov = pos.fullmoves();
        let color = pos.turn();
        Self(mov, color)
    }

    fn next(self) -> Self {
        match self.1 {
            Color::White => Self(self.0, Color::Black),
            Color::Black => Self(self.0.saturating_add(1), Color::White),
        }
    }
}

impl std::fmt::Display for MoveNo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.1 {
            Color::White => write!(f, "{}.", self.0),
            Color::Black => write!(f, "{}...", self.0),
        }
    }
}

impl Pgn {
    pub fn new(knowledge: &Knowledge) -> Result<Self> {
        let mut queue = VecDeque::new();
        let mut data = HashMap::new();
        queue.push_back(knowledge.root.clone());

        while let Some(fen) = queue.pop_front() {
            let Some(info) = knowledge.data.get(&fen) else {
                continue;
            };

            let info = match PosInfo::new(&fen, info) {
                Ok(info) => info,
                Err(err) => {
                    warn!(?err, "Cannot convert position to PGN");
                    continue;
                }
            };

            for mov in &info.moves {
                if !data.contains_key(&mov.pos) {
                    queue.push_back(mov.pos.clone())
                }
            }

            data.insert(fen, info);
        }

        let last = std::iter::successors(Some(&knowledge.root), |pos| {
            data.get(pos)
                .and_then(|info| info.moves.first())
                .map(|mov| &mov.pos)
        })
        // If we visit more positions that we collected, there is a loop which means it is a draw
        .take(data.len())
        .last()
        .unwrap_or(&knowledge.root);

        Ok(Self {
            root: knowledge.root.clone(),
            outcome: last.outcome(),
            data,
        })
    }

    async fn write_result<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> Result<()> {
        let result = match self.outcome {
            Some(Outcome::Draw) => "1/2-1/2",
            Some(Outcome::Decisive {
                winner: Color::White,
            }) => "1-0",
            Some(Outcome::Decisive {
                winner: Color::Black,
            }) => "0-1",
            None => "*",
        };

        writer.write_all(result.as_bytes()).await?;
        Ok(())
    }

    async fn write_tags<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> Result<()> {
        let date = Local::now();

        writer.write_all(b"[Event \"?\"]\n").await?;
        writer.write_all(b"[Event \"?\"]\n").await?;
        writer.write_all(b"[Site \"?\"]\n").await?;
        writer.write_all(b"[Date \"").await?;
        writer
            .write_all(date.format("%Y.%m.%d").to_string().as_bytes())
            .await?;
        writer.write_all(b"\"]\n").await?;
        writer.write_all(b"[Round \"?\"]\n").await?;
        writer.write_all(b"[White \"?\"]\n").await?;
        writer.write_all(b"[Black \"?\"]\n").await?;
        writer.write_all(b"[Result \"").await?;
        self.write_result(writer).await?;
        writer.write_all(b"\"]\n").await?;

        if self.root != Chess::new() {
            writer.write_all(b"[SetUp \"1\"]").await?;
            writer.write_all(b"[FEN \"").await?;
            writer
                .write_all(
                    Fen::from_position(self.root.clone(), EnPassantMode::Always)
                        .to_string()
                        .as_bytes(),
                )
                .await?;
            writer.write_all(b"\"]\n").await?;
        }

        Ok(())
    }

    async fn write_comment<W: AsyncWrite + Unpin>(
        _move_info: &MoveInfo,
        pos_info: Option<&PosInfo>,
        writer: &mut W,
    ) -> Result<()> {
        writer.write_all(b" { ").await?;
        if let Some(pos_info) = pos_info {
            if let Some(eval) = pos_info.eval {
                writer.write_all(b"Eval: ").await?;
                writer.write_all(eval.to_string().as_bytes()).await?;
                writer.write_all(b" ").await?;
            }
        }
        writer.write_all(b"}\n").await?;

        Ok(())
    }

    async fn write_moves<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> Result<()> {
        debug!(data = self.data.len(), "Storing moves");
        let mut stack: Vec<_> = self
            .data
            .get(&self.root)
            .map(|pos| (pos, 0, MoveNo::new(&self.root)))
            .into_iter()
            .collect();

        while let Some((pos, mov_idx, mov_no)) = stack.last_mut() {
            if *mov_idx > 0 {
                // Starting new variation
                writer.write_all(b"(").await?;
            }

            let mov = &pos.moves[*mov_idx];
            let new_pos = self.data.get(&mov.pos);
            writer.write_all(mov_no.to_string().as_bytes()).await?;
            writer.write_all(b" ").await?;
            writer.write_all(mov.mov.to_string().as_bytes()).await?;
            Self::write_comment(mov, new_pos, writer).await?;
            *mov_idx += 1;

            let next_mov_no = mov_no.next();
            if *mov_idx >= pos.moves.len() {
                stack.pop();
                // If that was not the last item - variation just finished
                if !stack.is_empty() {
                    writer.write_all(b")").await?;
                }
            }

            let new_pos = new_pos
                .map(|pos| (pos, 0, next_mov_no))
                .filter(|(pos, _, _)| !pos.moves.is_empty());

            stack.extend(new_pos)
        }

        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn write_pgn<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> Result<()> {
        self.write_tags(writer).await?;
        writer.write_all(b"\n").await?;
        self.write_moves(writer).await?;
        self.write_result(writer).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct PosInfo {
    /// Ordered moves. The first move is always a main line.
    moves: Vec<MoveInfo>,
    /// Engine evaluation.
    eval: Option<Score>,
}

impl PosInfo {
    fn new(fen: &Chess, info: &super::PosInfo) -> Result<Self> {
        let moves = info
            .moves
            .iter()
            .map(|(mov, info)| MoveInfo::new(fen, mov, info))
            .collect();

        Ok(Self {
            moves,
            eval: info.eval,
        })
    }
}

#[derive(Debug)]
pub struct MoveInfo {
    /// Move played
    mov: San,
    /// Position after move
    pos: Chess,
}

impl MoveInfo {
    fn new(board: &Chess, mov: &Move, info: &super::MoveInfo) -> Self {
        let mov = San::from_move(board, mov);

        Self {
            mov,
            pos: info.pos.clone(),
        }
    }
}
