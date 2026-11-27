//! The cell grid owns no heap text per cell, and the bytes it paints are the ones it always did
//! (larry#415). A cell held a `String`, so every `Buffer::new` was one allocation per cell and
//! every drop one free: 4.2 s of the 6.5 s the seeded frame sims spend painting 6,144 frames.
//! Requirement proved: a blank grid and an ordinary cell hold no heap, a combining mark joins the
//! cell before it exactly as text, and the wire of 96 seeds is the one the `String` cell wrote.
//! Source: ECMA-48 SGR and CUP; Unicode Standard 16.0 chapter 3 (combining character sequences).

use lgwks_std::hash::Hasher;

use super::super::buffer::Buffer;
use super::super::style::Style;
use super::{Seeded, drive};

/// Seeds the grid simulation sweeps.
const SEEDS: u64 = 64;

/// Seeds whose wire the pinned digest covers.
const PINNED_SEEDS: u64 = 96;

/// The BLAKE3 of the wire of seeds `0..PINNED_SEEDS`, 96 steps each, taken with the
/// command header painted at `started` so a running command's output has its context,
/// then advanced once for the queue panel, the queue help line and the working footer
/// (larry#262): the heap assertions above, not this digest, carry the no-heap-text
/// rule, so a paint feature moves this digest and names its issue here.
const PINNED: &str = "bce3b72ede00a060f3a04ccf7bb0786a907e0cb4bae6028549bb5458cd4823ea";

/// One piece of a seeded line: its text, the columns it takes and whether a mark joins its cell.
const PIECES: [(&str, usize, bool); 6] = [
    ("a", 1, false),
    (" ", 1, false),
    ("表", 2, false),
    ("😀", 2, false),
    ("e\u{301}", 1, true),
    ("o\u{302}\u{323}", 1, true),
];

/// A seeded line of pieces fits one row of a grid: no cell is heap text unless a combining mark
/// joined it, the heap cells are exactly the pieces that carry a mark, and the row reads back as
/// the pieces followed by blanks. A wide piece reads once, and a character written over the first
/// cell of a wide one leaves no half character behind.
#[test]
fn sim_a_grid_owns_no_heap_text_per_cell() -> Result<(), Box<dyn std::error::Error>> {
    let blank = Buffer::new(260, 90);
    assert_eq!(blank.heap_cells(), 0, "a blank grid owns no heap text");
    for seed in 0..SEEDS {
        let mut rng = Seeded::new(seed);
        let mut grid = Buffer::new(400, 2);
        let (mut text, mut columns, mut marked) = (String::new(), 0_usize, 0_usize);
        let mut first_wide = false;
        for step in 0..rng.pick(60) {
            let &(piece, width, has_mark) = PIECES.get(rng.pick(PIECES.len())).ok_or("a piece")?;
            let end = grid.put(columns, 0, 400, piece, Style::PLAIN);
            assert_eq!(end, columns.saturating_add(width), "seed {seed}: {piece:?}");
            first_wide = if step == 0 { width == 2 } else { first_wide };
            text.push_str(piece);
            columns = end;
            marked = marked.saturating_add(usize::from(has_mark));
        }
        assert_eq!(
            grid.heap_cells(),
            marked,
            "seed {seed}: one heap cell per mark"
        );
        text.push_str(&" ".repeat(400_usize.saturating_sub(columns)));
        assert_eq!(grid.row_text(0), text, "seed {seed}: the row reads back");
        if columns > 0 {
            assert_eq!(grid.put(0, 0, 400, "x", Style::PLAIN), 1, "seed {seed}");
            let read = grid.row_text(0);
            assert!(read.starts_with('x'), "seed {seed}: {read:?}");
            if first_wide {
                let rest = read.get('x'.len_utf8()..).ok_or("the rest of the row")?;
                assert!(
                    rest.starts_with(' '),
                    "seed {seed}: the half of a wide character is blanked: {read:?}"
                );
            }
        }
    }
    Ok(())
}

/// The wire of 96 seeds of 96 hostile steps, pinned as one digest: the pane paints the
/// command header at `started` (larry#252), so a running command's rows carry their `$`
/// context while it streams instead of only at `finished`.
#[test]
fn sim_the_wire_of_every_seed_is_the_pinned_one() {
    let mut hasher = Hasher::new();
    for seed in 0..PINNED_SEEDS {
        let (_, wire) = drive(seed, 96);
        hasher.update(&wire);
    }
    assert_eq!(hasher.finalize().to_hex(), PINNED);
}
