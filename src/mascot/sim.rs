//! Seeded controls for `larry mascot` (larry#152): the WebP encoder against a test-side VP8L
//! reader written from RFC 9649 (it refuses an incomplete prefix code, as libwebp does), the
//! spritesheet against its frames, the committed SVG against the render, every command-line
//! refusal, and whole-file replacement of the written files.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use lgwks_std::seeded::Seeded;

use crate::governance::sim_support::Scratch;

use super::webp::{WebpError, encode};
use super::{
    ATLAS_HEIGHT, ATLAS_WIDTH, MANIFEST, MascotError, OPEN, Request, USAGE, atlas, is_current, run,
    spritesheet, svg,
};

/// A test outcome.
type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;
/// Seeds per simulation.
const SEEDS: u64 = 64;
/// The order the code-length code's lengths arrive in (RFC 9649 section 3.7.2.1.2).
const ORDER: [usize; 19] = [
    17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];
/// An LSB-first bit reader over a VP8L payload.
struct Reader<'bytes> {
    /// The payload.
    bytes: &'bytes [u8],
    /// The next bit's index.
    bit: usize,
}

impl Reader<'_> {
    /// The next `width` bits as a number, first bit lowest.
    fn read(&mut self, width: u32) -> Check<u32> {
        let mut value = 0_u32;
        for shift in 0..width {
            let byte = self
                .bytes
                .get(self.bit.wrapping_shr(3))
                .ok_or("read past the payload")?;
            let set = byte.wrapping_shr(u32::try_from(self.bit & 7)?) & 1;
            value |= u32::from(set).wrapping_shl(shift);
            self.bit = self.bit.wrapping_add(1);
        }
        Ok(value)
    }

    /// The next `width` bits as an index.
    fn index(&mut self, width: u32) -> Check<usize> {
        Ok(usize::try_from(self.read(width)?)?)
    }
}

/// A prefix code as the decoder sees it: (length, canonical code) to symbol.
enum Decoding {
    /// One symbol, read in zero bits.
    Single(usize),
    /// A complete code.
    Table(BTreeMap<(u32, u32), usize>),
}

impl Decoding {
    /// Builds the canonical code of `lengths`, refusing an empty or incomplete one.
    fn build(lengths: &[u32]) -> Check<Self> {
        let present: Vec<(usize, u32)> = lengths
            .iter()
            .copied()
            .enumerate()
            .filter(|pair| pair.1 > 0)
            .collect();
        match *present.as_slice() {
            [] => return Err("an empty prefix code".into()),
            [(symbol, _)] => return Ok(Self::Single(symbol)),
            _ => {}
        }
        let mut kraft = 0_u64;
        for pair in &present {
            if pair.1 > 15 {
                return Err(format!("code length {} past 15", pair.1).into());
            }
            kraft = kraft.wrapping_add(1_u64.wrapping_shl(15_u32.wrapping_sub(pair.1)));
        }
        if kraft != 32_768 {
            return Err(format!("an incomplete prefix code (Kraft {kraft}/32768)").into());
        }
        let mut sorted = present;
        sorted.sort_by_key(|pair| (pair.1, pair.0));
        let mut table = BTreeMap::new();
        let mut code = 0_u32;
        let mut length = 0_u32;
        for pair in sorted {
            code = code.wrapping_shl(pair.1.wrapping_sub(length));
            length = pair.1;
            table.insert((length, code), pair.0);
            code = code.wrapping_add(1);
        }
        Ok(Self::Table(table))
    }

    /// Reads one symbol, the code's first bit first.
    fn symbol(&self, reader: &mut Reader<'_>) -> Check<usize> {
        let table = match *self {
            Self::Single(symbol) => return Ok(symbol),
            Self::Table(ref table) => table,
        };
        let mut code = 0_u32;
        for length in 1..=15_u32 {
            code = code.wrapping_shl(1) | reader.read(1)?;
            if let Some(&symbol) = table.get(&(length, code)) {
                return Ok(symbol);
            }
        }
        Err("no symbol within 15 bits".into())
    }
}

