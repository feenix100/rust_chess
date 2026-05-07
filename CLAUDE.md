# CLAUDE.md

Guidance for Claude Code (and other AI coding assistants) working in this
repository. Read this before making changes.

---

## Project overview

A native desktop chess application written in Rust. Two humans share the
keyboard and mouse; the app enforces all standard chess rules. Built on
`eframe` + `egui` for the GUI, `shakmaty` for chess rules, and
`usvg` + `resvg` + `tiny-skia` for rendering embedded SVG assets.

This is a player-vs-player baseline. There is no AI opponent, no
networking, no clocks. Do not add those features unless explicitly
asked.

---

## Common commands

```bash
# Build (debug)
cargo build

# Build (release — what users run)
cargo run --release

# Run all tests
cargo test

# Format
cargo fmt

# Lint (treat warnings as actionable)
cargo clippy --all-targets -- -D warnings

# Check without building binaries (fast)
cargo check
```

Always run `cargo fmt` and `cargo clippy` before declaring work
complete. Both must pass clean.

---

## Architecture

Single-binary, single-thread, immediate-mode GUI. Six source files,
each with a single responsibility:

```
src/
├── main.rs        eframe entry point and window setup
├── app.rs         top-level UI loop, sidebar, control flow
├── board.rs       board widget — rendering + click hit-testing
├── game.rs        shakmaty wrapper, move history, SAN, draw tracking
├── assets.rs      embedded SVG bytes + rasterized texture cache
└── promotion.rs   pawn promotion picker modal
```

### Module responsibilities (do not blur these lines)

- **`game.rs`** — pure chess state. Owns the `shakmaty::Chess`
  position, history with snapshots for undo, and a
  `HashMap<Chess, u32>` for threefold-repetition tracking. **It must
  not know anything about rendering, input, or `egui`.** Keep it
  reusable as a library.
- **`board.rs`** — board widget. Rasterizes squares and pieces via
  `AssetStore`, paints selection / last-move / legal-move overlays,
  and converts pointer clicks to `shakmaty::Square` values. Reads
  `Game` state but never mutates it.
- **`assets.rs`** — SVG embedding and texture cache. SVGs are
  `include_bytes!`'d at compile time, parsed with `usvg`, rasterized
  with `resvg` + `tiny-skia`, and uploaded to `egui` as textures.
  Cached by `(AssetKey, size_px)` so window resizes only re-rasterize
  when the square size actually changes.
- **`promotion.rs`** — modal that returns `Some(Role)` only on the
  frame the user clicks a button. Stateless from the model's
  perspective.
- **`app.rs`** — the only place that mutates `Game`, owns selection
  state and theme, and runs the egui update loop. All cross-module
  control flow lives here.
- **`main.rs`** — `eframe::run_native` and `ViewportBuilder` config.
  Nothing else belongs here.

### Data flow per frame

```
egui frame
   ├─ app.rs reads Game state
   ├─ app.rs paints sidebar (status, move list, controls)
   ├─ board.rs paints squares → highlights → pieces → coordinates
   ├─ board.rs returns clicked_square (Option<Square>)
   ├─ app.rs updates selection / plays move / opens promotion modal
   └─ if promotion modal is up, promotion.rs returns Option<Role>
```

The UI thread does all of this synchronously. There is no async, no
threading, no channels. Keep it that way.

---

## Tech stack and versions

| Concern             | Crate                |
| ------------------- | -------------------- |
| GUI                 | `eframe` + `egui`    |
| Chess rules         | `shakmaty`           |
| SVG parsing         | `usvg`               |
| SVG rasterization   | `resvg` + `tiny-skia`|
| Error handling      | `anyhow`             |

See `Cargo.toml` for pinned versions. **When upgrading any of these
crates, check the changelog for breaking API changes** — `egui` and
`shakmaty` both change shapes between minor versions.

---

## Code style and conventions

### Rust style

- **No `unwrap()` or `expect()` on user-facing paths.** Use
  `Result` / `anyhow::Result` and surface errors via the UI. Panics
  are reserved for impossible invariants, not for I/O or user input.
- **Prefer `?` propagation** over nested `match` arms.
- **No commented-out code, no stray `TODO` comments, no `dbg!`** in
  committed work. If something is incomplete, leave it out entirely.
- **Idiomatic borrowing.** Functions take `&Game` unless they need to
  mutate; helpers in `board.rs` borrow assets and game state rather
  than cloning.
- **Naming.** Types and functions describe intent. Use full words
  (`square_from_indices`, not `sq_idx`). Acceptable abbreviations:
  `fen`, `san`, `uci`.

### Comments

This repo's comments lean heavier than typical Rust. Match the
existing style when adding code:

