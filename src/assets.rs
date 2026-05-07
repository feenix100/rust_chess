//! Embedded SVG asset loading and texture caching.
//!
//! This module owns the chess piece and board square assets, embeds them
//! using `include_bytes!`, rasterizes SVGs to RGBA images, and caches the
//! resulting `egui` textures by asset key and target pixel size.

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use egui::{ColorImage, Context as EguiContext, TextureHandle, TextureOptions};
use shakmaty::{Color, Piece, Role};
use tiny_skia::{Pixmap, Transform};
use usvg::{Options, Tree};

/// The currently selected board square color theme.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BoardTheme {
    /// Brown light and dark squares.
    Brown,
    /// Gray light and dark squares.
    Gray,
    /// Purple light and dark squares.
    Purple,
    /// Tropical light and dark squares.
    Tropical,
}

/// A logical asset identifier used by the raster cache.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AssetKey {
    /// One of the four board square images.
    Square {
        /// The board theme that chooses the asset family.
        theme: BoardTheme,
        /// Whether the square uses the light variant.
        light: bool,
    },
    /// One of the twelve chess piece images.
    Piece {
        /// The piece color.
        color: Color,
        /// The piece role.
        role: Role,
    },
}

/// Embedded SVG bytes plus lazily populated texture cache.
pub struct AssetStore {
    /// Keyed by the logical asset and requested size. The same SVG can be
    /// rasterized at different sizes if the board is resized.
    texture_cache: HashMap<(AssetKey, u32), TextureHandle>,
}

impl AssetStore {
    /// Creates an empty asset store.
    pub fn new() -> Self {
        Self {
            texture_cache: HashMap::new(),
        }
    }

    /// Returns a cached texture for the requested asset and size, rasterizing
    /// the SVG if this is the first request for that combination.
    ///
    /// Returns an error if the SVG cannot be parsed, rasterized, or uploaded
    /// to `egui`.
    pub fn texture(
        &mut self,
        ctx: &EguiContext,
        key: AssetKey,
        size_px: u32,
    ) -> Result<&TextureHandle> {
        let cache_key = (key, size_px.max(1));

        if !self.texture_cache.contains_key(&cache_key) {
            // First use of this asset/size: turn SVG bytes into pixels and give
            // them to egui as a texture.
            let image = rasterize_svg(svg_bytes(key), cache_key.1)?;
            let texture = ctx.load_texture(
                format!("asset-{key:?}-{}", cache_key.1),
                image,
                TextureOptions::LINEAR,
            );
            self.texture_cache.insert(cache_key, texture);
        }

        self.texture_cache
            .get(&cache_key)
            .ok_or_else(|| anyhow!("texture cache insert failed for {key:?}"))
    }
}

/// Maps a `shakmaty::Piece` to the embedded piece asset key.
pub fn piece_asset_key(piece: Piece) -> AssetKey {
    AssetKey::Piece {
        color: piece.color,
        role: piece.role,
    }
}

fn svg_bytes(key: AssetKey) -> &'static [u8] {
    // `include_bytes!` embeds the SVG file into the executable at compile time.
    // That means the app does not need to find asset files at runtime.
    match key {
        AssetKey::Square {
            theme: BoardTheme::Brown,
            light: true,
        } => include_bytes!("../assets/squares/square_brown_light.svg"),
        AssetKey::Square {
            theme: BoardTheme::Brown,
            light: false,
        } => include_bytes!("../assets/squares/square_brown_dark.svg"),
        AssetKey::Square {
            theme: BoardTheme::Gray,
            light: true,
        } => include_bytes!("../assets/squares/square_gray_light.svg"),
        AssetKey::Square {
            theme: BoardTheme::Gray,
            light: false,
        } => include_bytes!("../assets/squares/square_gray_dark.svg"),
        AssetKey::Square {
            theme: BoardTheme::Purple,
            light: true,
        } => include_bytes!("../assets/squares/square_light_purple.svg"),
        AssetKey::Square {
            theme: BoardTheme::Purple,
            light: false,
        } => include_bytes!("../assets/squares/square_dark_purple.svg"),
        AssetKey::Square {
            theme: BoardTheme::Tropical,
            light: true,
        } => include_bytes!("../assets/squares/square_tropical_light.svg"),
        AssetKey::Square {
            theme: BoardTheme::Tropical,
            light: false,
        } => include_bytes!("../assets/squares/square_tropical_dark.svg"),
        AssetKey::Piece {
            color: Color::White,
            role: Role::Pawn,
        } => include_bytes!("../assets/pieces/w_pawn.svg"),
        AssetKey::Piece {
            color: Color::White,
            role: Role::Knight,
        } => include_bytes!("../assets/pieces/w_knight.svg"),
        AssetKey::Piece {
            color: Color::White,
            role: Role::Bishop,
        } => include_bytes!("../assets/pieces/w_bishop.svg"),
        AssetKey::Piece {
            color: Color::White,
            role: Role::Rook,
        } => include_bytes!("../assets/pieces/w_rook.svg"),
        AssetKey::Piece {
            color: Color::White,
            role: Role::Queen,
        } => include_bytes!("../assets/pieces/w_queen.svg"),
        AssetKey::Piece {
            color: Color::White,
            role: Role::King,
        } => include_bytes!("../assets/pieces/w_king.svg"),
        AssetKey::Piece {
            color: Color::Black,
            role: Role::Pawn,
        } => include_bytes!("../assets/pieces/b_pawn.svg"),
        AssetKey::Piece {
            color: Color::Black,
            role: Role::Knight,
        } => include_bytes!("../assets/pieces/b_knight.svg"),
        AssetKey::Piece {
            color: Color::Black,
            role: Role::Bishop,
        } => include_bytes!("../assets/pieces/b_bishop.svg"),
        AssetKey::Piece {
            color: Color::Black,
            role: Role::Rook,
        } => include_bytes!("../assets/pieces/b_rook.svg"),
        AssetKey::Piece {
            color: Color::Black,
            role: Role::Queen,
        } => include_bytes!("../assets/pieces/b_queen.svg"),
        AssetKey::Piece {
            color: Color::Black,
            role: Role::King,
        } => include_bytes!("../assets/pieces/b_king.svg"),
    }
}

fn rasterize_svg(bytes: &[u8], size_px: u32) -> Result<ColorImage> {
    // Parse the SVG into a tree that resvg can render.
    let options = Options::default();
    let tree = Tree::from_data(bytes, &options).context("failed to parse embedded SVG")?;
    let width = size_px.max(1);
    let height = size_px.max(1);
    // A Pixmap is an RGBA pixel buffer that resvg can draw into.
    let mut pixmap = Pixmap::new(width, height)
        .ok_or_else(|| anyhow!("failed to allocate pixmap {width}x{height}"))?;

    let svg_size = tree.size();
    // Scale the SVG into the requested square texture while keeping its aspect ratio.
    let scale = (size_px as f32 / svg_size.width()).min(size_px as f32 / svg_size.height());
    let x = (size_px as f32 - svg_size.width() * scale) * 0.5;
    let y = (size_px as f32 - svg_size.height() * scale) * 0.5;
    let transform = Transform::from_scale(scale, scale).post_translate(x, y);

    // SVGs may have arbitrary viewBox sizes, so the transform normalizes them
    // into a square texture while preserving aspect ratio and centering.
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let pixels = pixmap.data().to_vec();
    Ok(ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        &pixels,
    ))
}
