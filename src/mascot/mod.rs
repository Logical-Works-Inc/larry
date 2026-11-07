//! `larry mascot`: the 24x24 pixel raccoon is defined here once, and every rendering of it is
//! drawn from that grid. These are the native view's half-block hero, the base SVG
//! (`docs/larry-mascot.svg`), the truecolor terminal preview and the pet package (`pet.json`
//! plus a lossless WebP spritesheet). Ports `tools/rocco_pixel.py` (larry#152, ADR 0014).
//! Identity, from the canonical larry-flat.webp (Director 2026-09-08): paired dark-interior ears,
//! a bandit mask, large brown eyes with cream catchlights, a dark nose, an open smile with tongue.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Read as _, Write};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod sim;
mod webp;

use webp::WebpError;

/// The open-eyed grid, mirrored about the seam between columns 11 and 12. `.` is transparent;
/// `d` dark outline, mask, nose and mouth; `g` gray fur; `c` cream; `b` brown eye; `t` tongue.
pub(crate) const OPEN: [&str; 24] = [
    "........................",
    "...dd..............dd...",
    "..dddd............dddd..",
    "..dddd............dddd..",
    "..dggd............dggd..",
    "..dggd............dggd..",
    "..dgg..............ggd..",
    "..dgggg..........ggggd..",
    "..dggggggggggggggggggd..",
    ".dggggggggggggggggggggd.",
    ".dggdddddddggdddddddggd.",
    ".dggddbbbbdggdbbbbddggd.",
    ".dggddbcbbdggdbbcbddggd.",
    ".dggddbcbbdggdbbcbddggd.",
    ".dggddbbbbdggdbbbbddggd.",
    ".dggdddddddggdddddddggd.",
    ".dggggggccccccccggggggd.",
    ".dgggggccccccccccgggggd.",
    ".dggggccccddddccccggggd.",
    ".dggggccccddddccccggggd.",
    ".dgggggccccccccccgggggd.",
    ".dgggggcddddddddcgggggd.",
    ".dgggggcdttttttdcgggggd.",
    "...dddddddddddddddddd...",
];
/// The rows the blink closes: the eyes become the mask row above them.
const BLINK_ROWS: std::ops::RangeInclusive<usize> = 11..=14;
/// The mask row a closed eye shows.
const CLOSED_ROW: usize = 10;
/// Grid side, in pixels.
const SIDE: usize = 24;
/// Pet frame width, in pixels.
const FRAME_WIDTH: usize = 192;
/// Pixels per grid pixel in a pet frame.
const BLOCK: usize = 8;
/// The frame's top margin, in pixels.
const FRAME_TOP: usize = 8;
/// Atlas width: 8 frames of 192 pixels.
const ATLAS_WIDTH: usize = 1536;
/// Atlas height: 9 rows of 208-pixel frames; the first row holds the idle loop.
const ATLAS_HEIGHT: usize = 1872;
/// The idle loop: five open frames, two blinks, one open.
const LOOP: [bool; 8] = [false, false, false, false, false, true, true, false];
/// The committed base SVG, relative to the repository root.
const SVG_PATH: [&str; 2] = ["docs", "larry-mascot.svg"];
/// Largest committed SVG read by `--check`, in bytes.
const MAX_SVG_BYTES: u64 = 64 * 1024;
/// The pet manifest, as `json.dumps(manifest, indent=2)` printed it, renamed per ADR 0005.
const MANIFEST: &str = "{\n  \"id\": \"larry\",\n  \"displayName\": \"Larry\",\n  \"description\": \"Logical Works' Toronto raccoon. Warm, intelligent, calm, concrete. Blocky 24x24 translation of the canonical larry-flat.webp.\",\n  \"spritesheetPath\": \"spritesheet.webp\"\n}\n";
/// Every flag `larry mascot` reads.
const USAGE: &str =
    "usage: larry mascot [--preview] [--pet DIR] [--svg PATH] [--check [--root DIR]]";

