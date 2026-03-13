//! Color scheme and styling constants.

use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

/// Global active theme variant — read by `Colors::active()`.
static ACTIVE_VARIANT: AtomicU8 = AtomicU8::new(0);

/// Available theme variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ThemeVariant {
    #[default]
    Caboose,
    Firebox,
    SteamDome,
    Smokebox,
    SandDome,
}

impl ThemeVariant {
    pub const ALL: &[ThemeVariant] = &[
        ThemeVariant::Caboose,
        ThemeVariant::Firebox,
        ThemeVariant::SteamDome,
        ThemeVariant::Smokebox,
        ThemeVariant::SandDome,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Caboose => "Caboose",
            Self::Firebox => "Firebox",
            Self::SteamDome => "Steam Dome",
            Self::Smokebox => "Smokebox",
            Self::SandDome => "Sand Dome",
        }
    }

    #[allow(dead_code)]
    pub fn description(&self) -> &'static str {
        match self {
            Self::Caboose => "Classic caboose maroon (default)",
            Self::Firebox => "Blazing orange, intense heat",
            Self::SteamDome => "Cool steel blue, ethereal",
            Self::Smokebox => "Warm ash, industrial soot",
            Self::SandDome => "Golden sand, desert amber",
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            Self::Caboose => 0,
            Self::Firebox => 1,
            Self::SteamDome => 2,
            Self::Smokebox => 3,
            Self::SandDome => 4,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Firebox,
            2 => Self::SteamDome,
            3 => Self::Smokebox,
            4 => Self::SandDome,
            _ => Self::Caboose,
        }
    }

    /// Cycle to the next variant.
    #[allow(dead_code)]
    pub fn next(self) -> Self {
        let idx = (self.to_u8() + 1) % Self::ALL.len() as u8;
        Self::from_u8(idx)
    }
}

/// Set the global active theme variant.
pub fn set_active_variant(variant: ThemeVariant) {
    ACTIVE_VARIANT.store(variant.to_u8(), Ordering::Relaxed);
}

/// Get the current global active theme variant.
pub fn active_variant() -> ThemeVariant {
    ThemeVariant::from_u8(ACTIVE_VARIANT.load(Ordering::Relaxed))
}

/// Application color palette — dark theme with railroad brand accent.
#[allow(dead_code)]
pub struct Colors {
    // Backgrounds
    pub bg_primary: Color,
    pub bg_secondary: Color,
    pub bg_elevated: Color,
    pub bg_hover: Color,

    // Borders
    pub border: Color,
    pub border_active: Color,

    // Text hierarchy
    pub text: Color,
    pub text_secondary: Color,
    pub text_dim: Color,
    pub text_muted: Color,

    // Chat roles
    pub user_msg: Color,
    pub assistant_msg: Color,
    pub tool_msg: Color,
    pub system_msg: Color,

    // Semantic
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,

    // Brand
    pub brand: Color,
    pub brand_muted: Color,

    // Roundhouse
    pub roundhouse: Color,

    // Code blocks
    pub code_bg: Color,
    pub code_border: Color,
    pub code_text: Color,

    // Markdown elements
    pub header_h1: Color,
    pub header_h2: Color,
    pub header_h3: Color,
    pub blockquote_border: Color,
    pub link_text: Color,
    pub horizontal_rule: Color,
    pub list_bullet: Color,
}

impl Colors {
    /// Get colors for the current global active theme variant.
    pub fn active() -> Self {
        Self::for_variant(active_variant())
    }

    /// Get colors for a specific theme variant.
    pub fn for_variant(variant: ThemeVariant) -> Self {
        match variant {
            ThemeVariant::Caboose => Self::caboose(),
            ThemeVariant::Firebox => Self::firebox(),
            ThemeVariant::SteamDome => Self::steam_dome(),
            ThemeVariant::Smokebox => Self::smokebox(),
            ThemeVariant::SandDome => Self::sand_dome(),
        }
    }