/// Reads one prefix code over `alphabet` symbols, simple or normal.
fn read_code(reader: &mut Reader<'_>, alphabet: usize) -> Check<Decoding> {
    let mut lengths = vec![0_u32; alphabet];
    if reader.read(1)? == 1 {
        let count = reader.read(1)?;
        let first_width = if reader.read(1)? == 1 { 8 } else { 1 };
        let mut symbols = vec![reader.index(first_width)?];
        if count == 1 {
            symbols.push(reader.index(8)?);
        }
        for symbol in symbols {
            *lengths
                .get_mut(symbol)
                .ok_or("simple symbol past the alphabet")? = 1;
        }
        return Decoding::build(&lengths);
    }
    let stored = reader.index(4)?.wrapping_add(4);
    let mut code_lengths = [0_u32; 19];
    for &symbol in ORDER.get(..stored).ok_or("more than 19 code lengths")? {
        *code_lengths.get_mut(symbol).ok_or("code-length symbol")? = reader.read(3)?;
    }
    let code_length_code = Decoding::build(&code_lengths)?;
    let mut budget = if reader.read(1)? == 1 {
        let width = 2_u32.wrapping_add(reader.read(3)?.wrapping_mul(2));
        reader.index(width)?.wrapping_add(2)
    } else {
        alphabet
    };
    let mut at = 0_usize;
    let mut previous = 8_u32;
    while at < alphabet && budget > 0 {
        budget = budget.wrapping_sub(1);
        let symbol = code_length_code.symbol(reader)?;
        let (value, repeat) = match symbol {
            0..=15 => (u32::try_from(symbol)?, 1),
            16 => (previous, reader.index(2)?.wrapping_add(3)),
            17 => (0, reader.index(3)?.wrapping_add(3)),
            18 => (0, reader.index(7)?.wrapping_add(11)),
            _ => return Err("code-length symbol past 18".into()),
        };
        if symbol < 16 && value != 0 {
            previous = value;
        }
        for _ in 0..repeat {
            *lengths
                .get_mut(at)
                .ok_or("code lengths past the alphabet")? = value;
            at = at.wrapping_add(1);
        }
    }
    Decoding::build(&lengths)
}

/// The value of a length or distance prefix `symbol` and its extra bits (RFC 9649 5.2.2).
fn prefix_value(reader: &mut Reader<'_>, symbol: usize) -> Check<usize> {
    if symbol < 4 {
        return Ok(symbol.wrapping_add(1));
    }
    let extra_bits = u32::try_from(symbol.wrapping_sub(2).wrapping_shr(1))?;
    let offset = 2_usize.wrapping_add(symbol & 1).wrapping_shl(extra_bits);
    Ok(offset
        .wrapping_add(reader.index(extra_bits)?)
        .wrapping_add(1))
}

/// A little-endian `u32` at `at`.
fn le32(bytes: &[u8], at: usize) -> Check<u32> {
    let slice = bytes
        .get(at..at.wrapping_add(4))
        .ok_or("short RIFF header")?;
    Ok(u32::from_le_bytes(slice.try_into()?))
}

/// Decodes a WebP file of the subset the encoder writes into (width, height, RGBA).
fn decode(file: &[u8]) -> Check<(u32, u32, Vec<u8>)> {
    if file.get(..4) != Some(b"RIFF".as_slice()) || file.get(8..16) != Some(b"WEBPVP8L".as_slice())
    {
        return Err("not a RIFF WebP VP8L file".into());
    }
    if usize::try_from(le32(file, 4)?)? != file.len().wrapping_sub(8) {
        return Err("the RIFF size disagrees with the file".into());
    }
    let chunk = usize::try_from(le32(file, 16)?)?;
    let payload = file
        .get(20..20_usize.wrapping_add(chunk))
        .ok_or("short VP8L chunk")?;
    let mut reader = Reader {
        bytes: payload,
        bit: 0,
    };
    if reader.read(8)? != 0x2f {
        return Err("no VP8L signature".into());
    }
    let width = reader.read(14)?.wrapping_add(1);
    let height = reader.read(14)?.wrapping_add(1);
    let _alpha_hint = reader.read(1)?;
    if reader.read(3)? != 0 {
        return Err("a VP8L version other than 0".into());
    }
    if reader.read(1)? != 0 || reader.read(1)? != 0 || reader.read(1)? != 0 {
        return Err("a transform, colour cache or meta prefix code".into());
    }
    let codes = [
        read_code(&mut reader, 280)?,
        read_code(&mut reader, 256)?,
        read_code(&mut reader, 256)?,
        read_code(&mut reader, 256)?,
        read_code(&mut reader, 40)?,
    ];
    let [ref green, ref red, ref blue, ref alpha, ref distance] = codes;
    let columns = usize::try_from(width)?;
    let total = columns
        .checked_mul(usize::try_from(height)?)
        .ok_or("image too large")?;
    let mut pixels: Vec<[u8; 4]> = Vec::with_capacity(total);
    while pixels.len() < total {
        let green_symbol = green.symbol(&mut reader)?;
        if green_symbol < 256 {
            let red_value = u8::try_from(red.symbol(&mut reader)?)?;
            let blue_value = u8::try_from(blue.symbol(&mut reader)?)?;
            let alpha_value = u8::try_from(alpha.symbol(&mut reader)?)?;
            pixels.push([
                red_value,
                u8::try_from(green_symbol)?,
                blue_value,
                alpha_value,
            ]);
            continue;
        }
        let length = prefix_value(&mut reader, green_symbol.wrapping_sub(256))?;
        let distance_symbol = distance.symbol(&mut reader)?;
        let back = match prefix_value(&mut reader, distance_symbol)? {
            1 => columns,
            2 => 1,
            code if code > 120 => code.wrapping_sub(120),
            code => return Err(format!("plane code {code} is outside the subset").into()),
        };
        for _ in 0..length {
            let from = pixels
                .len()
                .checked_sub(back)
                .ok_or("a reference before the image")?;
            let pixel = *pixels.get(from).ok_or("a reference past the image")?;
            if pixels.len() >= total {
                return Err("a copy past the last pixel".into());
            }
            pixels.push(pixel);
        }
    }
    if reader.bit.wrapping_add(7).wrapping_shr(3) != payload.len() {
        return Err("bytes left after the last pixel".into());
    }
    Ok((width, height, pixels.into_iter().flatten().collect()))
}

