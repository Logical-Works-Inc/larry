//! Where the chat column and the artifact pane sit (larry#239).
//!
//! The Director, 2026-11-16: "I want the chat space to be condensed and similar
//! aspect ratio to phone (on a computer as well) with the rest of the computer
//! real estate being used for artifact like visualizations". A phone is 9:19.5;
//! a terminal cell is about twice as tall as it is wide, so a column as tall as
//! the screen keeps that shape at `rows * 18 / 19.5`, or `rows * 12 / 13`,
//! columns. The width is held to OpenCode's chat bounds (`ui/layout.ts`:
//! `SESSION_CONTENT_MIN_WIDTH = 44`, `PREFERRED = 64`), and the artifact pane
//! takes the rest once the screen is 108 columns (`SESSION_PANEL_SPLIT_MIN_WIDTH`).
//! Below that the chat is the whole screen, as on a phone.

use super::buffer::Rect;

/// Narrowest chat column beside the artifact pane, in cells.
pub(crate) const CHAT_MIN: usize = 44;
/// Widest chat column, in cells.
pub(crate) const CHAT_MAX: usize = 64;
/// Narrowest screen that holds the chat column and the artifact pane.
pub(crate) const SPLIT_MIN: usize = 108;

/// The two regions of a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Split {
    /// The chat column, always the left edge.
    pub(crate) chat: Rect,
    /// The artifact pane right of a one-cell rule, when the screen is wide enough.
    pub(crate) artifact: Option<Rect>,
}

/// The chat column's width on a screen `rows` tall: the phone ratio, held to
/// [`CHAT_MIN`]..=[`CHAT_MAX`].
pub(crate) fn chat_width(rows: usize) -> usize {
    let phone = match rows.saturating_mul(12).checked_div(13) {
        Some(columns) => columns,
        None => CHAT_MIN,
    };
    phone.clamp(CHAT_MIN, CHAT_MAX)
}

/// The regions of a `width` by `height` screen.
pub(crate) fn split(width: usize, height: usize) -> Split {
    if width < SPLIT_MIN {
        return Split {
            chat: Rect {
                x: 0,
                y: 0,
                width,
                height,
            },
            artifact: None,
        };
    }
    let chat = chat_width(height);
    let pane = chat.saturating_add(1);
    Split {
        chat: Rect {
            x: 0,
            y: 0,
            width: chat,
            height,
        },
        artifact: Some(Rect {
            x: pane,
            y: 0,
            width: width.saturating_sub(pane),
            height,
        }),
    }
}
