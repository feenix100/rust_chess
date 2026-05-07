//! Top-level application state and UI flow.
//!
//! This module owns the `ChessApp` struct, coordinates the board widget,
//! sidebar controls, promotion flow, and recoverable UI errors, and translates
//! board clicks into legal chess moves using [`crate::game::Game`].

use std::{fs, time::Instant};

use egui::{
    vec2, Align2, Area, Button, CentralPanel, Color32, Context, Frame, Id, Key, Margin, Order,
    Panel, RichText, ScrollArea, Stroke, Window,
};
use shakmaty::{Color, Move, Role, Square};

use crate::assets::{AssetStore, BoardTheme};
use crate::board::{show_board, BoardView};
use crate::game::{status_text, Game};
use crate::promotion::show_promotion_modal;

/// The native desktop chess application.
pub struct ChessApp {
    /// Owns and caches SVG textures so the board does not rasterize them every frame.
    assets: AssetStore,
    /// The rules model: current position, legal move generation, and move history.
    game: Game,
    /// The square the user clicked first while preparing a move.
    selected_square: Option<Square>,
    /// Legal target squares for the selected piece. The board draws these as markers.
    legal_destinations: Vec<Square>,
    /// If true, the board is drawn from Black's point of view.
    flipped: bool,
    /// User preference for drawing the origin/destination of the previous move.
    show_last_move: bool,
    /// Current visual style for board squares.
    board_theme: BoardTheme,
    /// If true, show the keyboard move entry controls.
    keyboard_moves_enabled: bool,
    /// Square currently targeted by arrow-key movement.
    keyboard_cursor: Square,
    /// If false, the clock UI is shown but no time is counted down.
    timed_game_enabled: bool,
    /// Preset selected by the clock buttons.
    timer_preset: TimerPreset,
    /// User-entered clock length used when `timer_preset` is `Custom`.
    custom_timer_minutes: u32,
    /// Remaining time for White, stored as seconds because frame updates are fractional.
    white_remaining_seconds: f32,
    /// Remaining time for Black, stored as seconds because frame updates are fractional.
    black_remaining_seconds: f32,
    /// The last time we subtracted from the active player's clock.
    last_timer_tick: Option<Instant>,
    /// The player who ran out of time, if the timed game has ended that way.
    timed_out: Option<Color>,
    /// True after a move is played and false after the single allowed undo is used.
    undo_available: bool,
    /// Promotion moves are ambiguous until the user chooses queen, rook, bishop, or knight.
    pending_promotion: Option<PendingPromotion>,
    /// If true, show the confirmation dialog before resetting the current game.
    show_new_game_confirmation: bool,
    /// Recoverable error text shown in the sidebar.
    last_error: Option<String>,
    /// Success/info text shown in the sidebar.
    last_message: Option<String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TimerPreset {
    OneMinute,
    ThreeMinutes,
    FiveMinutes,
    Custom,
}

#[derive(Clone)]
struct PendingPromotion {
    /// Legal promotion moves from the same source square to the same target square.
    candidates: Vec<Move>,
}

impl ChessApp {
    /// Creates a new application with the standard starting position.
    pub fn new(_egui_ctx: Context) -> Self {
        Self {
            assets: AssetStore::new(),
            game: Game::new(),
            selected_square: None,
            legal_destinations: Vec::new(),
            flipped: false,
            show_last_move: true,
            board_theme: BoardTheme::Purple,
            keyboard_moves_enabled: false,
            keyboard_cursor: Square::E2,
            timed_game_enabled: false,
            timer_preset: TimerPreset::FiveMinutes,
            custom_timer_minutes: 5,
            white_remaining_seconds: 5.0 * 60.0,
            black_remaining_seconds: 5.0 * 60.0,
            last_timer_tick: None,
            timed_out: None,
            undo_available: false,
            pending_promotion: None,
            show_new_game_confirmation: false,
            last_error: None,
            last_message: None,
        }
    }

    fn reset_game(&mut self) {
        // `reset` replaces the chess model with a fresh starting position.
        // UI-only state such as selections, promotion dialogs, and messages must
        // be cleared separately because it lives on `ChessApp`, not `Game`.
        self.game.reset();
        self.clear_selection();
        self.pending_promotion = None;
        self.keyboard_cursor = Square::E2;
        self.undo_available = false;
        self.reset_clocks();
        self.last_error = None;
        self.last_message = None;
    }

