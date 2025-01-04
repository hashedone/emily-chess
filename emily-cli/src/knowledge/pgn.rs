use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::num::NonZeroU32;

use chrono::Local;
use shakmaty::fen::Fen;
use shakmaty::san::San;
use shakmaty::{Chess, Color, EnPassantMode, Move, Outcome, Position};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tracing::{debug, instrument, warn};

use crate::adapters::TracingAdapt;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl PartialOrd for MoveNo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MoveNo {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (MoveNo(m1, Color::White), MoveNo(m2, Color::Black)) if m1 == m2 => {
                std::cmp::Ordering::Less
            }
            (MoveNo(m1, Color::Black), MoveNo(m2, Color::White)) if m1 == m2 => {
                std::cmp::Ordering::Greater
            }
            (MoveNo(m1, _), MoveNo(m2, _)) if m1 == m2 => std::cmp::Ordering::Equal,
            (MoveNo(m1, _), MoveNo(m2, _)) => m1.cmp(m2),
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
        .take(data.len())
        .last()
        .unwrap_or(&knowledge.root);

        let mut res = Self {
            root: knowledge.root.clone(),
            outcome: last.outcome(),
            data,
        };
        res.populate_pathes();

        Ok(res)
    }

    /// Calculates the cardinal path to each move
    #[instrument(skip_all)]
    fn populate_pathes(&mut self) {
        match self.data.get(&self.root) {
            None => return,
            Some(root) if root.moves.is_empty() => return,
            Some(_) => (),
        };

        let mut stack = vec![(self.root.clone(), 0, MoveNo::new(&self.root))];

        while let Some((pos, movidx, movno)) = stack.last_mut() {
            let Some(mut info) = self.data.get(pos).cloned() else {
                // This should never happen
                stack.pop();
                continue;
            };

            // Check if the position should be maintained on the stack or not, and if so - update
            // it for the next moveidx
            let (pos, movidx, movno) = if *movidx + 1 < info.moves.len() {
                *movidx += 1;
                (pos.clone(), *movidx, *movno)
            } else {
                stack.pop().unwrap()
            };

            let mov = &info.moves[movidx];
            let nextpos = &mov.pos;

            // Prevent updating the root first
            if *nextpos != self.root {
                if let Some(nextinfo) = self.data.get_mut(nextpos) {
                    if info.path.path.last().map(|(_, _, idx)| *idx) == Some(0) {
                        // Last move was not branching - removing as it is not relevant
                        info.path.path.pop();
                    }
                    info.path.path.push((movno, mov.mov.clone(), movidx));

                    // Path is update - we will need to propagate it.
                    if nextinfo.path.update(info.path, pos) && !nextinfo.moves.is_empty() {
                        stack.push((nextpos.clone(), 0, movno.next()))
                    }
                }
            }
        }
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
        transposition: bool,
        writer: &mut W,
    ) -> Result<()> {
        writer.write_all(b" { ").await?;
        if let Some(pos_info) = pos_info {
            if let Some(eval) = pos_info.eval {
                writer.write_all(b"Eval: ").await?;
                writer.write_all(eval.to_string().as_bytes()).await?;
                writer.write_all(b", ").await?;
            }

            if transposition {
                writer.write_all(b"Transposes: ").await?;
                writer
                    .write_all(pos_info.path.to_string().as_bytes())
                    .await?;
            }
        }
        writer.write_all(b"}\n").await?;

        Ok(())
    }

    #[instrument(skip_all)]
    async fn write_moves<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> Result<()> {
        debug!(count = self.data.len(), "Storing moves");
        let mut stack: Vec<_> = vec![(None, self.root.clone(), 0, MoveNo::new(&self.root))];

        while let Some((parent, pos, mov_idx, mov_no)) = stack.last_mut() {
            let Some(info) = self.data.get(pos) else {
                warn!(pos = pos.tr(), "Missing position info");
                stack.pop();
                continue;
            };

            if info.moves.is_empty() {
                debug!(pos = pos.tr(), "Final position, no moves to store");
                stack.pop();
                continue;
            }

            let mov = &info.moves[*mov_idx];

            debug!(pos = pos.tr(), mov = %mov.mov, "Storing next move");
            if *mov_idx > 0 {
                debug!("New variation");
                // Starting new variation
                writer.write_all(b"(").await?;
            }

            writer.write_all(mov_no.to_string().as_bytes()).await?;
            writer.write_all(b" ").await?;
            writer.write_all(mov.mov.to_string().as_bytes()).await?;
            *mov_idx += 1;

            let new_pos = self.data.get(&mov.pos);
            let parent = parent.clone();
            let pos = pos.clone();
            let next_mov_no = mov_no.next();
            let mov_idx = *mov_idx;

            let transposition = parent != info.path.parent;
            Self::write_comment(mov, new_pos, transposition, writer).await?;

            if mov_idx >= info.moves.len() {
                stack.pop();
                // If that was not the last item - variation just finished
                if !stack.is_empty() {
                    writer.write_all(b")").await?;
                }
            }

            if !transposition {
                // We don't analyze transpositions further
                stack.push((Some(pos.clone()), mov.pos.clone(), 0, next_mov_no));
            }
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

#[derive(Debug, Clone)]
pub struct PosInfo {
    /// Cardinal path to reach the position
    ///
    /// Cardinal path to the root position is always empty
    ///
    /// The cardinal path for any move occuring on the main line is always the move on the main
    /// line itself.
    ///
    /// If the move is not on the main line, the cardinal position which reaches the position
    /// branching on smaller moves ids (so using better lines)
    ///
    /// If the pathes are picking equally good choices, then the shortest is cardinal.
    ///
    /// If the pathes are the same length, the one that branches earlier is cardinal.
    path: PosPath,
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
            path: PosPath::default(),
        })
    }
}

