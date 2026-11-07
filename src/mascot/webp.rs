//! A lossless WebP encoder (VP8L, RFC 9649) for the mascot's pet spritesheet. No estate crate
//! encodes WebP, and the image is a few flat colours in 8x8 blocks, so the encoder keeps to the
//! smallest subset that compresses it: no transforms, no colour cache, one prefix-code group,
//! literals plus backward references to the previous pixel and the pixel above. Every prefix code
//! is complete (Kraft sum one), as libwebp requires of a code with two or more symbols.
//! Source: RFC 9649 sections 3 to 5; RFC 1951 section 3.2.2 for canonical codes.

use std::fmt;
use std::num::TryFromIntError;

/// Largest width or height: the header stores each minus one in 14 bits.
const MAX_SIDE: u32 = 16_384;
/// Longest backward reference the length prefix codes reach.
const MAX_COPY: usize = 4096;
/// Shortest run sent as a backward reference instead of literals.
const MIN_COPY: usize = 3;
/// Literal symbols in the green alphabet; the length prefixes follow them.
const LITERALS: usize = 256;
/// The green alphabet: 256 literals and 24 length prefixes, no colour cache.
const GREEN_ALPHABET: usize = 280;
/// The red, blue and alpha alphabets.
const CHANNEL_ALPHABET: usize = 256;
/// The distance alphabet.
const DISTANCE_ALPHABET: usize = 40;
/// The code-length alphabet: lengths 0 to 15, then the repeat codes 16, 17 and 18.
const LENGTH_ALPHABET: usize = 19;
/// The order the code-length code's lengths are written in (RFC 9649 section 3.7.2.1.2).
const CODE_LENGTH_ORDER: [usize; LENGTH_ALPHABET] = [
    17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];
/// The distance code of the pixel above: plane entry (0, 1).
const ABOVE: usize = 1;
/// The distance code of the previous pixel: plane entry (1, 0).
const LEFT: usize = 2;
/// Longest prefix code a VP8L code length may give.
const MAX_LENGTH: u8 = 15;

/// Why an image was not encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebpError {
    /// The width or height is zero or past 16384.
    Size {
        /// The width given.
        width: u32,
        /// The height given.
        height: u32,
    },
    /// The buffer is not width x height x 4 RGBA bytes.
    Pixels {
        /// The bytes the size needs, when they are addressable.
        expected: Option<usize>,
        /// The bytes given.
        actual: usize,
    },
    /// The encoded image is past the RIFF size field.
    TooLarge,
    /// A value does not fit the field it is written to.
    Narrow(TryFromIntError),
    /// A symbol had no code, or a code length passed 15: the encoder broke its own contract.
    Uncoded(usize),
}

impl fmt::Display for WebpError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Size { width, height } => write!(
                out,
                "a {width}x{height} image is outside 1 to {MAX_SIDE} pixels a side"
            ),
            Self::Pixels {
                expected: Some(expected),
                actual,
            } => write!(
                out,
                "{actual} RGBA bytes for an image that needs {expected}"
            ),
            Self::Pixels {
                expected: None,
                actual,
            } => write!(out, "{actual} RGBA bytes for an image too large to address"),
            Self::TooLarge => out.write_str("the encoded image exceeds the RIFF size field"),
            Self::Narrow(ref cause) => write!(out, "a value does not fit its field: {cause}"),
            Self::Uncoded(symbol) => write!(out, "symbol {symbol} has no prefix code"),
        }
    }
}

impl std::error::Error for WebpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::Narrow(ref cause) => Some(cause),
            _ => None,
        }
    }
}

/// The bit stream, least significant bit first.
struct Bits {
    /// Whole bytes written.
    bytes: Vec<u8>,
    /// Bits not yet a whole byte, in the low `count` bits.
    pending: u64,
    /// How many bits `pending` holds, always under 8 between calls.
    count: u32,
}

