use std::num::NonZeroU32;

use chrono::Local;
use shakmaty::fen::Fen;
use shakmaty::san::San;
use shakmaty::{Chess, Color, EnPassantMode, Outcome, Position};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tracing::{debug, instrument};

use super::{Knowledge, MoveInfo, PosInfo, Variation};
use crate::Result;

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

/// A single PGN move
#[derive(Debug, Clone)]
struct Mov<'a> {
    /// Move played
    mov: San,
    /// Move number
    no: MoveNo,
    /// Information the played move
    #[allow(unused)]
    movinfo: Option<&'a MoveInfo>,
    /// Information about the position after the move played
    posinfo: &'a PosInfo,
}

impl Mov<'_> {
    async fn write_comment<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> Result<()> {
        writer.write_all(b" { ").await?;
        if let Some(eval) = self.posinfo.eval {
            writer.write_all(b"Eval: ").await?;
            writer.write_all(eval.to_string().as_bytes()).await?;
            writer.write_all(b", ").await?;
        }
        writer.write_all(b"}\n").await?;

        Ok(())
    }

    async fn write<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> Result<()> {
        writer.write_all(self.no.to_string().as_bytes()).await?;
        writer.write_all(b" ").await?;
        writer.write_all(self.mov.to_string().as_bytes()).await?;
        self.write_comment(writer).await
    }
}

/// Single PGN node. Node is a line up until first branch followed by all the possible moves in
/// branching.
#[derive(Debug, Clone)]
struct Node<'a> {
    /// Moves up until first branch
    line: Vec<Mov<'a>>,
    /// Branching options
    branches: Vec<Node<'a>>,
    /// Main line outcome of this node
    outcome: Option<Outcome>,
}

impl Node<'_> {
    async fn write_line<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> Result<()> {
        for mov in &self.line {
            mov.write(writer).await?;
        }

        Ok(())
    }
}

impl<'a> Node<'a> {
    /// Adds `mov` move to the node after the `hm` move. Returns node it was added to (can change
    /// if branch was created) and `hm` of the new move in returned node.
    ///
    /// Assumes there is at least one move on the line.
    fn add_move(
        &mut self,
        hm: usize,
        mov: San,
        movinfo: Option<&'a MoveInfo>,
        posinfo: &'a PosInfo,
    ) -> (&mut Self, usize) {
        if hm == self.line.len() && self.branches.is_empty() {
            // Adding a move to the variation
            self.line.push(Mov {
                mov,
                no: self.line[hm - 1].no.next(),
                movinfo,
                posinfo,
            });
            (self, hm + 1)
        } else if hm == self.line.len() {
            // Branching point reached. Looking for variation to follow, or adding
            // new variation.
            match self
                .branches
                .iter_mut()
                .position(|branch| branch.line[0].mov == mov)
            {
                // Following the branch
                Some(idx) => (&mut self.branches[idx], 1),
                None => {
                    // Adding the new branch
                    self.branches.push(Node {
                        line: vec![Mov {
                            mov,
                            no: self.line[hm - 1].no.next(),
                            movinfo,
                            posinfo,
                        }],
                        branches: vec![],
                        outcome: None,
                    });
                    (self.branches.last_mut().unwrap(), 1)
                }
            }
        } else if self.line[hm].mov == mov {
            // Following the main line
            (self, hm + 1)
        } else {
            // Creating branching point
            self.branches.push(Node {
                line: self.line.split_off(hm),
                branches: vec![],
                outcome: self.outcome,
            });

            self.branches.push(Node {
                line: vec![Mov {
                    mov,
                    no: self.line[0].no.next(),
                    movinfo,
                    posinfo,
                }],
                branches: vec![],
                outcome: None,
            });

            (self.branches.last_mut().unwrap(), 1)
        }
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

/// [Knowledge] preprocessed for `PGN` storage
#[derive(Debug)]
pub struct Pgn<'a> {
    /// Starting position
    rootinfo: &'a PosInfo,
    /// Starting node
    line: Node<'a>,
}

impl<'a> Pgn<'a> {
    /// Orders variations
    ///
    /// The main line would always end up first, and will never be empty. The result would be empty
    /// if and only if all the variations are empty
    ///
    /// The multi-game PGNs are not supported, and variations not strting on the same position
    /// as the mainline would be ingnored.
    ///
    /// The mainline can be extended by later variation if it is its prefix.
    fn order_variations(knowledge: &Knowledge) -> Vec<&Variation> {
        let main = &knowledge.variations[knowledge.main];
        let main = match main.moves.is_empty() {
            true => knowledge
                .variations
                .iter()
                .find(|variation| !variation.moves.is_empty()),
            false => Some(main),
        };

        let Some(main) = main else { return vec![] };
        let root = main.positions[0];

        let mut variations: Vec<_> = knowledge
            .variations
            .iter()
            .filter(|variation| variation.positions[0] == root)
            .collect();
        variations.swap(0, knowledge.main);

        variations
    }