    /// Caboose — classic caboose maroon (default).
    fn caboose() -> Self {
        Self {
            // Backgrounds
            bg_primary: Color::Rgb(10, 10, 10),   // #0a0a0a
            bg_secondary: Color::Rgb(20, 20, 20), // #141414
            bg_elevated: Color::Rgb(26, 26, 26),  // #1a1a1a
            bg_hover: Color::Rgb(30, 30, 30),     // #1e1e1e

            // Borders
            border: Color::Rgb(42, 42, 42),         // #2a2a2a
            border_active: Color::Rgb(155, 35, 53), // #9b2335 — brand accent

            // Text hierarchy
            text: Color::Rgb(238, 238, 238),           // #eeeeee
            text_secondary: Color::Rgb(160, 160, 160), // #a0a0a0
            text_dim: Color::Rgb(96, 96, 96),          // #606060
            text_muted: Color::Rgb(64, 64, 64),        // #404040

            // Chat roles
            user_msg: Color::Rgb(238, 238, 238),      // #eeeeee
            assistant_msg: Color::Rgb(200, 200, 200), // #c8c8c8
            tool_msg: Color::Rgb(155, 35, 53),        // #9b2335 — brand
            system_msg: Color::Rgb(96, 96, 96),       // #606060

            // Semantic
            success: Color::Rgb(78, 200, 112), // #4ec870
            warning: Color::Rgb(232, 184, 61), // #e8b83d
            error: Color::Rgb(232, 84, 84),    // #e85454
            info: Color::Rgb(91, 155, 213),    // #5b9bd5

            // Brand
            brand: Color::Rgb(155, 35, 53), // #9b2335 — classic caboose maroon
            brand_muted: Color::Rgb(128, 128, 128), // #808080

            roundhouse: Color::Rgb(168, 85, 247), // #a855f7 — purple

            // Code blocks
            code_bg: Color::Rgb(20, 20, 20),      // #141414
            code_border: Color::Rgb(42, 42, 42),  // #2a2a2a
            code_text: Color::Rgb(212, 212, 212), // #d4d4d4

            // Markdown elements
            header_h1: Color::Rgb(155, 35, 53),  // #9b2335 — brand
            header_h2: Color::Rgb(212, 160, 42), // #d4a02a — warm amber
            header_h3: Color::Rgb(160, 160, 160), // #a0a0a0 — text_secondary
            blockquote_border: Color::Rgb(64, 64, 64), // #404040 — text_muted
            link_text: Color::Rgb(91, 155, 213), // #5b9bd5 — info
            horizontal_rule: Color::Rgb(42, 42, 42), // #2a2a2a — border
            list_bullet: Color::Rgb(96, 96, 96), // #606060 — text_dim
        }
    }

    /// Firebox — blazing orange, intense furnace heat.
    fn firebox() -> Self {
        Self {
            bg_primary: Color::Rgb(12, 8, 6),     // #0c0806
            bg_secondary: Color::Rgb(22, 18, 16), // #161210
            bg_elevated: Color::Rgb(30, 24, 20),  // #1e1814
            bg_hover: Color::Rgb(38, 30, 24),     // #261e18

            border: Color::Rgb(54, 42, 34),         // #362a22
            border_active: Color::Rgb(232, 84, 27), // #e8541b

            text: Color::Rgb(238, 232, 224),           // #eee8e0
            text_secondary: Color::Rgb(168, 152, 140), // #a8988c
            text_dim: Color::Rgb(104, 90, 80),         // #685a50
            text_muted: Color::Rgb(72, 60, 52),        // #483c34

            user_msg: Color::Rgb(238, 232, 224),      // #eee8e0
            assistant_msg: Color::Rgb(208, 196, 184), // #d0c4b8
            tool_msg: Color::Rgb(232, 84, 27),        // #e8541b
            system_msg: Color::Rgb(104, 90, 80),      // #685a50

            success: Color::Rgb(78, 200, 112), // #4ec870
            warning: Color::Rgb(232, 160, 32), // #e8a020
            error: Color::Rgb(232, 64, 64),    // #e84040
            info: Color::Rgb(212, 128, 64),    // #d48040

            brand: Color::Rgb(232, 84, 27), // #e8541b — blazing orange
            brand_muted: Color::Rgb(122, 64, 48), // #7a4030

            roundhouse: Color::Rgb(168, 85, 247), // #a855f7 — purple

            code_bg: Color::Rgb(22, 18, 16),      // #161210
            code_border: Color::Rgb(54, 42, 34),  // #362a22
            code_text: Color::Rgb(220, 208, 196), // #dcd0c4

            header_h1: Color::Rgb(232, 84, 27),  // #e8541b — brand
            header_h2: Color::Rgb(232, 160, 32), // #e8a020 — golden flame
            header_h3: Color::Rgb(160, 144, 136), // #a09088 — ash gray
            blockquote_border: Color::Rgb(72, 60, 52), // #483c34
            link_text: Color::Rgb(212, 128, 64), // #d48040 — warm info
            horizontal_rule: Color::Rgb(54, 42, 34), // #362a22
            list_bullet: Color::Rgb(104, 90, 80), // #685a50
        }
    }