impl Bits {
    /// Appends the low `width` bits of `value`; `width` is at most 32.
    fn put(&mut self, value: u32, width: u32) {
        let mask = 1_u64.wrapping_shl(width).wrapping_sub(1);
        let mask = if width >= 32 {
            u64::from(u32::MAX)
        } else {
            mask
        };
        self.pending |= (u64::from(value) & mask).wrapping_shl(self.count);
        self.count = self.count.wrapping_add(width);
        while self.count >= 8 {
            let [low, ..] = self.pending.to_le_bytes();
            self.bytes.push(low);
            self.pending = self.pending.wrapping_shr(8);
            self.count = self.count.wrapping_sub(8);
        }
    }

    /// Appends a symbol index known to fit in `width` bits.
    fn put_index(&mut self, value: usize, width: u32) -> Result<(), WebpError> {
        self.put(u32::try_from(value).map_err(WebpError::Narrow)?, width);
        Ok(())
    }

    /// The stream padded with zero bits to a whole byte.
    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            let [low, ..] = self.pending.to_le_bytes();
            self.bytes.push(low);
        }
        self.bytes
    }
}

/// One prefix code: each symbol's length and its code, bit-reversed for an LSB-first stream.
struct Code {
    /// Code length per symbol; zero means the symbol is absent.
    lengths: Vec<u8>,
    /// The reversed code per symbol.
    codes: Vec<u32>,
    /// Whether the code has one symbol and so is zero bits long.
    single: bool,
}

impl Code {
    /// A code over `lengths`, which must be complete or name one symbol.
    fn new(lengths: Vec<u8>, single: bool) -> Result<Self, WebpError> {
        Ok(Self {
            codes: canonical(&lengths)?,
            lengths,
            single,
        })
    }

    /// Writes `symbol`.
    fn put(&self, bits: &mut Bits, symbol: usize) -> Result<(), WebpError> {
        if self.single {
            return Ok(());
        }
        match (self.lengths.get(symbol), self.codes.get(symbol)) {
            (Some(&length), Some(&code)) if length > 0 => {
                bits.put(code, u32::from(length));
                Ok(())
            }
            _ => Err(WebpError::Uncoded(symbol)),
        }
    }
}

/// Adds one occurrence of `symbol` to `counts`.
fn bump(counts: &mut [u64], symbol: usize) -> Result<(), WebpError> {
    let slot = counts.get_mut(symbol).ok_or(WebpError::Uncoded(symbol))?;
    *slot = slot.saturating_add(1);
    Ok(())
}

/// The symbols of `counts` that occur, most frequent first, ties by symbol.
fn used(counts: &[u64]) -> Vec<usize> {
    let mut symbols: Vec<(usize, u64)> = counts
        .iter()
        .copied()
        .enumerate()
        .filter(|pair| pair.1 > 0)
        .collect();
    symbols.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    symbols.into_iter().map(|pair| pair.0).collect()
}

/// `symbols` with a second, unused symbol added when it holds only one, so the code built on it
/// is a complete two-symbol code.
fn at_least_two(mut symbols: Vec<usize>) -> Vec<usize> {
    if let [only] = *symbols.as_slice() {
        symbols.push(if only == 0 { 1 } else { 0 });
    }
    symbols
}

/// Complete code lengths for `symbols` (two or more, most frequent first) over `size` symbols:
/// with k = ceil(log2 n), the 2^k - n most frequent get k - 1 bits and the rest k, so the Kraft
/// sum is exactly one.
fn complete_lengths(symbols: &[usize], size: usize) -> Result<Vec<u8>, WebpError> {
    let mut bits = 0_u8;
    let mut reach = 1_usize;
    while reach < symbols.len() {
        bits = bits.saturating_add(1);
        reach = reach.saturating_mul(2);
    }
    if bits > MAX_LENGTH {
        return Err(WebpError::Uncoded(symbols.len()));
    }
    let short = reach.saturating_sub(symbols.len());
    let mut lengths = vec![0_u8; size];
    for (rank, &symbol) in symbols.iter().enumerate() {
        let slot = lengths.get_mut(symbol).ok_or(WebpError::Uncoded(symbol))?;
        *slot = if rank < short {
            bits.saturating_sub(1)
        } else {
            bits
        };
    }
    Ok(lengths)
}