    fn reset_clocks(&mut self) {
        let seconds = self.selected_timer_seconds();
        self.white_remaining_seconds = seconds;
        self.black_remaining_seconds = seconds;
        self.last_timer_tick = None;
        self.timed_out = None;
    }

    fn selected_timer_seconds(&self) -> f32 {
        // Convert the user's selected minute value into seconds once. The clock
        // stores seconds so it can subtract fractions of a second each frame.
        (match self.timer_preset {
            TimerPreset::OneMinute => 1,
            TimerPreset::ThreeMinutes => 3,
            TimerPreset::FiveMinutes => 5,
            TimerPreset::Custom => self.custom_timer_minutes.max(1),
        } as f32)
            * 60.0
    }

    fn game_started(&self) -> bool {
        // This app has no separate "Start" button. The first completed move is
        // treated as the start of the game.
        self.game.can_undo()
    }

    fn update_timer(&mut self, ctx: &Context) {
        // Timers should only run while a timed game is active, started, and not
        // already over. Returning early also prevents stale time deltas from
        // accumulating while the clock is paused.
        if !self.timed_game_enabled
            || !self.game_started()
            || self.game.outcome().is_some()
            || self.timed_out.is_some()
        {
            self.last_timer_tick = None;
            return;
        }

        let now = Instant::now();
        // `replace` stores `now` and gives us the previous tick. On the first
        // timed frame there is no previous tick yet, so we request another
        // repaint and wait until the next frame to subtract time.
        let Some(last_tick) = self.last_timer_tick.replace(now) else {
            ctx.request_repaint();
            return;
        };
        let elapsed = (now - last_tick).as_secs_f32();
        // Only the side to move loses time, like a normal chess clock.
        let remaining = match self.game.side_to_move() {
            Color::White => &mut self.white_remaining_seconds,
            Color::Black => &mut self.black_remaining_seconds,
        };

        *remaining = (*remaining - elapsed).max(0.0);
        if *remaining <= 0.0 {
            self.timed_out = Some(self.game.side_to_move());
            self.clear_selection();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn clear_selection(&mut self) {
        // Clearing both values keeps the UI consistent: no selected square means
        // there should also be no legal-move markers on the board.
        self.selected_square = None;
        self.legal_destinations.clear();
    }

    fn select_square(&mut self, square: Square) {
        // Store the selected square and ask the game model which legal moves
        // start there. The board only needs destination squares for drawing.
        self.selected_square = Some(square);
        self.legal_destinations = self
            .game
            .legal_moves_from(square)
            .into_iter()
            .filter_map(|mv| move_destination(&mv))
            .collect();
    }

    fn handle_square_click(&mut self, square: Square) {
        // Ignore board input while a modal choice is open or after the game has
        // ended. This keeps late clicks from changing finished positions.
        if self.pending_promotion.is_some()
            || self.game.outcome().is_some()
            || self.timed_out.is_some()
        {
            return;
        }

        let clicked_piece = self.game.piece_at(square);
        let selected_piece = self
            .selected_square
            .and_then(|selected| self.game.piece_at(selected));

        // Clicking the selected square again behaves like "cancel selection".
        if Some(square) == self.selected_square {
            self.clear_selection();
            return;
        }

        // Clicking one of your own pieces selects it instead of trying to move
        // the old selection onto an occupied friendly square.
        if let Some(piece) = clicked_piece {
            if piece.color == self.game.side_to_move() {
                self.select_square(square);
                return;
            }
        }

        let Some(from) = self.selected_square else {
            // No piece has been selected yet, and the clicked square was not a
            // selectable friendly piece.
            return;
        };

        if selected_piece.is_none() {
            // Defensive cleanup. This can happen if UI state and board state get
            // out of sync after undo/reset.
            self.clear_selection();
            return;
        }

        let candidates = self.game.legal_moves_between(from, square);
        if candidates.is_empty() {
            return;
        }

        // Promotion creates several legal moves with the same start and end
        // squares. The user must choose which piece the pawn becomes.
        if candidates.len() > 1 {
            self.pending_promotion = Some(PendingPromotion { candidates });
            return;
        }

        if let Some(mv) = candidates.into_iter().next() {
            self.play_move(mv);
        }
    }

    fn handle_keyboard_input(&mut self, ctx: &Context) {
        if !self.keyboard_moves_enabled {
            return;
        }

        let (left, right, up, down, select, cancel) = ctx.input(|input| {
            (
                input.key_pressed(Key::ArrowLeft),
                input.key_pressed(Key::ArrowRight),
                input.key_pressed(Key::ArrowUp),
                input.key_pressed(Key::ArrowDown),
                input.key_pressed(Key::Enter) || input.key_pressed(Key::Space),
                input.key_pressed(Key::Escape),
            )
        });

        if left {
            self.move_keyboard_cursor(-1, 0);
        }
        if right {
            self.move_keyboard_cursor(1, 0);
        }
        if up {
            self.move_keyboard_cursor(0, -1);
        }
        if down {
            self.move_keyboard_cursor(0, 1);
        }
        if cancel {
            self.clear_selection();
        }
        if select {
            self.handle_square_click(self.keyboard_cursor);
        }
    }

    fn move_keyboard_cursor(&mut self, display_file_delta: isize, display_rank_delta: isize) {
        let Some((display_file, display_rank)) = display_coords(self.keyboard_cursor, self.flipped)
        else {
            return;
        };

        let next_display_file = (display_file as isize + display_file_delta).clamp(0, 7) as usize;
        let next_display_rank = (display_rank as isize + display_rank_delta).clamp(0, 7) as usize;
        self.keyboard_cursor = displayed_square(next_display_file, next_display_rank, self.flipped);
    }

    fn handle_promotion_choice(&mut self, role: Role) {
        let Some(pending) = self.pending_promotion.clone() else {
            return;
        };

        // Pick the one pending promotion move whose promotion piece matches the
        // button the user clicked.
        if let Some(mv) = pending
            .candidates
            .into_iter()
            .find(|candidate| promotion_role(candidate) == Some(role))
        {
            self.pending_promotion = None;
            self.play_move(mv);
        }
    }

    fn play_move(&mut self, mv: Move) {
        // The game model validates the move. The UI only reacts to success or
        // stores an error message for the sidebar.
        match self.game.play(mv) {
            Ok(()) => {
                self.clear_selection();
                self.last_timer_tick = Some(Instant::now());
                self.undo_available = true;
                self.last_error = None;
                self.last_message = None;
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                self.last_message = None;
            }
        }
    }

    fn undo(&mut self) {
        if self.game.undo().is_some() {
            self.pending_promotion = None;
            self.clear_selection();
            self.undo_available = false;
            if self.game_started() {
                // Restart the clock delta from "now" after undo so a long pause
                // on the undo click does not charge time to the next player.
                self.last_timer_tick = Some(Instant::now());
                self.timed_out = None;
            } else {
                self.reset_clocks();
            }
            self.last_error = None;
            self.last_message = None;
        }
    }

    fn export_move_list_csv(&mut self) {
        // This writes beside the executable/current working directory. Keeping a
        // fixed filename avoids needing a native file-picker dependency.
        match fs::write(MOVE_LIST_EXPORT_PATH, self.game.move_list_csv()) {
            Ok(()) => {
                self.last_error = None;
                self.last_message = Some(format!("Exported {MOVE_LIST_EXPORT_PATH}"));
            }
            Err(error) => {
                self.last_message = None;
                self.last_error = Some(format!("Could not export CSV: {error}"));
            }
        }
    }
}

impl eframe::App for ChessApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // egui redraws the whole interface each frame. The app state above is
        // what makes the frame remember selections, moves, clocks, and settings.
        self.update_timer(ui.ctx());
        self.handle_keyboard_input(ui.ctx());

        // The right panel owns controls and move history. It is wrapped in a
        // ScrollArea so small windows can still reach every button.
        Panel::right("sidebar")
            .min_size(340.0)
            .resizable(false)
            .show_inside(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(10.0, 12.0);
                        ui.spacing_mut().button_padding = vec2(14.0, 10.0);

                        ui.label(RichText::new("Game").heading().size(28.0));
                        ui.label(RichText::new(status_text(&self.game)).strong().size(22.0));
                        if let Some(color) = self.timed_out {
                            ui.colored_label(
                                Color32::from_rgb(180, 40, 40),
                                RichText::new(format!("{} ran out of time", color_name(color)))
                                    .size(20.0),
                            );
                        }

                        ui.separator();
                        ui.label(RichText::new("Clock").strong().size(22.0));
                        if self.timed_game_enabled {
                            ui.label(
                                RichText::new(format!(
                                    "White {}   Black {}",
                                    format_clock(self.white_remaining_seconds),
                                    format_clock(self.black_remaining_seconds)
                                ))
                                .monospace()
                                .size(21.0),
                            );
                        } else {
                            ui.label(RichText::new("Untimed game").strong().size(21.0));
                        }
                        show_timer_controls(ui, self);

                        // Errors and success messages share the same sidebar
                        // location so the user gets immediate feedback after an
                        // invalid action or CSV export.
                        if let Some(error) = &self.last_error {
                            ui.colored_label(
                                Color32::from_rgb(180, 40, 40),
                                RichText::new(error).size(19.0),
                            );
                        }
                        if let Some(message) = &self.last_message {
                            ui.colored_label(
                                Color32::from_rgb(35, 120, 65),
                                RichText::new(message).size(19.0),
                            );
                        }

                        ui.separator();
                        ui.label(
                            RichText::new(format!("Moves ({})", self.game.san_history().len()))
                                .heading()
                                .size(26.0),
                        );
                        // This nested scroll area keeps a long move list from
                        // pushing the main controls out of reach.
                        ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for line in self.game.formatted_move_list() {
                                ui.label(RichText::new(line).monospace().size(20.0));
                            }
                        });