/// One seeded image: a palette of up to `colours` RGBA values (some transparent), filled with
/// runs from the left, copies from above and fresh picks.
fn seeded_image(rng: &mut Seeded, colours: usize) -> Check<(u32, u32, Vec<u8>)> {
    let width = rng.index(48)?.wrapping_add(1);
    let height = rng.index(48)?.wrapping_add(1);
    let count = rng.index(colours)?.wrapping_add(1);
    let mut palette = Vec::with_capacity(count);
    for _ in 0..count {
        let mut colour = [0_u8; 4];
        for channel in &mut colour {
            *channel = u8::try_from(rng.index(256)?)?;
        }
        if rng.index(4)? == 0 {
            colour = [0, 0, 0, 0];
        }
        palette.push(colour);
    }
    let mut pixels: Vec<[u8; 4]> = Vec::with_capacity(width.wrapping_mul(height));
    for at in 0..width.wrapping_mul(height) {
        let left = at.checked_sub(1).and_then(|from| pixels.get(from)).copied();
        let above = at
            .checked_sub(width)
            .and_then(|from| pixels.get(from))
            .copied();
        let choice = rng.index(4)?;
        let pixel = match (choice, left, above) {
            (0, Some(pixel), _) | (1, _, Some(pixel)) => pixel,
            _ => *palette
                .get(rng.index(palette.len())?)
                .ok_or("an empty palette")?,
        };
        pixels.push(pixel);
    }
    Ok((
        u32::try_from(width)?,
        u32::try_from(height)?,
        pixels.into_iter().flatten().collect(),
    ))
}

/// Seeded images, from one colour to 300, round-trip exactly through encode and the reader, so
/// simple codes, normal codes, both copy kinds and full 256-symbol channels are all exercised.
#[test]
fn sim_encoded_images_decode_to_their_pixels_for_every_seed() -> Check {
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let colours = if seed & 3 == 3 { 300 } else { 12 };
        let (width, height, rgba) = seeded_image(&mut rng, colours)?;
        let file = encode(&rgba, width, height)?;
        let decoded = decode(&file).map_err(|error| format!("seed {seed}: {error}"))?;
        assert_eq!(
            decoded,
            (width, height, rgba),
            "seed {seed}: the round trip"
        );
    }
    Ok(())
}

/// The sizes at the edges: one pixel, a run longer than one copy can carry, a column, a row of
/// distinct pixels, and the largest side.
#[test]
fn edge_images_round_trip() -> Check {
    let distinct: Vec<u8> = (0..=255_u8)
        .flat_map(|value| [value, value, value, 255])
        .collect();
    let cases: [(u32, u32, Vec<u8>); 5] = [
        (1, 1, vec![9, 8, 7, 6]),
        (9000, 1, [1_u8, 2, 3, 255].repeat(9000)),
        (1, 300, [0_u8, 0, 0, 0].repeat(300)),
        (256, 1, distinct),
        (16_384, 1, [5_u8, 5, 5, 5].repeat(16_384)),
    ];
    for (width, height, rgba) in cases {
        let file = encode(&rgba, width, height)?;
        assert_eq!(
            decode(&file)?,
            (width, height, rgba),
            "{width}x{height} round trip"
        );
    }
    Ok(())
}

