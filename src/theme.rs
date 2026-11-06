//! The Forge colour language, re-implemented against AppCUI.
//!
//! Tokens are the exact 24-bit values read from the Forge TUI
//! (`forge-harness/forge-tui/src/theme.rs`, inspected 2026-09-18): slate
//! `#2C2C2C`, cream `#F5F0E0`, muted `#968F87`, emerald `#50C878`, amber
//! `#FFBF00`, ruby `#E0115F`, cyan `#58A6FF`. Forge is reference material
//! only — no Forge code is imported, copied or ported (Director correction
//! 2026-09-16); this module re-states the token set and maps it onto what
//! this build can actually render.
//!
//! Two constraints shape the mapping, and both are deliberate:
//!
//! 1. **Colour depth.** `lgwks_deps` pins appcui with
//!    `default-features = false`, so AppCUI's `Color` is the 16-colour enum
//!    and `Color::from_rgb` quantizes with a crude per-channel `> 64` test.
//!    Measured against that test, Forge's emerald (80,200,120) and its cyan
//!    (88,166,255) both land on `Silver` — the accent and the MI chip would
//!    read as grey. [`TOKENS`] therefore names each ANSI16 stand-in
//!    explicitly. When the estate forwards `appcui/TRUE_COLORS` through
//!    `lgwks_deps`, the stand-ins become `Color::RGB` and nothing outside
//!    this file changes.
//!
//! 2. **Theme universality.** Forge paints body text `fg(CREAM)`, which
//!    assumes a dark terminal. Larry's accessibility pass (2026-09-18) made
//!    every body role resolve to the terminal default so light, dark,
//!    `NO_COLOR` and dumb terminals share one branch-free path. Body text
//!    therefore stays [`BASE`] = `Transparent`; Forge's *accent* language —
//!    which is what makes the surface recognisable — is rendered exactly.

use lgwks_deps::appcui::prelude::Color;

/// Every Forge token: `(name, 24-bit value, the colour this build renders)`.
///
/// This table is the single source for the palette. It is also the falsifier
/// for the mapping: see `tokens_stay_distinguishable` in the tests.
pub const TOKENS: [(&str, (u8, u8, u8), Color); 7] = [
    ("slate", (44, 44, 44), Color::Black),
    ("cream", (245, 240, 224), Color::Transparent),
    ("muted", (150, 145, 135), Color::Gray),
    ("emerald", (80, 200, 120), Color::Green),
    ("amber", (255, 191, 0), Color::Yellow),
    ("ruby", (224, 17, 95), Color::Red),
    ("cyan", (88, 166, 255), Color::Blue),
];

/// Body text — Forge `base()`, rendered as the terminal default (constraint 2).
pub const BASE: Color = Color::Transparent;

/// Secondary text — Forge `dim()`. `Gray` is the one ANSI16 tone that stays
/// readable on both a light and a dark background, so it keeps the
/// primary/secondary distinction without pinning a theme.
pub const MUTED: Color = Color::Gray;

/// The single accent — Forge `accent()` (emerald).
pub const ACCENT: Color = Color::Green;

/// Caution — Forge `warning()` (amber).
pub const WARN: Color = Color::Yellow;

/// Failure — Forge `error()` (ruby).
pub const ERR: Color = Color::Red;

/// The MI chip — Forge cyan.
pub const MI: Color = Color::Blue;

/// The canary chip's foreground. Forge draws the chip as `fg(slate) bg(colour)`;
/// slate becomes `Black` at this depth, which is legible on all three canary
/// backgrounds.
pub const CHIP: Color = Color::Black;