    /// Steam Dome — cool steel blue, ethereal and silvery.
    fn steam_dome() -> Self {
        Self {
            bg_primary: Color::Rgb(8, 10, 14),    // #080a0e
            bg_secondary: Color::Rgb(16, 20, 24), // #101418
            bg_elevated: Color::Rgb(24, 28, 34),  // #181c22
            bg_hover: Color::Rgb(30, 34, 40),     // #1e2228

            border: Color::Rgb(42, 48, 56),           // #2a3038
            border_active: Color::Rgb(124, 184, 216), // #7cb8d8

            text: Color::Rgb(228, 234, 238),           // #e4eaee
            text_secondary: Color::Rgb(148, 162, 172), // #94a2ac
            text_dim: Color::Rgb(88, 100, 112),        // #586470
            text_muted: Color::Rgb(56, 66, 76),        // #38424c

            user_msg: Color::Rgb(228, 234, 238),      // #e4eaee
            assistant_msg: Color::Rgb(192, 204, 212), // #c0ccd4
            tool_msg: Color::Rgb(124, 184, 216),      // #7cb8d8
            system_msg: Color::Rgb(88, 100, 112),     // #586470

            success: Color::Rgb(78, 200, 112), // #4ec870
            warning: Color::Rgb(232, 184, 61), // #e8b83d
            error: Color::Rgb(232, 84, 84),    // #e85454
            info: Color::Rgb(124, 184, 216),   // #7cb8d8

            brand: Color::Rgb(124, 184, 216), // #7cb8d8 — steel blue
            brand_muted: Color::Rgb(74, 104, 120), // #4a6878

            roundhouse: Color::Rgb(168, 85, 247), // #a855f7 — purple

            code_bg: Color::Rgb(16, 20, 24),      // #101418
            code_border: Color::Rgb(42, 48, 56),  // #2a3038
            code_text: Color::Rgb(200, 212, 220), // #c8d4dc

            header_h1: Color::Rgb(124, 184, 216), // #7cb8d8 — brand
            header_h2: Color::Rgb(168, 200, 216), // #a8c8d8 — silver
            header_h3: Color::Rgb(144, 154, 160), // #909aa0 — cool gray
            blockquote_border: Color::Rgb(56, 66, 76), // #38424c
            link_text: Color::Rgb(140, 196, 228), // #8cc4e4
            horizontal_rule: Color::Rgb(42, 48, 56), // #2a3038
            list_bullet: Color::Rgb(88, 100, 112), // #586470
        }
    }