                        ui.separator();
                        if new_game_button(ui, self.is_game_over()).clicked() {
                            self.show_new_game_confirmation = true;
                        }
                        if sidebar_button(ui, "Flip Board").clicked() {
                            self.flipped = !self.flipped;
                        }
                        let previous_move_label = if self.show_last_move {
                            "Hide Previous Move"
                        } else {
                            "Show Previous Move"
                        };
                        if sidebar_button(ui, previous_move_label).clicked() {
                            self.show_last_move = !self.show_last_move;
                        }
                        if ui
                            .add_sized(
                                [SIDEBAR_BUTTON_WIDTH, SIDEBAR_BUTTON_HEIGHT],
                                Button::selectable(
                                    self.keyboard_moves_enabled,
                                    RichText::new(if self.keyboard_moves_enabled {
                                        "Keyboard Moves: On"
                                    } else {
                                        "Keyboard Moves: Off"
                                    })
                                    .size(20.0),
                                ),
                            )
                            .clicked()
                        {
                            self.keyboard_moves_enabled = !self.keyboard_moves_enabled;
                        }
                        if ui
                            .add_enabled(
                                self.undo_available && self.game.can_undo(),
                                Button::new(RichText::new("Undo").size(20.0))
                                    .min_size(vec2(SIDEBAR_BUTTON_WIDTH, SIDEBAR_BUTTON_HEIGHT)),
                            )
                            .clicked()
                        {
                            self.undo();
                        }
                        if ui
                            .add_enabled(
                                self.game.can_undo(),
                                Button::new(RichText::new("Export CSV").size(20.0))
                                    .min_size(vec2(SIDEBAR_BUTTON_WIDTH, SIDEBAR_BUTTON_HEIGHT)),
                            )
                            .clicked()
                        {
                            self.export_move_list_csv();
                        }