    /// Prepares PGN form the Knowledge
    pub fn new(knowledge: &'a Knowledge) -> Self {
        let variations = Self::order_variations(knowledge);
        let mut pgn = Self {
            rootinfo: knowledge.root(),
            line: Node {
                line: vec![],
                branches: vec![],
                outcome: None,
            },
        };

        // No moves edge case. We can safely use `Iterator::all` here as there is at least one
        // variation by construction.
        if variations.is_empty() {
            return pgn;
        }

        // Adding the first move immediately to satisfy `Node::add_move`
        let main = &variations[0];
        pgn.rootinfo = knowledge.position(main.positions[0]);
        pgn.line.line.push(Mov {
            mov: San::from_move(&pgn.rootinfo.pos, &main.moves[0]),
            no: MoveNo::new(pgn.rootinfo.position()),
            movinfo: pgn.rootinfo.moves.get(&main.moves[0]),
            // Position after the move!
            posinfo: knowledge.position(main.positions[1]),
        });

        for variation in variations {
            let movinfos =
                variation
                    .moves
                    .iter()
                    .zip(&variation.positions[..])
                    .map(|(mov, pos)| {
                        let position = knowledge.position(*pos);
                        let movinfo = position.moves.get(mov);
                        let mov = San::from_move(&position.pos, mov);
                        (mov, movinfo)
                    });

            let posinfos = variation.positions[1..]
                .iter()
                .map(|position| knowledge.position(*position));

            let (node, _) = movinfos
                .zip(posinfos)
                .map(|((mov, movinfo), posinfo)| (mov, movinfo, posinfo))
                .fold((&mut pgn.line, 0), |(node, hm), (mov, movinfo, posinfo)| {
                    node.add_move(hm, mov, movinfo, posinfo)
                });

            node.outcome = variation.outcome;
        }

        pgn
    }

    async fn write_result<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> Result<()> {
        let result = match self.line.outcome {
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

        if *self.rootinfo.position() != Chess::new() {
            writer.write_all(b"[SetUp \"1\"]").await?;
            writer.write_all(b"[FEN \"").await?;
            writer
                .write_all(
                    Fen::from_position(self.rootinfo.position().clone(), EnPassantMode::Always)
                        .to_string()
                        .as_bytes(),
                )
                .await?;
            writer.write_all(b"\"]\n").await?;
        }

        Ok(())
    }

    #[instrument(skip_all)]
    async fn write_moves<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> Result<()> {
        debug!("Storing PGN");
        self.line.write_line(writer).await?;

        if self.line.branches.is_empty() {
            // Flat PGN
            return Ok(());
        }

        let mut stack: Vec<_> = vec![(&self.line, 0)];

        while let Some((line, branchidx)) = stack.last_mut() {
            if *branchidx >= line.branches.len() {
                // Visited all the branches
                stack.pop();
                writer.write_all(b")").await?;
                continue;
            }

            if *branchidx > 0 {
                // Opening new variation (0-th branch is mainline continuation)
                writer.write_all(b"(").await?;
            }

            let branch = &line.branches[*branchidx];
            branch.write_line(writer).await?;

            *branchidx += 1;
            match branch.branches.is_empty() {
                // Flat branch, immediately close the variation. Note that `branchidx` is already
                // incremented
                true if *branchidx > 1 => writer.write_all(b")").await?,
                // Branching, add variation to the stack
                false => stack.push((branch, 0)),
                // Flat branch, main line - don't need to close
                _ => (),
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
