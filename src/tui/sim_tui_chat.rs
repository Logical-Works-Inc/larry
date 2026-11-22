//! The chat column paints only left of the pane's rule (larry#340). `split` gives the
//! chat `CHAT_MIN`..=`CHAT_MAX` columns and the pane the rest, and `App::draw` must
//! hand the transcript the chat's own limit. The old bound check read the grid's
//! width, which `Buffer::new` clamps by construction; this one reads the frame the
//! person sees and looks for the chat's own text right of the `│` that opens the pane.

use super::super::app::App;
use super::{Event, Input, PAINT_LANES, SEEDS, header, split, sweep_on};

/// A chat line long enough to reach the pane if a paint forgot its column limit.
const CHAT_LEAK: &str = "CHATLEAK ";

/// Paints a chat at `width` by `height` that holds `CHAT_LEAK` and asserts no character
/// of it sits right of the pane's rule. The rule is the one `pane::paint` draws on every
/// row of the pane's height, so a frame with a pane always shows it.
fn assert_chat_stays_left_of_the_pane(width: usize, height: usize, seed: u64) {
    let mut app = App::new(header(), Vec::new(), width, height);
    app.handle(Input::Worker(Event::Handed {
        line: CHAT_LEAK.repeat(120),
        applied: false,
    }));
    let frame = app.draw();
    let regions = split(frame.buffer.width(), frame.buffer.height());
    let Some(pane) = regions.artifact else {
        return;
    };
    assert!(pane.width > 0, "seed {seed}: an empty pane");
    let mut ruled = 0_usize;
    for row in 0..frame.buffer.height() {
        let text = frame.buffer.row_text(row);
        if let Some(rule) = text.find('│') {
            ruled = ruled.saturating_add(1);
            if let Some(after) = text.get(rule.saturating_add('│'.len_utf8())..) {
                assert!(
                    !after.contains("CHATLEAK"),
                    "seed {seed} {width}x{height} row {row}: chat text painted in the pane columns: {text:?}"
                );
            }
        }
    }
    assert!(
        ruled > 0,
        "seed {seed} {width}x{height}: the pane painted no rule"
    );
}

/// No seed and no size lets chat text cross the rule into the pane's columns.
#[test]
fn sim_tui_chat_text_never_paints_inside_the_pane_columns() {
    let swept = sweep_on(PAINT_LANES, SEEDS, |_lane, seed| {
        assert_chat_stays_left_of_the_pane(120, 34, seed);
    });
    assert!(swept.is_ok(), "the seeded sweep ran: {swept:?}");
    for (width, height) in [
        (108, 10),
        (108, 34),
        (120, 90),
        (160, 34),
        (240, 48),
        (400, 500),
    ] {
        assert_chat_stays_left_of_the_pane(width, height, 0);
    }
}
