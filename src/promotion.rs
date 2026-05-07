//! Pawn promotion modal UI.
//!
//! This module displays a blocking promotion picker when a pawn move reaches
//! the final rank. The UI is kept separate from the game model so promotion
//! remains a view concern until the user picks a role.

use egui::{Align2, Context, Window};
use shakmaty::Role;

/// Shows the promotion dialog and returns the chosen role for this frame.
///
/// Returns `Some(role)` only when the user clicks one of the promotion
/// buttons. Otherwise returns `None` and keeps the dialog visible.
pub fn show_promotion_modal(ctx: &Context) -> Option<Role> {
    // The dialog is rebuilt every frame. `chosen_role` starts empty each frame
    // and becomes Some(role) only during the frame where a button is clicked.
    let mut chosen_role = None;

    Window::new("Choose promotion")
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .show(ctx, |ui| {
            ui.label("Promote pawn to:");
            ui.horizontal(|ui| {
                // A small array keeps the four buttons consistent without
                // repeating the same button code four times.
                for (label, role) in [
                    ("Queen", Role::Queen),
                    ("Rook", Role::Rook),
                    ("Bishop", Role::Bishop),
                    ("Knight", Role::Knight),
                ] {
                    if ui.button(label).clicked() {
                        chosen_role = Some(role);
                    }
                }
            });
        });

    chosen_role
}
