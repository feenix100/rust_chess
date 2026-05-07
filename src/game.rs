//! Chess rules wrapper and move history management.
//!
//! This module wraps `shakmaty::Chess` with undo snapshots, SAN move history,
//! promotion helpers, and draw-condition tracking that extends beyond
//! `Chess::outcome()` to include threefold repetition and the fifty-move rule.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use shakmaty::{san::San, Chess, Color, File, Move, Outcome, Piece, Position, Rank, Square};

/// A high-level game outcome used by the UI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameOutcome {
    /// White has won by checkmate.
    Checkmate { winner: Color },
    /// The position is a stalemate.
    Stalemate,
    /// Both sides lack mating material.
    InsufficientMaterial,
    /// The halfmove clock reached 100 ply.
    FiftyMoveRule,
    /// The same position occurred at least three times.
    ThreefoldRepetition,
}

/// Immutable information about one recorded move.
#[derive(Clone, Debug)]
pub struct MoveRecord {
    san: String,
    played_move: Move,
    previous_position: Chess,
}

/// The mutable game state backing the app UI.
pub struct Game {
    /// The current chess position from the `shakmaty` rules engine.
    position: Chess,
    /// Move records are stored so the UI can show history and undo moves.
    history: Vec<MoveRecord>,
    /// Counts how many times each position has appeared for threefold repetition.
    repetition_counts: HashMap<Chess, u32>,
}

impl Game {
    /// Creates a new game in the standard starting position.
    pub fn new() -> Self {
        let position = Chess::default();
        let mut repetition_counts = HashMap::new();
        // The starting position has occurred once as soon as the game begins.
        repetition_counts.insert(position.clone(), 1);

        Self {
            position,
            history: Vec::new(),
            repetition_counts,
        }
    }

    /// Returns the current chess position.
    pub fn position(&self) -> &Chess {
        &self.position
    }

    /// Returns the piece currently on `square`, if any.
    pub fn piece_at(&self, square: Square) -> Option<Piece> {
        self.position.board().piece_at(square)
    }

    /// Returns the side to move.
    pub fn side_to_move(&self) -> Color {
        self.position.turn()
    }

    /// Returns all legal moves that start on `square`.
    pub fn legal_moves_from(&self, square: Square) -> Vec<Move> {
        // shakmaty gives all legal moves. The UI often needs only the moves
        // that start from one clicked square, so we filter them here.
        self.position
            .legal_moves()
            .into_iter()
            .filter(|candidate| move_from(candidate) == Some(square))
            .collect()
    }

    /// Returns all legal moves from `from` to `to`.
    pub fn legal_moves_between(&self, from: Square, to: Square) -> Vec<Move> {
        // There can be more than one move between two squares when a pawn
        // promotes, because queen/rook/bishop/knight are separate legal moves.
        self.legal_moves_from(from)
            .into_iter()
            .filter(|candidate| move_to(candidate) == Some(to))
            .collect()
    }

    /// Plays the provided legal move and records it in SAN history.
    ///
    /// Returns an error if `mv` is illegal in the current position.
    pub fn play(&mut self, mv: Move) -> Result<()> {
        // Validate before changing state. This protects callers from accidentally
        // pushing illegal moves into the history.
        if !self.position.is_legal(mv.clone()) {
            return Err(anyhow!("illegal move"));
        }

        // Save the old position so undo can restore it exactly.
        let previous_position = self.position.clone();
        // SAN is the short chess notation used in the sidebar and CSV export.
        let san = San::from_move(&previous_position, mv.clone()).to_string();
        let next_position = previous_position.clone().play(mv.clone())?;

        self.position = next_position.clone();
        // Track repetitions after the move reaches the new position.
        *self.repetition_counts.entry(next_position).or_insert(0) += 1;
        self.history.push(MoveRecord {
            san,
            played_move: mv,
            previous_position,
        });
        Ok(())
    }