/// The canonical codes of `lengths` (RFC 1951 section 3.2.2), each bit-reversed for the
/// LSB-first stream; an absent symbol's code is zero and never written.
fn canonical(lengths: &[u8]) -> Result<Vec<u32>, WebpError> {
    let mut per_length = [0_u32; 16];
    for &length in lengths.iter().filter(|&&length| length > 0) {
        let slot = per_length
            .get_mut(usize::from(length))
            .ok_or(WebpError::Uncoded(usize::from(length)))?;
        *slot = slot.wrapping_add(1);
    }
    let mut next = [0_u32; 16];
    let mut code = 0_u32;
    for length in 1..16_usize {
        code = code
            .wrapping_add(per_length[length.wrapping_sub(1)])
            .wrapping_shl(1);
        next[length] = code;
    }
    let mut codes = Vec::with_capacity(lengths.len());
    for &length in lengths {
        if length == 0 {
            codes.push(0);
            continue;
        }
        let slot = next
            .get_mut(usize::from(length))
            .ok_or(WebpError::Uncoded(usize::from(length)))?;
        let assigned = *slot;
        *slot = slot.wrapping_add(1);
        codes.push(
            assigned
                .reverse_bits()
                .wrapping_shr(32_u32.wrapping_sub(u32::from(length))),
        );
    }
    Ok(codes)
}

/// One token of a code-length sequence: a length 0 to 15, or a zero run (17, 18) with its
/// extra bits.
struct LengthToken {
    /// The code-length symbol.
    symbol: usize,
    /// The extra bits' value.
    extra: u32,
    /// How many extra bits follow the symbol.
    extra_bits: u32,
}

/// The tokens of `lengths`: each nonzero length as itself, zero runs of 11 to 138 as 18, of 3
/// to 10 as 17, shorter ones as literal zeros.
fn length_tokens(lengths: &[u8]) -> Result<Vec<LengthToken>, WebpError> {
    let mut tokens = Vec::new();
    let mut at = 0_usize;
    while let Some(&length) = lengths.get(at) {
        if length != 0 {
            tokens.push(LengthToken {
                symbol: usize::from(length),
                extra: 0,
                extra_bits: 0,
            });
            at = at.saturating_add(1);
            continue;
        }
        let run = lengths
            .iter()
            .skip(at)
            .take_while(|&&length| length == 0)
            .count();
        let mut left = run;
        while left >= 11 {
            let take = left.min(138);
            tokens.push(LengthToken {
                symbol: 18,
                extra: u32::try_from(take.saturating_sub(11)).map_err(WebpError::Narrow)?,
                extra_bits: 7,
            });
            left = left.saturating_sub(take);
        }
        if left >= 3 {
            tokens.push(LengthToken {
                symbol: 17,
                extra: u32::try_from(left.saturating_sub(3)).map_err(WebpError::Narrow)?,
                extra_bits: 3,
            });
            left = 0;
        }
        for _ in 0..left {
            tokens.push(LengthToken {
                symbol: 0,
                extra: 0,
                extra_bits: 0,
            });
        }
        at = at.saturating_add(run);
    }
    Ok(tokens)
}

/// Writes a normal prefix code's lengths: the code-length code, then every length through it.
fn write_lengths(bits: &mut Bits, lengths: &[u8]) -> Result<(), WebpError> {
    let tokens = length_tokens(lengths)?;
    let mut counts = [0_u64; LENGTH_ALPHABET];
    for token in &tokens {
        bump(&mut counts, token.symbol)?;
    }
    let code_lengths = complete_lengths(&at_least_two(used(&counts)), LENGTH_ALPHABET)?;
    let last = CODE_LENGTH_ORDER
        .iter()
        .rposition(|&symbol| code_lengths.get(symbol).is_some_and(|&length| length > 0));
    let written = match last {
        Some(last) => last.saturating_add(1).max(4),
        None => 4,
    };
    bits.put_index(written.saturating_sub(4), 4)?;
    for &symbol in CODE_LENGTH_ORDER.iter().take(written) {
        let length = code_lengths.get(symbol).ok_or(WebpError::Uncoded(symbol))?;
        bits.put(u32::from(*length), 3);
    }
    bits.put(0, 1);
    let code = Code::new(code_lengths, false)?;
    for token in tokens {
        code.put(bits, token.symbol)?;
        bits.put(token.extra, token.extra_bits);
    }
    Ok(())
}

