//! Wrappers to alter how some types are displayed. All wrappers implements `Debug` to manipulate
//! how they are displayed.

use std::fmt::{Debug, Formatter, Result};

use shakmaty::fen::Fen;
use shakmaty::san::San;
use shakmaty::uci::UciMove;
use shakmaty::{Chess, EnPassantMode, Move};

/// Wrapper formatting an `Option` flattening the `Some`
pub struct FlatOpt<'a, T: ?Sized>(&'a T);

pub trait FlatOptExt {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result;

    fn d_opt(&self) -> FlatOpt<Self> {
        FlatOpt(self)
    }
}

impl<T> Debug for FlatOpt<'_, T>
where
    T: FlatOptExt + ?Sized,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        FlatOptExt::fmt(self.0, f)
    }
}

impl<T> FlatOptExt for Option<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Some(t) => t.fmt(f),
            None => write!(f, "None"),
        }
    }
}

/// Wrapper formatting a type as it's FEN
pub struct DFen<'a, T: ?Sized>(&'a T);

pub trait DFenExt {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result;

    fn d_fen(&self) -> DFen<Self> {
        DFen(self)
    }
}

impl<T> Debug for DFen<'_, T>
where
    T: DFenExt + ?Sized,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        DFenExt::fmt(self.0, f)
    }
}

impl DFenExt for Fen {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "[{self}]")
    }
}

impl DFenExt for Chess {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        DFenExt::fmt(&Fen::from_position(self.clone(), EnPassantMode::Legal), f)
    }
}

impl<T> DFenExt for Option<T>
where
    T: DFenExt,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Some(fen) => DFenExt::fmt(fen, f),
            None => write!(f, "[init]"),
        }
    }
}

/// Wrapper formatting a type as it's chess line
pub struct Line<'a, T: ?Sized>(&'a T);

pub trait LineExt {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result;

    fn d_line(&self) -> Line<Self> {
        Line(self)
    }
}

impl<T> Debug for Line<'_, T>
where
    T: LineExt + ?Sized,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        LineExt::fmt(self.0, f)
    }
}

impl LineExt for [UciMove] {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "<")?;

        if !self.is_empty() {
            write!(f, "{}", self[0])?;
        }

        if self.len() > 1 {
            for mov in &self[1..] {
                write!(f, " {mov}")?;
            }
        }

        write!(f, ">")
    }
}

impl LineExt for [Move] {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "<")?;

        if !self.is_empty() {
            write!(f, "{}", self[0])?;
        }

        if self.len() > 1 {
            for mov in &self[1..] {
                write!(f, " {mov}")?;
            }
        }

        write!(f, ">")
    }
}

impl<T> LineExt for Vec<T>
where
    [T]: LineExt,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        LineExt::fmt(&self[..], f)
    }
}

/// Wrapper formatting a type as it's chess move
pub struct Mov<'a, T: ?Sized>(&'a T);

pub trait MovExt {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result;

    fn d_mov(&self) -> Mov<'_, Self> {
        Mov(self)
    }
}

impl<T> Debug for Mov<'_, T>
where
    T: MovExt,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        MovExt::fmt(self.0, f)
    }
}

impl MovExt for Move {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{self}")
    }
}

impl MovExt for San {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{self}")
    }
}

impl MovExt for UciMove {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{self}")
    }
}