/// Why `larry mascot` stopped; each variant keeps its cause.
#[derive(Debug)]
#[non_exhaustive]
pub enum MascotError {
    /// The arguments were not ones this command reads.
    Usage(Vec<OsString>),
    /// No root was given and the working directory is unreadable.
    NoDirectory(io::Error),
    /// A file could not be read or written.
    File {
        /// The file.
        path: PathBuf,
        /// Why.
        cause: io::Error,
    },
    /// The committed SVG is past [`MAX_SVG_BYTES`].
    TooLarge(PathBuf),
    /// `--check` found the committed SVG differs from the render.
    Stale,
    /// The spritesheet could not be encoded; the encoder's error is kept as the cause.
    Encode(Box<dyn std::error::Error + Send + Sync>),
    /// Standard output could not be written.
    Output(io::Error),
}

impl fmt::Display for MascotError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Usage(ref given) => write!(out, "{USAGE} (got {given:?})"),
            Self::NoDirectory(ref cause) => write!(out, "mascot: no working directory: {cause}"),
            Self::File {
                ref path,
                ref cause,
            } => write!(out, "mascot: {}: {cause}", path.display()),
            Self::TooLarge(ref path) => write!(
                out,
                "mascot: {} exceeds {MAX_SVG_BYTES} bytes",
                path.display()
            ),
            Self::Stale => out.write_str(
                "FAIL: docs/larry-mascot.svg is stale — run `larry mascot --svg docs/larry-mascot.svg`",
            ),
            Self::Encode(ref cause) => write!(out, "mascot: spritesheet: {cause}"),
            Self::Output(ref cause) => write!(out, "mascot: stdout unwritable: {cause}"),
        }
    }
}

impl std::error::Error for MascotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Self::NoDirectory(ref cause)
            | Self::Output(ref cause)
            | Self::File { ref cause, .. } => Some(cause),
            Self::Encode(ref cause) => Some(&**cause),
            _ => None,
        }
    }
}

/// The parsed command line.
#[derive(Debug, Default, PartialEq, Eq)]
struct Request {
    /// Print the open and blink previews.
    preview: bool,
    /// Write the pet package into this directory.
    pet: Option<PathBuf>,
    /// Write the base SVG to this file.
    svg: Option<PathBuf>,
    /// Refuse a committed SVG that differs from the render.
    check: bool,
    /// The repository root `--check` reads, else the working directory.
    root: Option<PathBuf>,
}

impl Request {
    /// Parses the flags after `mascot`: each at most once, at least one action, `--root` only
    /// with `--check`.
    fn parse(args: &[OsString]) -> Result<Self, MascotError> {
        let usage = || MascotError::Usage(args.to_vec());
        let mut request = Self::default();
        let mut items = args.iter();
        while let Some(item) = items.next() {
            let mut value = || items.next().map(PathBuf::from).ok_or_else(usage);
            match item.to_str() {
                Some("--preview") if !request.preview => request.preview = true,
                Some("--check") if !request.check => request.check = true,
                Some("--pet") if request.pet.is_none() => request.pet = Some(value()?),
                Some("--svg") if request.svg.is_none() => request.svg = Some(value()?),
                Some("--root") if request.root.is_none() => request.root = Some(value()?),
                _ => return Err(usage()),
            }
        }
        let acts =
            request.preview || request.check || request.pet.is_some() || request.svg.is_some();
        if !acts || (request.root.is_some() && !request.check) {
            return Err(usage());
        }
        Ok(request)
    }
}

/// The grid row `index` of the open or blinking frame; past the grid, an empty row.
fn row(blink: bool, index: usize) -> &'static str {
    let source = if blink && BLINK_ROWS.contains(&index) {
        CLOSED_ROW
    } else {
        index
    };
    match OPEN.get(source) {
        Some(line) => line,
        None => "",
    }
}