#[test]
fn sizes_outside_the_format_and_short_buffers_are_refused() -> Check {
    assert_eq!(
        encode(&[], 0, 1),
        Err(WebpError::Size {
            width: 0,
            height: 1
        }),
        "a zero width"
    );
    assert_eq!(
        encode(&[], 1, 16_385),
        Err(WebpError::Size {
            width: 1,
            height: 16_385
        }),
        "a side past 16384"
    );
    assert_eq!(
        encode(&[0; 7], 2, 1),
        Err(WebpError::Pixels {
            expected: Some(8),
            actual: 7
        }),
        "a short buffer"
    );
    Ok(())
}

/// The spritesheet decodes to the atlas: eight idle frames in the first row (frames 5 and 6
/// blink), every other row transparent, and the eye's catchlight where the grid puts it.
#[test]
fn the_spritesheet_decodes_to_the_idle_loop() -> Check {
    let file = spritesheet()?;
    let (width, height, rgba) = decode(&file)?;
    assert_eq!(
        (usize::try_from(width)?, usize::try_from(height)?),
        (ATLAS_WIDTH, ATLAS_HEIGHT),
        "the 8x9 atlas of 192x208 frames"
    );
    assert_eq!(rgba, atlas(), "the decoded pixels are the frames");
    let pixel = |frame: usize, grid_x: usize, grid_y: usize| -> Check<[u8; 4]> {
        let x_pixel = frame.wrapping_mul(192).wrapping_add(grid_x.wrapping_mul(8));
        let y_pixel = grid_y.wrapping_mul(8).wrapping_add(8);
        let start = y_pixel
            .wrapping_mul(ATLAS_WIDTH)
            .wrapping_add(x_pixel)
            .wrapping_mul(4);
        let cell = rgba.get(start..start.wrapping_add(4)).ok_or("outside")?;
        Ok(cell.try_into()?)
    };
    assert_eq!(
        pixel(0, 7, 12)?,
        [244, 239, 226, 255],
        "an open eye's catchlight"
    );
    assert_eq!(
        pixel(5, 7, 12)?,
        [42, 38, 32, 255],
        "a blink closes the eye"
    );
    assert_eq!(pixel(7, 7, 12)?, [244, 239, 226, 255], "the loop ends open");
    let first_row_end = 208_usize.wrapping_mul(ATLAS_WIDTH).wrapping_mul(4);
    assert!(
        rgba.get(first_row_end..)
            .is_some_and(|rest| rest.iter().all(|&byte| byte == 0)),
        "the rows past the idle loop are transparent"
    );
    assert!(
        file.len() < 64 * 1024,
        "the sheet compresses: {} bytes",
        file.len()
    );
    // libwebp's `dwebp -pam` of `larry mascot --pet` output decoded to 11,501,568 RGBA bytes
    // with this BLAKE3 digest (2026-11-07), so the frames are what the reference decoder shows.
    assert_eq!(
        lgwks_std::hash::blake3(&rgba).to_hex(),
        "ee19e612b5b225f7b42811e21172261277432abaf367428fedf50a6b82ad647c",
        "the pixels libwebp decoded"
    );
    Ok(())
}

#[test]
fn the_committed_svg_is_the_render() -> Check {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(is_current(root)?, "docs/larry-mascot.svg equals the render");
    let rendered = svg();
    assert_eq!(
        rendered.matches("<rect ").count(),
        144,
        "one rect per same-colour run"
    );
    assert_eq!(OPEN.len(), 24, "a 24-row grid");
    assert!(OPEN.iter().all(|line| line.len() == 24), "24 columns a row");
    assert!(
        OPEN.iter().all(|line| line.bytes().eq(line.bytes().rev())),
        "every row mirrors about the seam"
    );
    Ok(())
}

/// The arguments for a seeded request, and whether it is legal.
fn seeded_args(rng: &mut Seeded) -> Check<(Vec<String>, bool)> {
    let pool: [&[&str]; 6] = [
        &["--preview"],
        &["--check"],
        &["--pet", "out/pet"],
        &["--svg", "out/m.svg"],
        &["--root", "."],
        &["--bogus"],
    ];
    let mut args: Vec<String> = Vec::new();
    let mut seen = [false; 6];
    let mut legal = true;
    for _ in 0..rng.index(5)? {
        let pick = rng.index(pool.len())?;
        let flag = pool.get(pick).ok_or("pool")?;
        let slot = seen.get_mut(pick).ok_or("seen")?;
        legal &= !*slot && pick != 5;
        *slot = true;
        args.extend(flag.iter().map(|part| String::from(*part)));
    }
    let [preview, check, pet, written, root, _] = seen;
    legal &= (preview || check || pet || written) && (!root || check);
    Ok((args, legal))
}