    /// Smokebox — dark, moody, industrial soot with desaturated palette.
    fn smokebox() -> Self {
        Self {
            bg_primary: Color::Rgb(10, 9, 8),     // #0a0908
            bg_secondary: Color::Rgb(20, 19, 16), // #141310
            bg_elevated: Color::Rgb(28, 26, 22),  // #1c1a16
            bg_hover: Color::Rgb(36, 32, 28),     // #24201c

            border: Color::Rgb(52, 46, 40),           // #342e28
            border_active: Color::Rgb(160, 136, 120), // #a08878

            text: Color::Rgb(220, 212, 204),           // #dcd4cc
            text_secondary: Color::Rgb(152, 144, 136), // #989088
            text_dim: Color::Rgb(96, 88, 80),          // #605850
            text_muted: Color::Rgb(68, 62, 56),        // #443e38

            user_msg: Color::Rgb(220, 212, 204),      // #dcd4cc
            assistant_msg: Color::Rgb(188, 180, 172), // #bcb4ac
            tool_msg: Color::Rgb(160, 136, 120),      // #a08878
            system_msg: Color::Rgb(96, 88, 80),       // #605850

            success: Color::Rgb(120, 184, 128), // #78b880 — desaturated
            warning: Color::Rgb(200, 168, 96),  // #c8a860 — desaturated
            error: Color::Rgb(200, 104, 88),    // #c86858 — desaturated
            info: Color::Rgb(136, 152, 168),    // #8898a8 — desaturated

            brand: Color::Rgb(160, 136, 120), // #a08878 — warm ash
            brand_muted: Color::Rgb(96, 84, 72), // #605448

            roundhouse: Color::Rgb(168, 85, 247), // #a855f7 — purple

            code_bg: Color::Rgb(20, 19, 16),      // #141310
            code_border: Color::Rgb(52, 46, 40),  // #342e28
            code_text: Color::Rgb(192, 184, 176), // #c0b8b0

            header_h1: Color::Rgb(160, 136, 120), // #a08878 — brand
            header_h2: Color::Rgb(192, 176, 160), // #c0b0a0 — pale smoke
            header_h3: Color::Rgb(136, 128, 120), // #888078 — muted ash
            blockquote_border: Color::Rgb(68, 62, 56), // #443e38
            link_text: Color::Rgb(136, 152, 168), // #8898a8 — muted info
            horizontal_rule: Color::Rgb(52, 46, 40), // #342e28
            list_bullet: Color::Rgb(96, 88, 80),  // #605850
        }
    }

    /// Sand Dome — golden sand, warm desert amber.
    fn sand_dome() -> Self {
        Self {
            bg_primary: Color::Rgb(12, 10, 6),   // #0c0a06
            bg_secondary: Color::Rgb(22, 20, 8), // #161408
            bg_elevated: Color::Rgb(30, 26, 16), // #1e1a10
            bg_hover: Color::Rgb(38, 32, 22),    // #262016

            border: Color::Rgb(54, 44, 28),          // #362c1c
            border_active: Color::Rgb(212, 168, 80), // #d4a850

            text: Color::Rgb(238, 232, 216),           // #eee8d8
            text_secondary: Color::Rgb(168, 160, 136), // #a8a088
            text_dim: Color::Rgb(104, 96, 76),         // #68604c
            text_muted: Color::Rgb(72, 66, 50),        // #484232

            user_msg: Color::Rgb(238, 232, 216),      // #eee8d8
            assistant_msg: Color::Rgb(208, 200, 180), // #d0c8b4
            tool_msg: Color::Rgb(212, 168, 80),       // #d4a850
            system_msg: Color::Rgb(104, 96, 76),      // #68604c

            success: Color::Rgb(120, 200, 112), // #78c870
            warning: Color::Rgb(212, 168, 80),  // #d4a850 — matches brand
            error: Color::Rgb(216, 112, 80),    // #d87050
            info: Color::Rgb(160, 184, 200),    // #a0b8c8

            brand: Color::Rgb(212, 168, 80), // #d4a850 — golden sand
            brand_muted: Color::Rgb(138, 122, 72), // #8a7a48

            roundhouse: Color::Rgb(168, 85, 247), // #a855f7 — purple

            code_bg: Color::Rgb(22, 20, 8),       // #161408
            code_border: Color::Rgb(54, 44, 28),  // #362c1c
            code_text: Color::Rgb(216, 208, 188), // #d8d0bc

            header_h1: Color::Rgb(212, 168, 80), // #d4a850 — brand
            header_h2: Color::Rgb(232, 200, 120), // #e8c878 — light sand
            header_h3: Color::Rgb(160, 152, 120), // #a09878 — dusty
            blockquote_border: Color::Rgb(72, 66, 50), // #484232
            link_text: Color::Rgb(160, 184, 200), // #a0b8c8 — cool contrast
            horizontal_rule: Color::Rgb(54, 44, 28), // #362c1c
            list_bullet: Color::Rgb(104, 96, 76), // #68604c
        }
    }
}

impl Default for Colors {
    fn default() -> Self {
        Self::active()
    }
}