                        ui.separator();
                        ui.label(RichText::new("Theme").strong().size(22.0));
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add_sized(
                                    [92.0, SIDEBAR_BUTTON_HEIGHT],
                                    Button::selectable(
                                        self.board_theme == BoardTheme::Brown,
                                        RichText::new("Brown").size(20.0),
                                    ),
                                )
                                .clicked()
                            {
                                self.board_theme = BoardTheme::Brown;
                            }
                            if ui
                                .add_sized(
                                    [92.0, SIDEBAR_BUTTON_HEIGHT],
                                    Button::selectable(
                                        self.board_theme == BoardTheme::Gray,
                                        RichText::new("Gray").size(20.0),
                                    ),
                                )
                                .clicked()
                            {
                                self.board_theme = BoardTheme::Gray;
                            }
                            if ui
                                .add_sized(
                                    [104.0, SIDEBAR_BUTTON_HEIGHT],
                                    Button::selectable(
                                        self.board_theme == BoardTheme::Purple,
                                        RichText::new("Purple").size(20.0),
                                    ),
                                )
                                .clicked()
                            {
                                self.board_theme = BoardTheme::Purple;
                            }
                            if ui
                                .add_sized(
                                    [118.0, SIDEBAR_BUTTON_HEIGHT],
                                    Button::selectable(
                                        self.board_theme == BoardTheme::Tropical,
                                        RichText::new("Tropical").size(20.0),
                                    ),
                                )
                                .clicked()
                            {
                                self.board_theme = BoardTheme::Tropical;
                            }
                        });
                    });
            });

        CentralPanel::default().show_inside(ui, |ui| {
            ui.vertical_centered(|ui| {
                // `show_board` paints the board and returns an optional clicked
                // square. The app then decides what that click means.
                let board_result = show_board(
                    ui,
                    BoardView {
                        game: &self.game,
                        assets: &mut self.assets,
                        flipped: self.flipped,
                        show_last_move: self.show_last_move,
                        theme: self.board_theme,
                        selected: self.selected_square,
                        legal_destinations: &self.legal_destinations,
                        keyboard_cursor: self
                            .keyboard_moves_enabled
                            .then_some(self.keyboard_cursor),
                    },
                );

                match board_result {
                    Ok(board_response) => {
                        if let Some(square) = board_response.clicked_square {
                            self.handle_square_click(square);
                        }
                    }
                    Err(error) => {
                        self.last_error = Some(error.to_string());
                    }
                }
            });
        });

        if let Some(choice) = show_promotion_if_needed(self.pending_promotion.is_some(), ui.ctx()) {
            self.handle_promotion_choice(choice);
        }

        if show_new_game_confirmation(ui.ctx(), &mut self.show_new_game_confirmation) {
            self.reset_game();
        }

        // This is drawn last so it appears above the board and sidebar.
        show_timeout_overlay(ui.ctx(), self.timed_out);
    }
}