/// `args` as the process hands them over.
fn to_os(args: &[String]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// Every seeded command line parses when legal, and is refused with the usage line naming what
/// was given when not: an unknown or repeated flag, a flag missing its value, no action, or
/// `--root` without `--check`.
#[test]
fn sim_command_lines_get_their_exact_refusal() -> Check {
    for seed in 0..SEEDS.wrapping_mul(4) {
        let mut rng = Seeded::from_seed(seed);
        let (args, legal) = seeded_args(&mut rng)?;
        let parsed = Request::parse(&to_os(&args));
        if legal {
            assert!(parsed.is_ok(), "seed {seed}: {args:?} is legal: {parsed:?}");
        } else {
            let message = parsed
                .map(|_| String::new())
                .map_err(|error| error.to_string());
            assert_eq!(
                message,
                Err(format!("{USAGE} (got {:?})", to_os(&args))),
                "seed {seed}: {args:?} is refused"
            );
        }
    }
    let missing = Request::parse(&to_os(&[String::from("--svg")]));
    assert!(
        matches!(missing, Err(MascotError::Usage(_))),
        "a flag without its value: {missing:?}"
    );
    Ok(())
}

#[test]
fn a_drifted_svg_fails_the_check_and_writes_replace_whole() -> Check {
    let root = Scratch::new("mascot")?;
    let svg_path = root.path().join("docs").join("larry-mascot.svg");
    let check = to_os(&[
        String::from("--check"),
        String::from("--root"),
        root.path().display().to_string(),
    ]);
    let mut out: Vec<u8> = Vec::new();
    let missing = run(&check, &mut out);
    assert!(
        matches!(missing, Err(MascotError::File { .. })),
        "a missing SVG is named: {missing:?}"
    );
    fs::create_dir_all(svg_path.parent().ok_or("parent")?)?;
    let write = to_os(&[String::from("--svg"), svg_path.display().to_string()]);
    run(&write, &mut out)?;
    run(&check, &mut out)?;
    let printed = String::from_utf8(out.clone())?;
    assert!(
        printed.ends_with("PASS: docs/larry-mascot.svg is current\n"),
        "the check passes on a fresh render: {printed}"
    );
    let mut drifted = svg().into_bytes();
    if let Some(byte) = drifted.get_mut(200) {
        *byte = b'0';
    }
    fs::write(&svg_path, &drifted)?;
    assert!(
        matches!(run(&check, &mut out), Err(MascotError::Stale)),
        "one changed byte is stale"
    );
    fs::write(&svg_path, vec![b' '; 64 * 1024 + 1])?;
    assert!(
        matches!(run(&check, &mut out), Err(MascotError::TooLarge(_))),
        "an oversize SVG is refused before comparing"
    );
    let pet = root.path().join("pet");
    let pet_args = to_os(&[String::from("--pet"), pet.display().to_string()]);
    run(&pet_args, &mut out)?;
    run(&pet_args, &mut out)?;
    assert_eq!(
        fs::read(pet.join("pet.json"))?,
        MANIFEST.as_bytes(),
        "the manifest"
    );
    assert_eq!(
        decode(&fs::read(pet.join("spritesheet.webp"))?)?.2,
        atlas(),
        "the written sheet decodes to the frames"
    );
    let mut names: Vec<String> = fs::read_dir(&pet)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    names.sort();
    assert_eq!(
        names,
        ["pet.json", "spritesheet.webp"],
        "a rewrite replaces both files and leaves no temporary file"
    );
    Ok(())
}

#[test]
fn the_preview_shows_open_then_blink_top_pixel_on_top() -> Check {
    let mut out: Vec<u8> = Vec::new();
    run(&to_os(&[String::from("--preview")]), &mut out)?;
    let printed = String::from_utf8(out)?;
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(lines.len(), 25, "twelve rows, a blank line, twelve rows");
    assert_eq!(
        lines.get(12),
        Some(&""),
        "the frames are separated by a blank line"
    );
    for (index, (open, blink)) in lines.iter().take(12).zip(lines.iter().skip(13)).enumerate() {
        let eyes = (5..=7).contains(&index);
        assert_eq!(open != blink, eyes, "row {index}: only the eye rows blink");
    }
    let catchlight = "\x1b[38;2;244;239;226;48;2;244;239;226m▀";
    assert!(
        lines.get(6).is_some_and(|line| line.contains(catchlight)),
        "the catchlight cell paints the top pixel as the foreground"
    );
    Ok(())
}