/// Writes the prefix code for `counts` and returns it: a simple code when at most two symbols
/// occur and both are under 256, else a normal code.
fn write_code(bits: &mut Bits, counts: &[u64]) -> Result<Code, WebpError> {
    let symbols = used(counts);
    let mut sorted = symbols.clone();
    sorted.sort_unstable();
    let simple = match *sorted.as_slice() {
        [] => Some((0, None)),
        [only] if only < LITERALS => Some((only, None)),
        [low, high] if high < LITERALS => Some((low, Some(high))),
        _ => None,
    };
    if let Some((first, second)) = simple {
        bits.put(1, 1);
        bits.put(u32::from(second.is_some()), 1);
        let wide = first > 1;
        bits.put(u32::from(wide), 1);
        bits.put_index(first, if wide { 8 } else { 1 })?;
        let mut lengths = vec![0_u8; counts.len()];
        if let Some(high) = second {
            bits.put_index(high, 8)?;
            for symbol in [first, high] {
                *lengths.get_mut(symbol).ok_or(WebpError::Uncoded(symbol))? = 1;
            }
        }
        return Code::new(lengths, second.is_none());
    }
    let lengths = complete_lengths(&at_least_two(symbols), counts.len())?;
    bits.put(0, 1);
    write_lengths(bits, &lengths)?;
    Code::new(lengths, false)
}

/// The prefix symbol, extra-bit count and extra bits of `value`, 1 or more (RFC 9649 5.2.2).
fn prefix(value: usize) -> Result<(usize, u32, u32), WebpError> {
    let offset = value.checked_sub(1).ok_or(WebpError::Uncoded(value))?;
    if offset < 4 {
        return Ok((offset, 0, 0));
    }
    let high = usize::BITS
        .wrapping_sub(1)
        .wrapping_sub(offset.leading_zeros());
    let extra_bits = high.wrapping_sub(1);
    let second = offset.wrapping_shr(extra_bits) & 1;
    let symbol = usize::try_from(high)
        .map_err(WebpError::Narrow)?
        .wrapping_mul(2)
        .wrapping_add(second);
    let mask = 1_usize.wrapping_shl(extra_bits).wrapping_sub(1);
    let extra = u32::try_from(offset & mask).map_err(WebpError::Narrow)?;
    Ok((symbol, extra_bits, extra))
}

/// One element of the pixel stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Element {
    /// An RGBA pixel sent as itself.
    Literal([u8; 4]),
    /// A run copied from `distance` code [`ABOVE`] or [`LEFT`].
    Copy {
        /// Pixels copied, 3 to 4096.
        length: usize,
        /// The distance code.
        distance: usize,
    },
}

/// How many pixels from `at` equal the pixel `back` before each, up to [`MAX_COPY`].
fn run(pixels: &[[u8; 4]], at: usize, back: usize) -> usize {
    let mut length = 0_usize;
    while length < MAX_COPY {
        let here = at.saturating_add(length);
        let earlier = here.checked_sub(back).and_then(|from| pixels.get(from));
        match (pixels.get(here), earlier) {
            (Some(pixel), Some(source)) if pixel == source => length = length.saturating_add(1),
            _ => break,
        }
    }
    length
}

/// The pixel stream: at each pixel the longer run from above or from the left when it reaches
/// [`MIN_COPY`], else a literal.
fn elements(pixels: &[[u8; 4]], width: usize) -> Vec<Element> {
    let mut stream = Vec::new();
    let mut at = 0_usize;
    while let Some(&pixel) = pixels.get(at) {
        let above = if at >= width {
            run(pixels, at, width)
        } else {
            0
        };
        let left = if at >= 1 { run(pixels, at, 1) } else { 0 };
        let (length, distance) = if above >= left {
            (above, ABOVE)
        } else {
            (left, LEFT)
        };
        if length >= MIN_COPY {
            stream.push(Element::Copy { length, distance });
            at = at.saturating_add(length);
        } else {
            stream.push(Element::Literal(pixel));
            at = at.saturating_add(1);
        }
    }
    stream
}

