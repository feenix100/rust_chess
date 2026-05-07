//! Board rendering and click hit-testing.
//!
//! This module exposes a board widget that paints the square textures, pieces,
//! selection state, and legal-move markers. It also converts pointer clicks
//! back into `shakmaty::Square` values while respecting flipped orientation.

use anyhow::Result;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use shakmaty::{File, Position, Rank, Square};

use crate::assets::{piece_asset_key, AssetKey, AssetStore, BoardTheme};
use crate::game::{square_from_indices, Game, GameOutcome};

/// Immutable board rendering options for a single frame.
pub struct BoardView<'a> {
    /// The active game state.
    pub game: &'a Game,
    /// Embedded assets and raster cache.
    pub assets: &'a mut AssetStore,
    /// Whether black should appear at the bottom.
    pub flipped: bool,
    /// Whether the last move should be highlighted.
    pub show_last_move: bool,
    /// The current board color theme.
    pub theme: BoardTheme,
    /// The selected source square, if any.
    pub selected: Option<Square>,
    /// Legal destination squares from the selected piece.
    pub legal_destinations: &'a [Square],
    /// Square currently targeted by keyboard navigation, if enabled.
    pub keyboard_cursor: Option<Square>,
}

/// Result of showing the board widget.
pub struct BoardResponse {
    /// The square that was clicked this frame, if any.
    pub clicked_square: Option<Square>,
}

/// Paints the chess board and returns the clicked square, if any.
///
/// Returns a rendering error if an embedded SVG cannot be rasterized.
pub fn show_board(ui: &mut Ui, view: BoardView<'_>) -> Result<BoardResponse> {
    let available = ui.available_size();
    // Keep the board square by using the smaller available dimension.
    let side = available.x.min(available.y).max(1.0);
    // `allocate_exact_size` reserves space and also gives a click response.
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    let mut view = view;

    // Paint order matters. Later calls draw on top of earlier calls.
    paint_squares(ui, rect, &mut view)?;
    if view.show_last_move {
        paint_last_move_highlight(ui, rect, &view);
    }
    paint_highlights(ui, rect, &view);
    paint_pieces(ui, rect, &mut view)?;
    paint_coordinates(ui, rect, view.flipped);
    paint_checkmate_overlay(ui, rect, &view);

    let clicked_square = if response.clicked() {
        // Pointer coordinates are screen/UI coordinates. Convert them back into
        // a chess square before returning the click to the app layer.
        response
            .interact_pointer_pos()
            .and_then(|pointer| square_at_pointer(rect, pointer, view.flipped))
    } else {
        None
    };

    let _ = response;
    Ok(BoardResponse { clicked_square })
}

fn paint_squares(ui: &mut Ui, rect: Rect, view: &mut BoardView<'_>) -> Result<()> {
    let square_size = rect.width() / 8.0;
    // Rasterize each square SVG at roughly the size it appears on screen.
    let texture_size = square_size.ceil().max(1.0) as u32;

    for display_rank in 0..8 {
        for display_file in 0..8 {
            // display_file/display_rank are screen positions. board_square is
            // the real chess square at that screen position.
            let board_square = displayed_square(display_file, display_rank, view.flipped);
            let is_light = (display_file + display_rank) % 2 == 0;
            let square_rect = square_rect(rect, display_file, display_rank);
            let texture = view.assets.texture(
                ui.ctx(),
                AssetKey::Square {
                    theme: view.theme,
                    light: is_light,
                },
                texture_size,
            )?;

            ui.painter().image(
                texture.id(),
                square_rect,
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                Color32::WHITE,
            );

            if Some(board_square) == view.selected {
                // Draw the selected square outline after the square image so it
                // remains visible.
                ui.painter().rect_stroke(
                    square_rect.shrink(2.0),
                    6.0,
                    Stroke::new(3.0, Color32::from_rgb(245, 199, 73)),
                    StrokeKind::Inside,
                );
            }

            if Some(board_square) == view.keyboard_cursor {
                ui.painter().rect_stroke(
                    square_rect.shrink(6.0),
                    4.0,
                    Stroke::new(4.0, Color32::from_rgb(80, 235, 245)),
                    StrokeKind::Inside,
                );
            }
        }
    }

    Ok(())
}