impl ChessApp {
    fn is_game_over(&self) -> bool {
        self.game.outcome().is_some() || self.timed_out.is_some()
    }
}

const SIDEBAR_BUTTON_WIDTH: f32 = 280.0;
const SIDEBAR_BUTTON_HEIGHT: f32 = 48.0;
const MOVE_LIST_EXPORT_PATH: &str = "move_list.csv";

fn sidebar_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add_sized(
        [SIDEBAR_BUTTON_WIDTH, SIDEBAR_BUTTON_HEIGHT],
        Button::new(RichText::new(text).size(20.0)),
    )
}

fn new_game_button(ui: &mut egui::Ui, is_game_over: bool) -> egui::Response {
    let mut button = Button::new(RichText::new("New Game").size(20.0));

    // When the game is over, the restart action becomes the most important
    // action, so the button gets a stronger color treatment.
    if is_game_over {
        button = button
            .fill(Color32::from_rgb(58, 150, 165))
            .stroke(Stroke::new(3.0, Color32::from_rgb(18, 76, 88)));
    }

    ui.add_sized([SIDEBAR_BUTTON_WIDTH, SIDEBAR_BUTTON_HEIGHT], button)
}

fn show_new_game_confirmation(ctx: &Context, is_open: &mut bool) -> bool {
    if !*is_open {
        return false;
    }

    let mut open = *is_open;
    let mut confirmed = false;
    let mut should_close = false;

    Window::new("Start new game?")
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(RichText::new("Start a new game and clear the current board?").size(18.0));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .add_sized(
                        [132.0, 40.0],
                        Button::new(RichText::new("Start New Game").size(18.0)),
                    )
                    .clicked()
                {
                    confirmed = true;
                    should_close = true;
                }

                if ui
                    .add_sized(
                        [92.0, 40.0],
                        Button::new(RichText::new("Cancel").size(18.0)),
                    )
                    .clicked()
                {
                    should_close = true;
                }
            });
        });

    if should_close {
        open = false;
    }
    *is_open = open;

    confirmed
}