- **Module-level `//!` docs** at the top of every file describing
  responsibility, public API surface, and dependencies on other
  modules.
- **Doc comments `///`** on every public item — purpose, parameters,
  return value, panics, assumptions.
- **Inline `//` comments** explaining the *reasoning* behind
  non-obvious logic. Particularly:
  - Chess rule edge cases (en passant target, castling rights,
    promotion encoding)
  - Library quirks (egui's top-left coordinate origin, SVG viewBox
    handling)
  - Conversions between display coordinates and board coordinates in
    `board.rs`
- **Don't comment the obvious.** `// increment i` is noise.

### Section dividers

In files over ~150 lines, use banner comments:

```rust
// ============================================================
// Section name
// ============================================================
```

---

## Important domain details

### Coordinate systems

There are **two** coordinate systems in `board.rs` and they are easy
to confuse:

- **Board coordinates**: `shakmaty::Square` — file `a` to `h` (left
  to right from White's perspective), rank `1` to `8` (bottom to top
  from White's perspective).
- **Display coordinates**: `(display_file, display_rank)`, both
  `0..8`, with `(0, 0)` at the **top-left** of the screen because
  egui uses a top-left origin.

When the board is flipped, display ↔ board mapping inverts. The
helpers `displayed_square()` and `display_coords()` in `board.rs`
handle this. **Always go through them** rather than recomputing
indices inline.

### Move types

`shakmaty::Move` is an enum. The variants relevant here:

- `Move::Normal { from, to, promotion, .. }` — most moves
- `Move::EnPassant { from, to, .. }` — en passant captures
- `Move::Castle { king, rook }` — both castling directions

**Castling is the trap**: shakmaty stores the *rook's* square as the
target. The UI shows the king's destination instead. The
`move_to()` helper in `game.rs` handles this — use it.

### Threefold repetition

`shakmaty::Position::outcome()` does **not** detect threefold
repetition or the fifty-move rule on its own. We track repetitions
manually in `Game::repetition_counts: HashMap<Chess, u32>`,
incrementing on `play()` and decrementing on `undo()`. If you change
how moves are recorded, keep this map in sync.

### Promotion

When a pawn move reaches the last rank, there can be **four** legal
moves between the same two squares (one per promotion piece). The
helper `Game::legal_moves_between(from, to)` returns all of them,
and `app.rs` opens the promotion modal to disambiguate. Do not
collapse this to a single move.

---

## Assets

All SVGs are embedded via `include_bytes!` in `assets.rs`. The binary
ships standalone — there is no asset directory at runtime.

### Adding a new piece set or theme

1. Drop the SVGs into `assets/pieces/` or `assets/squares/`.
2. Add a variant to `BoardTheme` (for squares) or extend the
   `svg_bytes()` match (for pieces).
3. Wire the new theme into the sidebar toggle in `app.rs`.

Keep the existing filename pattern (`{color}_{piece}.svg` for
pieces). The original files used a `_svg_NoShadow.svg` suffix; current
code expects clean names — preserve whichever convention is in
`svg_bytes()` today.

---

## Testing approach

This repo currently has limited automated tests. When adding features
that touch chess logic in `game.rs`, prefer adding unit tests there
over manual UI testing — the rules code is pure and easy to test.

For UI changes in `board.rs` or `app.rs`, manual smoke testing is
expected:

- Standard opening moves work
- Castling (both sides, both colors)
- En passant (`1.e4 a6 2.e5 d5 3.exd6` is a clean repro)
- Promotion modal opens, all four pieces work
- Scholar's Mate (`1.e4 e5 2.Bc4 Bc5 3.Qh5 Nc6 4.Qxf7#`) shows
  checkmate correctly
- Undo restores prior state exactly, including repetition counts
- Flip Board does not desync clicks from rendered squares

---

## Out of scope (do not add unprompted)

The scope is intentionally small. **Do not add** the following
without an explicit request:

- AI opponent or UCI engine integration
- Online play, multiplayer, or networking
- Time controls / clocks
- Opening book or analysis tools
- PGN import/export beyond the existing CSV move list
- Sound effects
- Drag-and-drop input (click-to-move is the intended interaction)
- Settings persistence to disk

If a feature seems useful but isn't on the requirements list, propose
it in a comment or PR description rather than adding it silently.

---

## Pull request checklist

Before submitting:

- [ ] `cargo fmt` clean
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo test` passes
- [ ] `cargo build --release` succeeds
- [ ] No new `unwrap()`, `expect()`, `dbg!`, `println!`, or `TODO` in
      committed code
- [ ] New public items have `///` doc comments
- [ ] If chess logic changed: manual smoke tests above pass
- [ ] If `Cargo.toml` changed: explain why in the PR description