fn paint_last_move_highlight(ui: &mut Ui, rect: Rect, view: &BoardView<'_>) {
    let Some((from, to)) = view.game.last_move_squares() else {
        return;
    };

    // Highlight both ends of the last move: where the piece started and where
    // it landed.
    for square in [from, to] {
        let Some((display_file, display_rank)) = display_coords(square, view.flipped) else {
            continue;
        };

        ui.painter().rect_stroke(
            square_rect(rect, display_file, display_rank).shrink(3.0),
            4.0,
            Stroke::new(4.0, Color32::from_rgb(90, 170, 255)),
            StrokeKind::Inside,
        );
    }
}

fn paint_highlights(ui: &mut Ui, rect: Rect, view: &BoardView<'_>) {
    for destination in view.legal_destinations {
        if let Some((display_file, display_rank)) = display_coords(*destination, view.flipped) {
            let square_rect = square_rect(rect, display_file, display_rank);
            let center = square_rect.center();

            if view
                .game
                .position()
                .board()
                .piece_at(*destination)
                .is_some()
            {
                // Captures get a ring around the target piece.
                ui.painter().circle_stroke(
                    center,
                    square_rect.width() * 0.32,
                    Stroke::new(5.0, Color32::from_rgba_unmultiplied(20, 20, 20, 180)),
                );
            } else {
                // Quiet moves get a smaller dot.
                ui.painter().circle_filled(
                    center,
                    square_rect.width() * 0.12,
                    Color32::from_rgba_unmultiplied(20, 20, 20, 150),
                );
            }
        }
    }
}

fn paint_pieces(ui: &mut Ui, rect: Rect, view: &mut BoardView<'_>) -> Result<()> {
    let texture_size = (rect.width() / 8.0).ceil().max(1.0) as u32;

    // Iterate over real board coordinates, then convert each occupied square to
    // display coordinates so flipped boards work automatically.
    for rank in 0..8 {
        for file in 0..8 {
            let square = square_from_indices(file, rank);
            let Some(piece) = view.game.piece_at(square) else {
                continue;
            };

            let Some((display_file, display_rank)) = display_coords(square, view.flipped) else {
                continue;
            };

            let texture = view
                .assets
                .texture(ui.ctx(), piece_asset_key(piece), texture_size)?;
            let piece_rect = square_rect(rect, display_file, display_rank).shrink(2.0);

            ui.painter().image(
                texture.id(),
                piece_rect,
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }

    Ok(())
}

fn paint_coordinates(ui: &mut Ui, rect: Rect, flipped: bool) {
    let painter = ui.painter();
    // Scale coordinate text with board size, but clamp it so it stays readable
    // and does not become huge on large windows.
    let font = FontId::proportional((rect.width() * 0.035).clamp(18.0, 28.0));

    for display_file in 0..8 {
        // Files are letters a-h. Flipping reverses which file appears on the left.
        let file = if flipped {
            File::new((7 - display_file) as u32)
        } else {
            File::new(display_file as u32)
        };
        let text_pos = pos2(
            rect.left() + display_file as f32 * rect.width() / 8.0 + 7.0,
            rect.bottom() - font.size - 5.0,
        );
        paint_coordinate_label(painter, text_pos, file.char(), font.clone());
    }

    for display_rank in 0..8 {
        // Ranks are numbers 1-8. In normal orientation, rank 8 is at the top.
        let rank = if flipped {
            Rank::new(display_rank as u32)
        } else {
            Rank::new((7 - display_rank) as u32)
        };
        let text_pos = pos2(
            rect.left() + 7.0,
            rect.top() + display_rank as f32 * rect.height() / 8.0 + 5.0,
        );
        paint_coordinate_label(painter, text_pos, rank.char(), font.clone());
    }
}

fn paint_coordinate_label(painter: &egui::Painter, pos: Pos2, text: char, font: FontId) {
    // Draw a light outline first, then dark text on top. This makes coordinates
    // readable on both light and dark square themes.
    for offset in [
        vec2(-1.5, 0.0),
        vec2(1.5, 0.0),
        vec2(0.0, -1.5),
        vec2(0.0, 1.5),
    ] {
        painter.text(
            pos + offset,
            Align2::LEFT_TOP,
            text,
            font.clone(),
            Color32::from_rgba_unmultiplied(255, 255, 255, 230),
        );
    }

    painter.text(
        pos,
        Align2::LEFT_TOP,
        text,
        font,
        Color32::from_rgba_unmultiplied(0, 0, 0, 230),
    );
}

fn paint_checkmate_overlay(ui: &mut Ui, rect: Rect, view: &BoardView<'_>) {
    let Some(GameOutcome::Checkmate { winner }) = view.game.outcome() else {
        return;
    };

    // The overlay is board-sized, so it stays centered over the chess position.
    let winner = match winner {
        shakmaty::Color::White => "White",
        shakmaty::Color::Black => "Black",
    };
    let text = format!("Checkmate - {winner} wins");
    let font_size = (rect.width() * 0.09).clamp(28.0, 64.0);
    let painter = ui.painter();
    let center = rect.center();

    painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 120));
    painter.text(
        center + vec2(3.0, 3.0),
        Align2::CENTER_CENTER,
        &text,
        FontId::proportional(font_size),
        Color32::from_rgba_unmultiplied(0, 0, 0, 220),
    );
    painter.text(
        center,
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(font_size),
        Color32::WHITE,
    );
}