fn show_timer_controls(ui: &mut egui::Ui, app: &mut ChessApp) {
    let enabled = !app.game_started();

    // Timer settings lock after the first move. That prevents changing clock
    // rules in the middle of a game.
    ui.add_enabled_ui(enabled, |ui| {
        if ui
            .add_sized(
                [SIDEBAR_BUTTON_WIDTH, SIDEBAR_BUTTON_HEIGHT],
                Button::selectable(
                    app.timed_game_enabled,
                    RichText::new(if app.timed_game_enabled {
                        "Timed Game: On"
                    } else {
                        "Timed Game: Off"
                    })
                    .size(20.0),
                ),
            )
            .clicked()
        {
            app.timed_game_enabled = !app.timed_game_enabled;
            app.reset_clocks();
        }
    });

    ui.add_enabled_ui(enabled && app.timed_game_enabled, |ui| {
        ui.horizontal(|ui| {
            if timer_option_button(ui, app.timer_preset == TimerPreset::OneMinute, "1 min")
                .clicked()
            {
                app.timer_preset = TimerPreset::OneMinute;
                app.reset_clocks();
            }
            if timer_option_button(ui, app.timer_preset == TimerPreset::ThreeMinutes, "3 min")
                .clicked()
            {
                app.timer_preset = TimerPreset::ThreeMinutes;
                app.reset_clocks();
            }
            if timer_option_button(ui, app.timer_preset == TimerPreset::FiveMinutes, "5 min")
                .clicked()
            {
                app.timer_preset = TimerPreset::FiveMinutes;
                app.reset_clocks();
            }
        });

        ui.horizontal(|ui| {
            if timer_option_button(ui, app.timer_preset == TimerPreset::Custom, "Custom").clicked()
            {
                app.timer_preset = TimerPreset::Custom;
                app.reset_clocks();
            }

            let response = ui.add_sized(
                [92.0, 40.0],
                egui::DragValue::new(&mut app.custom_timer_minutes)
                    .range(1..=999)
                    .speed(1),
            );
            ui.label(RichText::new("min").size(19.0));

            if response.changed() {
                // Editing the custom value automatically selects custom mode.
                app.timer_preset = TimerPreset::Custom;
                app.reset_clocks();
            }
        });
    });
}

fn timer_option_button(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
    ui.add_sized(
        [84.0, 40.0],
        Button::selectable(selected, RichText::new(text).size(18.0)),
    )
}

fn format_clock(seconds: f32) -> String {
    let total_seconds = seconds.ceil().max(0.0) as u32;
    format!("{:02}:{:02}", total_seconds / 60, total_seconds % 60)
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

fn show_timeout_overlay(ctx: &Context, timed_out: Option<Color>) {
    let Some(loser) = timed_out else {
        return;
    };

    // If one player timed out, the other player wins.
    let winner = opposite_color(loser);
    Area::new(Id::new("timeout_overlay"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            Frame::new()
                .fill(Color32::from_rgba_unmultiplied(20, 20, 20, 235))
                .stroke(Stroke::new(3.0, Color32::WHITE))
                .corner_radius(8.0)
                .inner_margin(Margin::same(28))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(format!("{} wins on time", color_name(winner)))
                                .heading()
                                .strong()
                                .size(44.0)
                                .color(Color32::WHITE),
                        );
                        ui.label(
                            RichText::new(format!("{} ran out of time", color_name(loser)))
                                .size(24.0)
                                .color(Color32::from_rgb(230, 230, 230)),
                        );
                    });
                });
        });
}

fn show_promotion_if_needed(is_open: bool, ctx: &Context) -> Option<Role> {
    if is_open {
        show_promotion_modal(ctx)
    } else {
        None
    }
}

fn move_destination(mv: &Move) -> Option<Square> {
    match mv {
        Move::Normal { to, .. } => Some(*to),
        Move::EnPassant { to, .. } => Some(*to),
        // shakmaty represents castling as king-to-rook. The UI should feel like
        // normal chess, so we convert it to the king's final square: g-file or c-file.
        Move::Castle { king, .. } => mv
            .castling_side()
            .map(|side| Square::from_coords(side.king_to_file(), king.rank())),
        _ => None,
    }
}

fn promotion_role(mv: &Move) -> Option<Role> {
    match mv {
        Move::Normal { promotion, .. } => *promotion,
        _ => None,
    }
}

fn displayed_square(display_file: usize, display_rank: usize, flipped: bool) -> Square {
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
    crate::game::square_from_indices(board_file, board_rank)
}

fn display_coords(square: Square, flipped: bool) -> Option<(usize, usize)> {
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