    /// Undoes the most recent move, returning it if one existed.
    pub fn undo(&mut self) -> Option<Move> {
        // `pop` returns None when there is no history, which naturally means
        // there is nothing to undo.
        let record = self.history.pop()?;

        // The current position is being removed from the game history, so its
        // repetition count must be decreased too.
        if let Some(count) = self.repetition_counts.get_mut(&self.position) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.repetition_counts.remove(&self.position);
            }
        }

        self.position = record.previous_position;
        Some(record.played_move)
    }

    /// Resets the game to the standard starting position.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Returns the SAN strings for all played moves.
    pub fn san_history(&self) -> Vec<&str> {
        self.history
            .iter()
            .map(|record| record.san.as_str())
            .collect()
    }

    /// Returns the current game outcome, if the game is over.
    pub fn outcome(&self) -> Option<GameOutcome> {
        // Checkmate is detected first because it is the most specific decisive
        // outcome and determines the winner.
        if self.position.is_checkmate() {
            return Some(GameOutcome::Checkmate {
                winner: opposite_color(self.position.turn()),
            });
        }

        if self.position.is_stalemate() {
            return Some(GameOutcome::Stalemate);
        }

        if self.position.is_insufficient_material() {
            return Some(GameOutcome::InsufficientMaterial);
        }

        if self.position.halfmoves() >= 100 {
            return Some(GameOutcome::FiftyMoveRule);
        }

        // The HashMap tracks exact repeated positions as the game is played.
        if self
            .repetition_counts
            .get(&self.position)
            .copied()
            .unwrap_or_default()
            >= 3
        {
            return Some(GameOutcome::ThreefoldRepetition);
        }

        match self.position.outcome() {
            Outcome::Unknown => None,
            Outcome::Known(_) => None,
        }
    }

    /// Returns whether the side to move is currently in check.
    pub fn is_check(&self) -> bool {
        self.position.is_check()
    }

    /// Returns whether there is any move history to undo.
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    /// Returns the origin and destination squares for the last played move.
    pub fn last_move_squares(&self) -> Option<(Square, Square)> {
        // `?` exits with None if there is no last move or if the move type has
        // no normal from/to squares.
        let mv = &self.history.last()?.played_move;
        Some((move_from(mv)?, move_to(mv)?))
    }

    /// Returns a formatted move list with move numbers.
    pub fn formatted_move_list(&self) -> Vec<String> {
        // Build display strings from the same row helper used by CSV export so
        // both outputs stay consistent.
        self.history
            .iter()
            .enumerate()
            .map(|(index, record)| {
                let row = move_list_row(index, record);
                match (row.from, row.to) {
                    (Some(from), Some(to)) => {
                        format!("{} {}: {from} -> {to}", row.move_prefix, row.color)
                    }
                    _ => format!("{} {}: {}", row.move_prefix, row.color, record.san),
                }
            })
            .collect()
    }

    /// Returns the move list as CSV text.
    pub fn move_list_csv(&self) -> String {
        // Constructing a plain String keeps this dependency-free; no CSV crate
        // is needed for this small export.
        let mut csv = "move_number,side,from,to,san\n".to_owned();

        for (index, record) in self.history.iter().enumerate() {
            let row = move_list_row(index, record);
            let from = row.from.map(square_text).unwrap_or_default();
            let to = row.to.map(square_text).unwrap_or_default();
            csv.push_str(&format!(
                "{},{},{},{},{}\n",
                row.move_number,
                csv_escape(row.color),
                csv_escape(&from),
                csv_escape(&to),
                csv_escape(record.san.as_str()),
            ));
        }

        csv
    }
}

struct MoveListRow {
    move_number: usize,
    move_prefix: String,
    color: &'static str,
    from: Option<Square>,
    to: Option<Square>,
}

/// Builds a user-facing status string for the current game state.
pub fn status_text(game: &Game) -> String {
    if let Some(outcome) = game.outcome() {
        return match outcome {
            GameOutcome::Checkmate {
                winner: Color::White,
            } => "Checkmate - White wins".to_owned(),
            GameOutcome::Checkmate {
                winner: Color::Black,
            } => "Checkmate - Black wins".to_owned(),
            GameOutcome::Stalemate => "Stalemate - draw".to_owned(),
            GameOutcome::InsufficientMaterial => "Draw - insufficient material".to_owned(),
            GameOutcome::FiftyMoveRule => "Draw - 50-move rule".to_owned(),
            GameOutcome::ThreefoldRepetition => "Draw - threefold repetition".to_owned(),
        };
    }

    if game.is_check() {
        format!("Check! {} to move", color_name(game.side_to_move()))
    } else {
        format!("{} to move", color_name(game.side_to_move()))
    }
}

/// Converts a board coordinate into a `Square`.
pub fn square_from_indices(file_index: usize, rank_index: usize) -> Square {
    Square::from_coords(File::new(file_index as u32), Rank::new(rank_index as u32))
}

fn color_name(color: Color) -> &'static str {
    match color {
        Color::White => "White",
        Color::Black => "Black",
    }
}

fn opposite_color(color: Color) -> Color {
    match color {
        Color::White => Color::Black,
        Color::Black => Color::White,
    }
}

fn move_from(mv: &Move) -> Option<Square> {
    match mv {
        Move::Normal { from, .. } => Some(*from),
        Move::EnPassant { from, .. } => Some(*from),
        // For castling, shakmaty stores the king's starting square here.
        Move::Castle { king, .. } => Some(*king),
        _ => None,
    }
}

fn move_to(mv: &Move) -> Option<Square> {
    match mv {
        Move::Normal { to, .. } => Some(*to),
        Move::EnPassant { to, .. } => Some(*to),
        // shakmaty stores the rook square as the castle target. The UI and move
        // list use the king's final square instead because that is what players
        // expect to see.
        Move::Castle { king, .. } => mv
            .castling_side()
            .map(|side| Square::from_coords(side.king_to_file(), king.rank())),
        _ => None,
    }
}

fn move_squares(mv: &Move) -> Option<(Square, Square)> {
    Some((move_from(mv)?, move_to(mv)?))
}

fn move_list_row(index: usize, record: &MoveRecord) -> MoveListRow {
    // `index` counts plies: 0 for White's first move, 1 for Black's first move,
    // 2 for White's second move, and so on.
    let move_number = index / 2 + 1;
    let move_prefix = if index % 2 == 0 {
        format!("{move_number}.")
    } else {
        format!("{move_number}...")
    };
    let color = color_name(record.previous_position.turn());
    let (from, to) = move_squares(&record.played_move)
        .map(|(from, to)| (Some(from), Some(to)))
        .unwrap_or((None, None));

    MoveListRow {
        move_number,
        move_prefix,
        color,
        from,
        to,
    }
}

fn square_text(square: Square) -> String {
    square.to_string()
}

fn csv_escape(value: &str) -> String {
    // CSV fields containing commas, quotes, or newlines must be wrapped in
    // quotes. Any quote inside the field becomes two quotes.
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}