fn square_at_pointer(rect: Rect, pointer: Pos2, flipped: bool) -> Option<Square> {
    if !rect.contains(pointer) {
        return None;
    }

    // `egui` uses a top-left origin, so display rank increases downward and
    // must be inverted to recover chess ranks when the board is not flipped.
    let square_size = rect.width() / 8.0;
    let display_file = ((pointer.x - rect.left()) / square_size).floor() as usize;
    let display_rank = ((pointer.y - rect.top()) / square_size).floor() as usize;

    if display_file > 7 || display_rank > 7 {
        return None;
    }

    Some(displayed_square(display_file, display_rank, flipped))
}

fn displayed_square(display_file: usize, display_rank: usize, flipped: bool) -> Square {
    // Convert from UI grid coordinates to chess coordinates. UI ranks go down
    // the screen, while chess ranks normally increase upward from White's side.
    let board_file = if flipped {
        7 - display_file
    } else {
        display_file
    };
    let board_rank = if flipped {
        display_rank
    } else {
        7 - display_rank
    };
    square_from_indices(board_file, board_rank)
}

fn display_coords(square: Square, flipped: bool) -> Option<(usize, usize)> {
    // Convert from a real chess square to where it should appear on screen.
    let (file, rank) = square.coords();
    let file_index = file.to_usize();
    let rank_index = rank.to_usize();

    let display_file = if flipped {
        7usize.checked_sub(file_index)?
    } else {
        file_index
    };
    let display_rank = if flipped {
        rank_index
    } else {
        7usize.checked_sub(rank_index)?
    };
    Some((display_file, display_rank))
}

fn square_rect(board_rect: Rect, display_file: usize, display_rank: usize) -> Rect {
    // Return the rectangle for one square inside the full board rectangle.
    let square_size = board_rect.width() / 8.0;
    Rect::from_min_size(
        pos2(
            board_rect.left() + display_file as f32 * square_size,
            board_rect.top() + display_rank as f32 * square_size,
        ),
        vec2(square_size, square_size),
    )
}