#[derive(Debug, Clone)]
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

/// Describes the path to reach a position for describing transposition.
///
/// The path is list of half-moves where not the first move is choosen plus the last half-move
/// played (even if it was the first one) starting at the root position.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PosPath {
    /// The path to reach this position. The last item on the path node is the index of which move
    /// id it was, so how "good" choice it was (lower = better)
    path: Vec<(MoveNo, San, usize)>,
    /// The previous position on this path (`None` for root)
    parent: Option<Chess>,
}

impl PartialOrd for PosPath {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The better position which reaches the position branching on smaller moves idices (so using better lines)
///
/// If the pathes are picking equally good choices, then the shortest is better.
///
/// If the pathes are the same length, the one that branches earlier is better.
impl Ord for PosPath {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let (tie0, tie2) = self
            .path
            .iter()
            .zip(&other.path)
            .map(|(mov1, mov2)| {
                // Returns tiebreakers - first is to determine which path takes lower indicies, second
                // - which one branches earlier
                (mov1.2.cmp(&mov2.2), mov1.0.cmp(&mov2.0))
            })
            .fold(
                (std::cmp::Ordering::Equal, std::cmp::Ordering::Equal),
                |(tie0, tie1), (newt0, newt1)| {
                    use std::cmp::Ordering::*;

                    let tie0 = match (tie0, newt0) {
                        (Equal, new) => new,
                        (old, _) => old,
                    };

                    let tie1 = match (tie1, newt1) {
                        (Equal, new) => new,
                        (old, _) => old,
                    };

                    (tie0, tie1)
                },
            );

        if tie0 != std::cmp::Ordering::Equal {
            return tie0;
        }

        match self.path.len().cmp(&other.path.len()) {
            std::cmp::Ordering::Equal => (),
            ord => return ord,
        }

        tie2
    }
}

impl PosPath {
    /// Updates the path if the new path is better. The last move on the path is provided
    /// separately. The empty path is assumed not to be a root, so it would always be updated!
    ///
    /// Returns if the path was updated
    fn update(&mut self, path: PosPath, parent: Chess) -> bool {
        if self.path.is_empty() || path < *self {
            self.path = path.path;
            self.parent = Some(parent);
            true
        } else {
            false
        }
    }
}

impl Display for PosPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut it = self.path.iter();

        if let Some((mov, san, _)) = it.next() {
            write!(f, "{mov} {san}")?;
        } else {
            write!(f, "(root)")?;
            return Ok(());
        }

        for (mov, san, _) in it {
            write!(f, " {mov} {san}")?;
        }
        Ok(())
    }
}