/// The brand colour of a grid pixel; `None` is transparent.
const fn rgb(pixel: u8) -> Option<[u8; 3]> {
    match pixel {
        b'd' => Some([42, 38, 32]),
        b'g' => Some([122, 115, 115]),
        b'c' => Some([244, 239, 226]),
        b'b' => Some([128, 96, 60]),
        b't' => Some([201, 88, 74]),
        _ => None,
    }
}

/// The base SVG: one rect per same-colour run, one unit a pixel, one unit of padding.
fn svg() -> String {
    let mut runs: Vec<String> = Vec::new();
    for (y_index, line) in OPEN.iter().enumerate() {
        let bytes = line.as_bytes();
        let mut start = 0_usize;
        while let Some(&pixel) = bytes.get(start) {
            let width = bytes
                .iter()
                .skip(start)
                .take_while(|&&other| other == pixel)
                .count();
            if let Some([red, green, blue]) = rgb(pixel) {
                runs.push(format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{width}\" height=\"1\" fill=\"#{red:02X}{green:02X}{blue:02X}\"/>",
                    start.saturating_add(1),
                    y_index.saturating_add(1),
                ));
            }
            start = start.saturating_add(width);
        }
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 26 26\" width=\"260\" height=\"260\" shape-rendering=\"crispEdges\">\n  {}\n</svg>\n",
        runs.join("\n  ")
    )
}

/// The truecolor half-block preview of one frame: the top pixel in the upper half (foreground),
/// the bottom pixel in the lower half (background), as the native view paints it.
fn preview(blink: bool) -> String {
    let lines: Vec<String> = (0..SIDE)
        .step_by(2)
        .map(|top_index| {
            let top = row(blink, top_index).as_bytes();
            let bottom = row(blink, top_index.saturating_add(1)).as_bytes();
            top.iter()
                .zip(bottom)
                .map(|(&upper, &lower)| match (rgb(upper), rgb(lower)) {
                    (Some([tr, tg, tb]), Some([br, bg, bb])) => {
                        format!("\x1b[38;2;{tr};{tg};{tb};48;2;{br};{bg};{bb}m▀\x1b[0m")
                    }
                    (Some([tr, tg, tb]), None) => format!("\x1b[38;2;{tr};{tg};{tb}m▀\x1b[0m"),
                    (None, Some([br, bg, bb])) => format!("\x1b[38;2;{br};{bg};{bb}m▄\x1b[0m"),
                    (None, None) => String::from(" "),
                })
                .collect()
        })
        .collect();
    lines.join("\n")
}

/// The spritesheet's RGBA pixels: the idle loop in the atlas's first row, the rest transparent.
fn atlas() -> Vec<u8> {
    let mut pixels = vec![0_u8; ATLAS_WIDTH.saturating_mul(ATLAS_HEIGHT).saturating_mul(4)];
    for (frame, &blink) in LOOP.iter().enumerate() {
        let left = frame.saturating_mul(FRAME_WIDTH);
        for grid_y in 0..SIDE {
            for (grid_x, &pixel) in row(blink, grid_y).as_bytes().iter().enumerate() {
                let Some([red, green, blue]) = rgb(pixel) else {
                    continue;
                };
                for block_y in 0..BLOCK {
                    let y_pixel = FRAME_TOP
                        .saturating_add(grid_y.saturating_mul(BLOCK))
                        .saturating_add(block_y);
                    let x_pixel = left.saturating_add(grid_x.saturating_mul(BLOCK));
                    let start = y_pixel
                        .saturating_mul(ATLAS_WIDTH)
                        .saturating_add(x_pixel)
                        .saturating_mul(4);
                    let end = start.saturating_add(BLOCK.saturating_mul(4));
                    if let Some(span) = pixels.get_mut(start..end) {
                        for cell in span.as_chunks_mut::<4>().0 {
                            *cell = [red, green, blue, 255];
                        }
                    }
                }
            }
        }
    }
    pixels
}