/// Encodes `rgba` (row-major, 4 bytes a pixel) of `width` x `height` as a lossless WebP file.
pub(crate) fn encode(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, WebpError> {
    if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
        return Err(WebpError::Size { width, height });
    }
    let columns = usize::try_from(width).map_err(WebpError::Narrow)?;
    let rows = usize::try_from(height).map_err(WebpError::Narrow)?;
    let expected = columns
        .checked_mul(rows)
        .and_then(|count| count.checked_mul(4));
    if expected != Some(rgba.len()) {
        return Err(WebpError::Pixels {
            expected,
            actual: rgba.len(),
        });
    }
    let (pixels, _) = rgba.as_chunks::<4>();
    let stream = elements(pixels, columns);
    let mut green = vec![0_u64; GREEN_ALPHABET];
    let mut red = vec![0_u64; CHANNEL_ALPHABET];
    let mut blue = vec![0_u64; CHANNEL_ALPHABET];
    let mut alpha = vec![0_u64; CHANNEL_ALPHABET];
    let mut distances = vec![0_u64; DISTANCE_ALPHABET];
    for element in &stream {
        match *element {
            Element::Literal([r_value, g_value, b_value, a_value]) => {
                bump(&mut green, usize::from(g_value))?;
                bump(&mut red, usize::from(r_value))?;
                bump(&mut blue, usize::from(b_value))?;
                bump(&mut alpha, usize::from(a_value))?;
            }
            Element::Copy { length, distance } => {
                bump(&mut green, LITERALS.saturating_add(prefix(length)?.0))?;
                bump(&mut distances, prefix(distance)?.0)?;
            }
        }
    }
    let mut bits = Bits {
        bytes: Vec::new(),
        pending: 0,
        count: 0,
    };
    bits.put(0x2f, 8);
    bits.put(width.wrapping_sub(1), 14);
    bits.put(height.wrapping_sub(1), 14);
    bits.put(1, 1);
    bits.put(0, 3);
    bits.put(0, 1);
    bits.put(0, 1);
    bits.put(0, 1);
    let green = write_code(&mut bits, &green)?;
    let red = write_code(&mut bits, &red)?;
    let blue = write_code(&mut bits, &blue)?;
    let alpha = write_code(&mut bits, &alpha)?;
    let distances = write_code(&mut bits, &distances)?;
    for element in stream {
        match element {
            Element::Literal([r_value, g_value, b_value, a_value]) => {
                green.put(&mut bits, usize::from(g_value))?;
                red.put(&mut bits, usize::from(r_value))?;
                blue.put(&mut bits, usize::from(b_value))?;
                alpha.put(&mut bits, usize::from(a_value))?;
            }
            Element::Copy { length, distance } => {
                let (symbol, extra_bits, extra) = prefix(length)?;
                green.put(&mut bits, LITERALS.saturating_add(symbol))?;
                bits.put(extra, extra_bits);
                let (symbol, extra_bits, extra) = prefix(distance)?;
                distances.put(&mut bits, symbol)?;
                bits.put(extra, extra_bits);
            }
        }
    }
    riff(&bits.finish())
}

/// `payload` as a RIFF WebP file with one `VP8L` chunk, padded to an even length.
fn riff(payload: &[u8]) -> Result<Vec<u8>, WebpError> {
    let chunk = u32::try_from(payload.len()).map_err(WebpError::Narrow)?;
    let pad = chunk & 1;
    let size = chunk
        .checked_add(12)
        .and_then(|size| size.checked_add(pad))
        .ok_or(WebpError::TooLarge)?;
    let mut file = Vec::with_capacity(payload.len().saturating_add(21));
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&size.to_le_bytes());
    file.extend_from_slice(b"WEBPVP8L");
    file.extend_from_slice(&chunk.to_le_bytes());
    file.extend_from_slice(payload);
    if pad == 1 {
        file.push(0);
    }
    Ok(file)
}