/// The spritesheet as a lossless WebP file.
fn spritesheet() -> Result<Vec<u8>, WebpError> {
    let width = u32::try_from(ATLAS_WIDTH).map_err(WebpError::Narrow)?;
    let height = u32::try_from(ATLAS_HEIGHT).map_err(WebpError::Narrow)?;
    webp::encode(&atlas(), width, height)
}

/// Replaces `directory/name` with `bytes`, whole or not at all.
fn replace(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), MascotError> {
    crate::replace_file(directory, name, bytes)
        .map_err(|(path, cause)| MascotError::File { path, cause })
}

/// Writes `pet.json` and `spritesheet.webp` into `directory`, creating it.
fn write_pet(directory: &Path) -> Result<(), MascotError> {
    let sheet = spritesheet().map_err(|cause| MascotError::Encode(Box::new(cause)))?;
    fs::create_dir_all(directory).map_err(|cause| MascotError::File {
        path: directory.to_path_buf(),
        cause,
    })?;
    replace(directory, "spritesheet.webp", &sheet)?;
    replace(directory, "pet.json", MANIFEST.as_bytes())
}

/// Writes the base SVG to `path`.
fn write_svg(path: &Path) -> Result<(), MascotError> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(MascotError::File {
            path: path.to_path_buf(),
            cause: io::Error::new(io::ErrorKind::InvalidInput, "not a UTF-8 file name"),
        });
    };
    replace(parent, name, svg().as_bytes())
}

/// Whether `root/docs/larry-mascot.svg` is the render.
fn is_current(root: &Path) -> Result<bool, MascotError> {
    let path = SVG_PATH
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part));
    let file_error = |cause| MascotError::File {
        path: path.clone(),
        cause,
    };
    let file = fs::File::open(&path).map_err(file_error)?;
    let mut bytes = Vec::new();
    file.take(MAX_SVG_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(file_error)?;
    if bytes.len()
        > usize::try_from(MAX_SVG_BYTES).map_err(|cause| MascotError::File {
            path: path.clone(),
            cause: io::Error::other(cause),
        })?
    {
        return Err(MascotError::TooLarge(path));
    }
    Ok(bytes == svg().as_bytes())
}

/// Writes `lines` to `out`, each with a newline.
fn say<W: Write>(out: &mut W, lines: &[&str]) -> Result<(), MascotError> {
    for line in lines {
        out.write_all(line.as_bytes())
            .and_then(|()| out.write_all(b"\n"))
            .map_err(MascotError::Output)?;
    }
    out.flush().map_err(MascotError::Output)
}

/// `larry mascot`: runs each requested action in order (preview, pet, SVG, check) and reports
/// each on `out`, as the Python tool printed them.
///
/// # Errors
/// Returns a [`MascotError`] on a bad command line, a failed write, an unencodable sheet or a
/// stale committed SVG.
pub fn run<W: Write>(args: &[OsString], out: &mut W) -> Result<(), MascotError> {
    let request = Request::parse(args)?;
    if request.preview {
        say(out, &[&preview(false), "", &preview(true)])?;
    }
    if let Some(ref directory) = request.pet {
        write_pet(directory)?;
        say(
            out,
            &[&format!(
                "wrote {}/pet.json + spritesheet.webp",
                directory.display()
            )],
        )?;
    }
    if let Some(ref path) = request.svg {
        write_svg(path)?;
        say(out, &[&format!("wrote {}", path.display())])?;
    }
    if request.check {
        let root = match request.root {
            Some(root) => root,
            None => std::env::current_dir().map_err(MascotError::NoDirectory)?,
        };
        if !is_current(&root)? {
            return Err(MascotError::Stale);
        }
        say(out, &["PASS: docs/larry-mascot.svg is current"])?;
    }
    Ok(())
}
