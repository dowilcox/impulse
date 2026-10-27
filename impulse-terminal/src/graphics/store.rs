//! A terminal's images and where they're placed.
//!
//! Direct placements sit beside the grid, anchored to the row they were
//! placed on: an absolute row (`row_base` + history + screen line, as
//! command blocks count them) on the primary screen, a screen line on the
//! alternate one. Virtual placements only give an image's size in cells;
//! placeholder cells in the grid say where it shows.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;

use super::command::Command;
use super::decode::{self, Decoder, Job, Pixels};

/// Widest and tallest image, in pixels.
const MAX_SIDE: u32 = 10_000;
/// Largest image, decoded (4 bytes a pixel): 4096 × 4096.
pub(crate) const MAX_IMAGE_BYTES: usize = 64 << 20;
/// Largest transmission, assembled from its chunks (or read from a file).
const MAX_TRANSMISSION_BYTES: usize = 64 << 20;
/// The most a terminal keeps, decoded; the oldest unplaced images go first.
const QUOTA_BYTES: usize = 256 << 20;
/// The most placements a terminal keeps (the oldest go).
const MAX_PLACEMENTS: usize = 4096;
/// The most cells one placement spans either way.
const MAX_CELLS: u32 = 2_000;

/// An image, decoded.
#[derive(Clone)]
pub struct TerminalImage {
    pub id: u32,
    /// Changes when the id is given new data.
    pub serial: u64,
    pub width: u32,
    pub height: u32,
    /// RGBA, row-major.
    pub rgba: Arc<[u8]>,
}

/// A direct placement, in viewport rows and cells.
#[derive(Clone, Debug, PartialEq)]
pub struct ImagePlacement {
    pub image_id: u32,
    /// The viewport row of its top; negative when that's scrolled above.
    pub row: i32,
    pub col: usize,
    /// Its size, in cells (not whole ones: an image at its own size).
    pub width: f32,
    pub height: f32,
    /// Where it starts in its first cell, in cells.
    pub offset_x: f32,
    pub offset_y: f32,
    /// The part of the image shown: x, y, width and height in pixels.
    pub source: [u32; 4],
    /// Under the text when negative.
    pub z: i32,
}

/// What the viewport shows: the direct placements in it, in drawing
/// order, and every virtual placement's size in cells.
#[derive(Clone, Debug, Default)]
pub struct ImageLayer {
    pub placements: Vec<ImagePlacement>,
    /// By image and placement id: columns and rows.
    pub virtuals: HashMap<(u32, u32), (u32, u32)>,
    /// Each image's size in pixels, for placeholders without a virtual
    /// placement of their own.
    pub sizes: HashMap<u32, (u32, u32)>,
}

impl ImageLayer {
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty() && self.virtuals.is_empty()
    }
}

/// Where a command arrived: the cursor and the screen around it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cursor {
    pub alt: bool,
    /// The anchor row of screen line 0.
    pub top: i64,
    pub line: usize,
    pub col: usize,
    pub columns: usize,
    pub lines: usize,
    /// A cell's size in pixels.
    pub cell: (u32, u32),
}

/// What a command did.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Outcome {
    /// The answer to write back to the program.
    pub reply: Option<String>,
    /// Lines to move the cursor down (scrolling at the bottom), then the
    /// column to put it in: past an image just placed.
    pub cursor_move: Option<(usize, usize)>,
    /// Something to draw changed.
    pub changed: bool,
}

struct Image {
    number: u32,
    serial: u64,
    width: u32,
    height: u32,
    pixels: Pixels,
    bytes: usize,
}

#[derive(Clone, Debug)]
struct Placement {
    image_id: u32,
    placement_id: u32,
    serial: u64,
    alt: bool,
    row: i64,
    col: usize,
    cols: usize,
    rows: usize,
    width: f32,
    height: f32,
    offset_x: f32,
    offset_y: f32,
    source: [u32; 4],
    z: i32,
}

impl Placement {
    fn covers(&self, alt: bool, row: i64, col: usize) -> bool {
        self.alt == alt
            && (self.row..self.row + self.rows as i64).contains(&row)
            && (self.col..self.col + self.cols).contains(&col)
    }

    fn crosses_rows(&self, alt: bool, top: i64, bottom: i64) -> bool {
        self.alt == alt && self.row < bottom && self.row + self.rows as i64 > top
    }
}

/// A chunked transmission so far.
struct Pending {
    cmd: Command,
    data: Vec<u8>,
    /// Base64 left over from the last chunk (not a multiple of 4).
    tail: Vec<u8>,
    failed: Option<&'static str>,
}

pub(crate) struct ImageStore {
    images: HashMap<u32, Image>,
    placements: Vec<Placement>,
    virtuals: HashMap<(u32, u32), (u32, u32)>,
    pending: Option<Pending>,
    serial: u64,
    /// The next id to try for an image the program didn't name.
    next_id: u32,
    row_base: i64,
    bytes: usize,
    decoder: Decoder,
}

/// Base64 as kitty reads it: padding optional, and the unused bits of a
/// last character not checked (chafa leaves them set).
const BASE64: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    base64::engine::GeneralPurposeConfig::new()
        .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

impl ImageStore {
    /// `notify` runs (on the decoding thread) when an image finishes
    /// decoding.
    pub fn new(notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        ImageStore {
            images: HashMap::new(),
            placements: Vec::new(),
            virtuals: HashMap::new(),
            pending: None,
            serial: 0,
            next_id: 1 << 31,
            row_base: 0,
            bytes: 0,
            decoder: Decoder::new(notify),
        }
    }

    pub fn row_base(&self) -> i64 {
        self.row_base
    }

    /// Whether anything is placed on the given screen (so clearing it has
    /// something to do).
    pub fn has_placements(&self, alt: bool) -> bool {
        self.placements.iter().any(|p| p.alt == alt)
    }

    fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    // MARK: Commands

    pub fn handle(&mut self, cmd: Command, payload: &[u8], at: &Cursor) -> Outcome {
        if cmd.continuation {
            if let Some(mut pending) = self.pending.take() {
                pending.add(payload);
                if cmd.more {
                    self.pending = Some(pending);
                    return Outcome::default();
                }
                let mut first = pending.cmd;
                if cmd.quiet != 0 {
                    first.quiet = cmd.quiet;
                }
                return match pending.failed {
                    Some(error) => Outcome {
                        reply: reply(&first, first.id, Err(error)),
                        ..Default::default()
                    },
                    None => self.transmit(first, pending.data, at),
                };
            }
        }
        // Anything else ends a transmission left unfinished.
        self.pending = None;
        match cmd.action {
            b't' | b'T' | b'q' => {
                if cmd.more {
                    let mut pending = Pending {
                        cmd,
                        data: Vec::new(),
                        tail: Vec::new(),
                        failed: None,
                    };
                    pending.add(payload);
                    self.pending = Some(pending);
                    return Outcome::default();
                }
                let data = match decode_base64(payload) {
                    Some(data) => data,
                    None => {
                        return Outcome {
                            reply: reply(&cmd, cmd.id, Err("EINVAL:bad base64 data")),
                            ..Default::default()
                        }
                    }
                };
                self.transmit(cmd, data, at)
            }
            b'p' => {
                let Some(id) = self.image_id(&cmd) else {
                    return Outcome {
                        reply: reply(&cmd, cmd.id, Err("ENOENT:no such image")),
                        ..Default::default()
                    };
                };
                self.place(&cmd, id, at)
            }
            b'd' => self.delete(&cmd, at),
            b'f' | b'a' | b'c' => Outcome {
                reply: reply(&cmd, cmd.id, Err("EINVAL:animation isn't supported")),
                ..Default::default()
            },
            _ => Outcome {
                reply: reply(&cmd, cmd.id, Err("EINVAL:unknown action")),
                ..Default::default()
            },
        }
    }

    /// The image a command names: by id, else the newest with its number.
    fn image_id(&self, cmd: &Command) -> Option<u32> {
        if cmd.id != 0 {
            return self.images.contains_key(&cmd.id).then_some(cmd.id);
        }
        if cmd.number != 0 {
            return self
                .images
                .iter()
                .filter(|(_, image)| image.number == cmd.number)
                .max_by_key(|(_, image)| image.serial)
                .map(|(id, _)| *id);
        }
        None
    }

    /// Transmit (and with `a=T` place, with `a=q` only check) an image.
    fn transmit(&mut self, cmd: Command, data: Vec<u8>, at: &Cursor) -> Outcome {
        let checked = match self.check(&cmd, data) {
            Ok(checked) => checked,
            Err(error) => {
                return Outcome {
                    reply: reply(&cmd, cmd.id, Err(error)),
                    ..Default::default()
                }
            }
        };
        if cmd.action == b'q' {
            return Outcome {
                reply: reply(&cmd, cmd.id, Ok(())),
                ..Default::default()
            };
        }
        let (data, compressed, width, height) = checked;
        let id = if cmd.id != 0 { cmd.id } else { self.free_id() };
        self.remove_image(id, false);
        let slot = Pixels::default();
        self.decoder.decode(Job {
            slot: Arc::clone(&slot),
            data,
            format: cmd.format,
            compressed,
            width,
            height,
        });
        let bytes = width as usize * height as usize * 4;
        let serial = self.next_serial();
        self.images.insert(
            id,
            Image {
                number: cmd.number,
                serial,
                width,
                height,
                pixels: slot,
                bytes,
            },
        );
        self.bytes += bytes;
        self.evict(id);
        if cmd.action == b'T' {
            let mut placed = self.place(&cmd, id, at);
            placed.changed = true;
            return placed;
        }
        Outcome {
            reply: reply(&cmd, id, Ok(())),
            changed: true,
            ..Default::default()
        }
    }

    /// The transmission's data, whether it's compressed, and the image's
    /// size; or why it can't be taken.
    fn check(
        &self,
        cmd: &Command,
        data: Vec<u8>,
    ) -> Result<(Vec<u8>, bool, u32, u32), &'static str> {
        let data = match cmd.medium {
            b'd' => data,
            b't' => read_temporary_file(&data, cmd.offset, cmd.size)?,
            // A file the program names: `kitten icat` sends a PNG so. Any
            // output Impulse shows (`cat` of a file, a server over ssh) can
            // name one, so only PNGs are read and nothing is ever answered
            // (`reply`): at worst a PNG of yours shows on your screen, and
            // the sender learns nothing about your files.
            b'f' if cmd.format == 100 => read_named_png(&data, cmd.offset, cmd.size)?,
            _ => return Err("EINVAL:unsupported transmission medium"),
        };
        let compressed = match cmd.compression {
            0 | b'0' => false,
            b'z' => true,
            _ => return Err("EINVAL:unknown compression"),
        };
        let (data, compressed, width, height) = match cmd.format {
            100 => {
                let data = if compressed {
                    decode::inflate(&data, MAX_TRANSMISSION_BYTES).ok_or("EINVAL:bad zlib data")?
                } else {
                    data
                };
                let (width, height) = decode::png_size(&data).ok_or("EBADPNG:not a PNG")?;
                (data, false, width, height)
            }
            24 | 32 => {
                if cmd.width == 0 || cmd.height == 0 {
                    return Err("EINVAL:the width and height are missing");
                }
                let expected = cmd.width as usize * cmd.height as usize * (cmd.format as usize / 8);
                if !compressed && data.len() < expected {
                    return Err("ENODATA:insufficient image data");
                }
                (data, compressed, cmd.width, cmd.height)
            }
            _ => return Err("EINVAL:unknown format"),
        };
        if width == 0
            || height == 0
            || width > MAX_SIDE
            || height > MAX_SIDE
            || width as usize * height as usize * 4 > MAX_IMAGE_BYTES
        {
            return Err("EINVAL:image too large");
        }
        Ok((data, compressed, width, height))
    }

    /// An id no image has.
    fn free_id(&mut self) -> u32 {
        loop {
            let id = self.next_id;
            self.next_id = self.next_id.checked_add(1).unwrap_or(1 << 31);
            if !self.images.contains_key(&id) {
                return id;
            }
        }
    }

    /// Free images (the oldest unplaced first, then the oldest) until the
    /// store is within its quota, keeping `keep`.
    fn evict(&mut self, keep: u32) {
        while self.bytes > QUOTA_BYTES {
            let placed: std::collections::HashSet<u32> = self
                .placements
                .iter()
                .map(|p| p.image_id)
                .chain(self.virtuals.keys().map(|(id, _)| *id))
                .collect();
            let victim = self
                .images
                .iter()
                .filter(|(id, _)| **id != keep)
                .min_by_key(|(id, image)| (placed.contains(id), image.serial))
                .map(|(id, _)| *id);
            match victim {
                Some(id) => self.remove_image(id, true),
                None => break,
            }
        }
    }

    /// Forget an image and its direct placements (and its virtual ones
    /// too, with `virtuals`).
    fn remove_image(&mut self, id: u32, virtuals: bool) {
        if let Some(image) = self.images.remove(&id) {
            self.bytes = self.bytes.saturating_sub(image.bytes);
        }
        self.placements.retain(|p| p.image_id != id);
        if virtuals {
            self.virtuals.retain(|(image, _), _| *image != id);
        }
    }

    fn place(&mut self, cmd: &Command, id: u32, at: &Cursor) -> Outcome {
        let Some(image) = self.images.get(&id) else {
            return Outcome {
                reply: reply(cmd, id, Err("ENOENT:no such image")),
                ..Default::default()
            };
        };
        let (image_w, image_h) = (image.width, image.height);
        let x = cmd.x.min(image_w);
        let y = cmd.y.min(image_h);
        let w = if cmd.w == 0 {
            image_w - x
        } else {
            cmd.w.min(image_w - x)
        };
        let h = if cmd.h == 0 {
            image_h - y
        } else {
            cmd.h.min(image_h - y)
        };
        if w == 0 || h == 0 {
            return Outcome {
                reply: reply(cmd, id, Err("EINVAL:nothing of the image to show")),
                ..Default::default()
            };
        }
        let (cell_w, cell_h) = (at.cell.0.max(1) as f32, at.cell.1.max(1) as f32);
        let (w_f, h_f) = (w as f32, h as f32);
        // The size in cells: as asked, one side worked out from the
        // other keeping the image's shape, or its own size in pixels.
        let (width, height) = match (cmd.cols, cmd.rows) {
            (0, 0) => (w_f / cell_w, h_f / cell_h),
            (c, 0) => (c as f32, c as f32 * cell_w * h_f / w_f / cell_h),
            (0, r) => (r as f32 * cell_h * w_f / h_f / cell_w, r as f32),
            (c, r) => (c as f32, r as f32),
        };
        let (offset_x, offset_y) = if cmd.cols == 0 && cmd.rows == 0 {
            (
                cmd.cell_x.min(at.cell.0) as f32 / cell_w,
                cmd.cell_y.min(at.cell.1) as f32 / cell_h,
            )
        } else {
            (0.0, 0.0)
        };
        let span = |cells: f32| (cells.ceil() as u32).clamp(1, MAX_CELLS);
        let (cols, rows) = (span(offset_x + width), span(offset_y + height));
        if cmd.virtual_placement {
            self.virtuals.insert((id, cmd.placement), (cols, rows));
            return Outcome {
                reply: reply(cmd, id, Ok(())),
                changed: true,
                ..Default::default()
            };
        }
        if cmd.placement != 0 {
            self.placements
                .retain(|p| !(p.image_id == id && p.placement_id == cmd.placement));
        }
        let serial = self.next_serial();
        self.placements.push(Placement {
            image_id: id,
            placement_id: cmd.placement,
            serial,
            alt: at.alt,
            row: at.top + at.line as i64,
            col: at.col,
            cols: cols as usize,
            rows: rows as usize,
            width,
            height,
            offset_x,
            offset_y,
            source: [x, y, w, h],
            z: cmd.z,
        });
        if self.placements.len() > MAX_PLACEMENTS {
            let excess = self.placements.len() - MAX_PLACEMENTS;
            self.placements.drain(..excess);
        }
        // Past the image, as kitty moves it: right by its columns, down by
        // its rows less one, wrapping and scrolling as text would (kitty's
        // handle_put_command and screen_handle_graphics_command).
        let cursor_move = (!cmd.keep_cursor).then(|| {
            let mut col = at.col + cols as usize;
            let mut down = rows as usize - 1;
            if col >= at.columns {
                col = 0;
                down += 1;
            }
            (down, col)
        });
        Outcome {
            reply: reply(cmd, id, Ok(())),
            cursor_move,
            changed: true,
        }
    }

    fn delete(&mut self, cmd: &Command, at: &Cursor) -> Outcome {
        let target = if cmd.delete == 0 { b'a' } else { cmd.delete };
        let free = target.is_ascii_uppercase();
        let (top, bottom) = (at.top, at.top + at.lines as i64);
        // A cell named in the command: 1-based column and screen row.
        let cell_row = top + i64::from(cmd.y.max(1)) - 1;
        let cell_col = cmd.x.max(1) as usize - 1;
        let at_row = at.top + at.line as i64;
        // The images whose data may go with uppercase targets.
        let mut candidates: Vec<u32> = Vec::new();
        let before = (self.placements.len(), self.virtuals.len());
        let mut remove = |store: &mut Self, matches: &dyn Fn(&Placement) -> bool| {
            store.placements.retain(|p| {
                let gone = matches(p);
                if gone {
                    candidates.push(p.image_id);
                }
                !gone
            });
        };
        match target.to_ascii_lowercase() {
            b'a' => remove(self, &|p| p.crosses_rows(at.alt, top, bottom)),
            b'i' | b'n' => {
                let id = if target.eq_ignore_ascii_case(&b'i') {
                    Some(cmd.id)
                } else {
                    self.image_id(&Command {
                        id: 0,
                        ..cmd.clone()
                    })
                };
                if let Some(id) = id {
                    let placement = cmd.placement;
                    remove(self, &|p| {
                        p.image_id == id && (placement == 0 || p.placement_id == placement)
                    });
                    self.virtuals.retain(|(image, p), _| {
                        !(*image == id && (placement == 0 || *p == placement))
                    });
                    candidates.push(id);
                }
            }
            b'c' => remove(self, &|p| p.covers(at.alt, at_row, at.col)),
            b'p' => remove(self, &|p| p.covers(at.alt, cell_row, cell_col)),
            b'q' => remove(self, &|p| {
                p.covers(at.alt, cell_row, cell_col) && p.z == cmd.z
            }),
            b'x' => remove(self, &|p| {
                p.alt == at.alt && (p.col..p.col + p.cols).contains(&cell_col)
            }),
            b'y' => remove(self, &|p| p.crosses_rows(at.alt, cell_row, cell_row + 1)),
            b'z' => remove(self, &|p| p.alt == at.alt && p.z == cmd.z),
            b'r' => {
                let range = cmd.x..=cmd.y;
                remove(self, &|p| range.contains(&p.image_id));
                self.virtuals.retain(|(image, _), _| !range.contains(image));
                candidates.extend(self.images.keys().filter(|id| range.contains(id)));
            }
            _ => {}
        }
        let mut changed = before != (self.placements.len(), self.virtuals.len());
        if free {
            candidates.sort_unstable();
            candidates.dedup();
            for id in candidates {
                let referenced = self.placements.iter().any(|p| p.image_id == id)
                    || self.virtuals.keys().any(|(image, _)| *image == id);
                if !referenced && self.images.contains_key(&id) {
                    self.remove_image(id, true);
                    changed = true;
                }
            }
        }
        Outcome {
            changed,
            ..Default::default()
        }
    }

    // MARK: The screen changing under the images

    /// The screen was erased (ED 2): its placements go.
    pub fn erase_screen(&mut self, alt: bool, top: i64, lines: usize) -> bool {
        let before = self.placements.len();
        let bottom = top + lines as i64;
        self.placements
            .retain(|p| !p.crosses_rows(alt, top, bottom));
        before != self.placements.len()
    }

    /// The scrollback is about to be erased (ED 3), `history` lines of it:
    /// the placements wholly in it go, and the rest keep their lines.
    pub fn erase_scrollback(&mut self, top: i64, history: usize) -> bool {
        let before = self.placements.len();
        self.placements
            .retain(|p| p.alt || p.row + p.rows as i64 > top);
        self.row_base += history as i64;
        before != self.placements.len()
    }

    /// The alternate screen was entered or left: what was on it goes.
    pub fn clear_alt(&mut self) -> bool {
        let before = self.placements.len();
        self.placements.retain(|p| !p.alt);
        before != self.placements.len()
    }

    /// A full reset (RIS): everything goes.
    pub fn reset(&mut self) -> bool {
        let had =
            !self.images.is_empty() || !self.placements.is_empty() || !self.virtuals.is_empty();
        self.images.clear();
        self.placements.clear();
        self.virtuals.clear();
        self.pending = None;
        self.bytes = 0;
        had
    }

    /// `evicted` lines left the top of the scrollback.
    pub fn trim(&mut self, evicted: u64) -> bool {
        self.row_base += evicted.min(i64::MAX as u64) as i64;
        let before = self.placements.len();
        let first = self.row_base;
        self.placements
            .retain(|p| p.alt || p.row + p.rows as i64 > first);
        before != self.placements.len()
    }

    /// Move each primary-screen placement's row through `map`, which takes
    /// and gives rows counted from the top of the grid (a resize reflowed
    /// it).
    pub fn remap_rows(&mut self, mut map: impl FnMut(i64) -> i64) {
        let base = self.row_base;
        for p in self.placements.iter_mut().filter(|p| !p.alt) {
            p.row = base + map(p.row - base);
        }
    }

    // MARK: Drawing

    /// The placements on `alt`'s screen in the viewport, whose line 0 is
    /// anchor row `top` (shown `display_offset` rows scrolled back).
    pub fn layer(&self, alt: bool, top: i64, display_offset: usize, lines: usize) -> ImageLayer {
        let mut placements: Vec<&Placement> = self
            .placements
            .iter()
            .filter(|p| p.alt == alt && self.images.contains_key(&p.image_id))
            .filter(|p| {
                let row = p.row - top + display_offset as i64;
                row < lines as i64 && row + p.rows as i64 > 0
            })
            .collect();
        placements.sort_by_key(|p| (p.z, p.serial));
        ImageLayer {
            placements: placements
                .into_iter()
                .map(|p| ImagePlacement {
                    image_id: p.image_id,
                    row: (p.row - top + display_offset as i64)
                        .clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                    col: p.col,
                    width: p.width,
                    height: p.height,
                    offset_x: p.offset_x,
                    offset_y: p.offset_y,
                    source: p.source,
                    z: p.z,
                })
                .collect(),
            virtuals: self.virtuals.clone(),
            sizes: self
                .images
                .iter()
                .map(|(id, image)| (*id, (image.width, image.height)))
                .collect(),
        }
    }

    /// An image's pixels, once decoded.
    pub fn image(&self, id: u32) -> Option<TerminalImage> {
        let image = self.images.get(&id)?;
        let rgba = image.pixels.get()?.clone()?;
        Some(TerminalImage {
            id,
            serial: image.serial,
            width: image.width,
            height: image.height,
            rgba,
        })
    }
}

impl Pending {
    fn add(&mut self, payload: &[u8]) {
        if self.failed.is_some() {
            return;
        }
        self.tail.extend_from_slice(payload);
        let whole = self.tail.len() / 4 * 4;
        match BASE64.decode(&self.tail[..whole]) {
            Ok(bytes) => self.data.extend(bytes),
            Err(_) => self.failed = Some("EINVAL:bad base64 data"),
        }
        self.tail.drain(..whole);
        if self.data.len() > MAX_TRANSMISSION_BYTES {
            self.failed = Some("EFBIG:transmission too large");
            self.data = Vec::new();
        }
    }
}

fn decode_base64(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() / 4 * 3 > MAX_TRANSMISSION_BYTES {
        return None;
    }
    BASE64.decode(payload).ok()
}

/// A temporary file's data (`t=t`), under kitty's rules: a regular file
/// in the temporary folder with `tty-graphics-protocol` in its name,
/// deleted once read.
fn read_temporary_file(path: &[u8], offset: u32, size: u32) -> Result<Vec<u8>, &'static str> {
    use std::io::{Read, Seek};
    const REFUSED: &str = "EPERM:not a temporary file Impulse may read";
    let path = std::str::from_utf8(path).map_err(|_| REFUSED)?;
    let path = Path::new(path);
    let named = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains("tty-graphics-protocol"));
    let folder = path.parent().and_then(|p| p.canonicalize().ok());
    let temporary = folder.is_some_and(|folder| {
        let mut roots = vec![std::env::temp_dir(), "/tmp".into(), "/dev/shm".into()];
        if let Some(dir) = std::env::var_os("TMPDIR") {
            roots.push(dir.into());
        }
        roots
            .iter()
            .filter_map(|root| root.canonicalize().ok())
            .any(|root| folder.starts_with(root))
    });
    let regular = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file());
    if !(named && temporary && regular) {
        return Err(REFUSED);
    }
    let read = || -> std::io::Result<Vec<u8>> {
        let mut file = std::fs::File::open(path)?;
        file.seek(std::io::SeekFrom::Start(u64::from(offset)))?;
        let limit = if size == 0 {
            MAX_TRANSMISSION_BYTES as u64 + 1
        } else {
            u64::from(size).min(MAX_TRANSMISSION_BYTES as u64 + 1)
        };
        let mut data = Vec::new();
        file.take(limit).read_to_end(&mut data)?;
        Ok(data)
    };
    let data = read();
    let _ = std::fs::remove_file(path);
    match data {
        Ok(data) if data.len() <= MAX_TRANSMISSION_BYTES => Ok(data),
        Ok(_) => Err("EFBIG:transmission too large"),
        Err(_) => Err("EBADF:couldn't read the file"),
    }
}

/// A PNG file a program names (`t=f`): a regular file, outside the
/// kernel's and devices' folders (whose files can block a read forever),
/// within the transmission limit, that starts as a PNG does.
fn read_named_png(path: &[u8], offset: u32, size: u32) -> Result<Vec<u8>, &'static str> {
    use std::io::{Read, Seek};
    const REFUSED: &str = "EPERM:not a PNG Impulse may read";
    let path = std::str::from_utf8(path).map_err(|_| REFUSED)?;
    let path = Path::new(path).canonicalize().map_err(|_| REFUSED)?;
    if ["/proc", "/sys", "/dev"]
        .iter()
        .any(|root| path.starts_with(root))
    {
        return Err(REFUSED);
    }
    let metadata = std::fs::metadata(&path).map_err(|_| REFUSED)?;
    if !metadata.is_file() || metadata.len() > MAX_TRANSMISSION_BYTES as u64 {
        return Err(REFUSED);
    }
    let mut file = std::fs::File::open(&path).map_err(|_| REFUSED)?;
    file.seek(std::io::SeekFrom::Start(u64::from(offset)))
        .map_err(|_| REFUSED)?;
    let limit = if size == 0 {
        metadata.len()
    } else {
        u64::from(size).min(metadata.len())
    };
    let mut data = Vec::new();
    file.take(limit)
        .read_to_end(&mut data)
        .map_err(|_| REFUSED)?;
    if decode::png_size(&data).is_none() {
        return Err(REFUSED);
    }
    Ok(data)
}

/// The answer to a command, unless it asked for none: only commands that
/// name an image get one, and `q` keeps OKs (1) or everything (2) back.
/// A file named by its path (`t=f`) is never answered, so the answer
/// can't tell the sender whether the file exists.
pub(crate) fn reply(cmd: &Command, id: u32, result: Result<(), &str>) -> Option<String> {
    if cmd.id == 0 && cmd.number == 0 || cmd.medium == b'f' {
        return None;
    }
    let message = match result {
        Ok(()) if cmd.quiet >= 1 => return None,
        Err(_) if cmd.quiet >= 2 => return None,
        Ok(()) => "OK",
        Err(error) => error,
    };
    let mut keys = Vec::new();
    if id != 0 {
        keys.push(format!("i={id}"));
    }
    if cmd.number != 0 {
        keys.push(format!("I={}", cmd.number));
    }
    if cmd.placement != 0 {
        keys.push(format!("p={}", cmd.placement));
    }
    Some(format!("\x1b_G{};{message}\x1b\\", keys.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::decode::tests::png;

    fn store() -> ImageStore {
        ImageStore::new(Arc::new(|| {}))
    }

    fn at(line: usize, col: usize) -> Cursor {
        Cursor {
            alt: false,
            top: 100,
            line,
            col,
            columns: 80,
            lines: 24,
            cell: (10, 20),
        }
    }

    fn run(store: &mut ImageStore, apc: &str, at: &Cursor) -> Outcome {
        let (cmd, payload) = Command::parse(apc.as_bytes()).unwrap();
        store.handle(cmd, payload, at)
    }

    fn png_base64(width: u32, height: u32) -> String {
        let pixels = vec![200u8; (width * height * 4) as usize];
        BASE64.encode(png(width, height, &pixels))
    }

    /// Wait for the decoding thread.
    fn decoded(store: &ImageStore, id: u32) -> TerminalImage {
        for _ in 0..500 {
            if let Some(image) = store.image(id) {
                return image;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("image {id} never decoded");
    }

    #[test]
    fn transmits_and_places_a_png() {
        let mut store = store();
        let data = png_base64(25, 30);
        let outcome = run(&mut store, &format!("Ga=T,f=100,i=7;{data}"), &at(2, 3));
        assert_eq!(outcome.reply.as_deref(), Some("\x1b_Gi=7;OK\x1b\\"));
        // 25×30 pixels in 10×20 cells: 3 columns and 2 rows; the cursor
        // goes right 3 and down 1.
        assert_eq!(outcome.cursor_move, Some((1, 6)));
        assert!(outcome.changed);
        let layer = store.layer(false, 100, 0, 24);
        assert_eq!(layer.placements.len(), 1);
        let p = &layer.placements[0];
        assert_eq!((p.row, p.col), (2, 3));
        assert_eq!((p.width, p.height), (2.5, 1.5));
        assert_eq!(p.source, [0, 0, 25, 30]);
        let image = decoded(&store, 7);
        assert_eq!(
            (image.width, image.height, image.rgba.len()),
            (25, 30, 25 * 30 * 4)
        );
    }

    #[test]
    fn chunks_assemble_and_quiet_keeps_answers_back() {
        let mut store = store();
        let data = png_base64(4, 4);
        let (first, rest) = data.split_at(8);
        assert_eq!(
            run(
                &mut store,
                &format!("Ga=t,f=100,i=3,q=1,m=1;{first}"),
                &at(0, 0)
            ),
            Outcome::default()
        );
        let done = run(&mut store, &format!("Gm=0;{rest}"), &at(0, 0));
        assert_eq!(done.reply, None);
        assert_eq!(decoded(&store, 3).width, 4);
        // Errors still answer with q=1, not with q=2.
        let missing = run(&mut store, "Ga=p,i=99,q=1", &at(0, 0));
        assert_eq!(
            missing.reply.as_deref(),
            Some("\x1b_Gi=99;ENOENT:no such image\x1b\\")
        );
        assert_eq!(run(&mut store, "Ga=p,i=99,q=2", &at(0, 0)).reply, None);
    }

    #[test]
    fn chunks_encoded_each_on_their_own() {
        // As chafa sends them: the command alone, each chunk's bytes
        // encoded on their own, with the unused bits of the character
        // before the padding set, then an empty last chunk.
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut store = store();
        let png = png(4, 4, &[200u8; 4 * 4 * 4]);
        run(&mut store, "Ga=T,f=100,i=4,q=2,m=1", &at(0, 0));
        for piece in png.chunks(10) {
            let mut chunk = BASE64.encode(piece).into_bytes();
            // Ten bytes end in one byte: two characters, four bits unused.
            if let Some(last) = chunk
                .iter()
                .rposition(|&c| c != b'=')
                .filter(|&i| i + 1 < chunk.len())
            {
                let index = ALPHABET.iter().position(|&c| c == chunk[last]).unwrap();
                chunk[last] = ALPHABET[index | 0x0F];
            }
            let chunk = String::from_utf8(chunk).unwrap();
            run(&mut store, &format!("Gm=1;{chunk}"), &at(0, 0));
        }
        let done = run(&mut store, "Gm=0", &at(0, 0));
        assert!(done.changed);
        assert_eq!(decoded(&store, 4).width, 4);
    }

    #[test]
    fn queries_answer_without_storing() {
        let mut store = store();
        let outcome = run(&mut store, "Ga=q,i=31,s=1,v=1,t=d,f=24;AAAA", &at(0, 0));
        assert_eq!(outcome.reply.as_deref(), Some("\x1b_Gi=31;OK\x1b\\"));
        assert!(store.image(31).is_none() && store.images.is_empty());
        // A named file is never answered, whatever it is.
        let file = run(
            &mut store,
            "Ga=q,i=32,s=1,v=1,t=f,f=24;L2V0Yy9wYXNzd2Q=",
            &at(0, 0),
        );
        assert_eq!(file.reply, None);
        let shm = run(&mut store, "Ga=q,i=33,s=1,v=1,t=s,f=24;L2ljYXQ=", &at(0, 0));
        assert_eq!(
            shm.reply.as_deref(),
            Some("\x1b_Gi=33;EINVAL:unsupported transmission medium\x1b\\")
        );
        // No id, no answer.
        assert_eq!(
            run(&mut store, "Ga=q,s=1,v=1,f=24;AAAA", &at(0, 0)).reply,
            None
        );
    }

    #[test]
    fn numbers_get_ids_and_answers_name_both() {
        let mut store = store();
        let outcome = run(&mut store, "Ga=t,I=5,s=1,v=1,f=24;AAAA", &at(0, 0));
        let reply = outcome.reply.unwrap();
        assert!(reply.starts_with("\x1b_Gi=2147483648,I=5;OK"), "{reply}");
        // Placed by number.
        let placed = run(&mut store, "Ga=p,I=5", &at(0, 0));
        assert!(placed.changed);
    }

    #[test]
    fn sizes_from_columns_and_rows() {
        let mut store = store();
        run(&mut store, "Ga=t,i=1,s=40,v=20,f=32,o=z;", &at(0, 0));
        // o=z with no data fails later, at decoding; the size is known.
        let outcome = run(&mut store, "Ga=p,i=1,c=4", &at(0, 0));
        // 4 columns of 10 px for 40×20 px: 20 px tall, one row.
        assert_eq!(outcome.cursor_move, Some((0, 4)));
        let p = &store.layer(false, 100, 0, 24).placements[0];
        assert_eq!((p.width, p.height), (4.0, 1.0));
        let outcome = run(&mut store, "Ga=p,i=1,c=3,r=5,C=1", &at(0, 78));
        assert_eq!(outcome.cursor_move, None);
        // Past the right edge: the next line's start.
        let outcome = run(&mut store, "Ga=p,i=1,c=3,r=2", &at(0, 78));
        assert_eq!(outcome.cursor_move, Some((2, 0)));
    }

    #[test]
    fn deletes_by_id_cell_and_screen() {
        let mut store = store();
        run(&mut store, "Ga=t,i=1,s=10,v=20,f=32,o=z;", &at(0, 0));
        run(&mut store, "Ga=p,i=1,p=1", &at(0, 0));
        run(&mut store, "Ga=p,i=1,p=2", &at(5, 10));
        run(&mut store, "Ga=p,i=1,p=3,z=-1", &at(8, 0));
        assert_eq!(store.layer(false, 100, 0, 24).placements.len(), 3);
        // The cell at column 11, row 6 (1-based).
        assert!(run(&mut store, "Ga=d,d=p,x=11,y=6", &at(0, 0)).changed);
        assert_eq!(store.layer(false, 100, 0, 24).placements.len(), 2);
        assert!(run(&mut store, "Ga=d,d=z,z=-1", &at(0, 0)).changed);
        assert!(run(&mut store, "Ga=d,d=i,i=1,p=1", &at(0, 0)).changed);
        assert!(store.layer(false, 100, 0, 24).placements.is_empty());
        // Lowercase keeps the data; uppercase frees what nothing uses.
        assert!(store.images.contains_key(&1));
        run(&mut store, "Ga=d,d=I,i=1", &at(0, 0));
        assert!(!store.images.contains_key(&1));
    }

    #[test]
    fn delete_all_takes_the_visible_screen_only() {
        let mut store = store();
        run(&mut store, "Ga=t,i=1,s=10,v=20,f=32,o=z;", &at(0, 0));
        run(&mut store, "Ga=p,i=1", &at(0, 0));
        // Scrolled 30 lines on: the first placement is in the scrollback.
        let later = Cursor {
            top: 130,
            ..at(0, 0)
        };
        run(&mut store, "Ga=p,i=1", &later);
        run(&mut store, "Ga=d", &later);
        assert_eq!(store.placements.len(), 1);
        assert_eq!(store.placements[0].row, 100);
    }

    #[test]
    fn virtual_placements_keep_their_size() {
        let mut store = store();
        run(&mut store, "Ga=t,i=9,s=100,v=40,f=32,o=z;", &at(0, 0));
        let outcome = run(&mut store, "Ga=p,U=1,i=9,p=2", &at(3, 3));
        assert_eq!(outcome.cursor_move, None);
        let layer = store.layer(false, 100, 0, 24);
        assert!(layer.placements.is_empty());
        assert_eq!(layer.virtuals.get(&(9, 2)), Some(&(10, 2)));
        run(&mut store, "Ga=d,d=a", &at(0, 0));
        assert!(store
            .layer(false, 100, 0, 24)
            .virtuals
            .contains_key(&(9, 2)));
        run(&mut store, "Ga=d,d=i,i=9", &at(0, 0));
        assert!(store.layer(false, 100, 0, 24).virtuals.is_empty());
    }

    #[test]
    fn erasing_and_trimming() {
        let mut store = store();
        run(&mut store, "Ga=t,i=1,s=10,v=20,f=32,o=z;", &at(0, 0));
        run(&mut store, "Ga=p,i=1", &at(0, 0));
        // 30 lines later the first is in the scrollback; the second is on
        // screen.
        let later = Cursor {
            top: 130,
            ..at(0, 0)
        };
        run(&mut store, "Ga=p,i=1", &Cursor { line: 4, ..later });
        assert!(store.erase_screen(false, 130, 24));
        assert_eq!(store.placements.len(), 1);
        // The scrollback (130 lines) goes: so does its placement, and the
        // base moves so the screen keeps its anchors.
        run(&mut store, "Ga=p,i=1", &Cursor { line: 2, ..later });
        assert!(store.erase_scrollback(130, 130));
        assert_eq!(store.placements.len(), 1);
        assert_eq!(store.row_base(), 130);
        // Lines leaving the top of the scrollback take what's on them.
        assert!(store.trim(5));
        assert!(store.placements.is_empty());
    }

    #[test]
    fn alternate_screen_placements_stay_apart() {
        let mut store = store();
        run(&mut store, "Ga=t,i=1,s=10,v=20,f=32,o=z;", &at(0, 0));
        let alt = Cursor {
            alt: true,
            top: 0,
            ..at(1, 1)
        };
        run(&mut store, "Ga=p,i=1", &alt);
        run(&mut store, "Ga=p,i=1", &at(1, 1));
        assert_eq!(store.layer(true, 0, 0, 24).placements.len(), 1);
        assert_eq!(store.layer(false, 100, 0, 24).placements.len(), 1);
        assert!(store.clear_alt());
        assert!(store.layer(true, 0, 0, 24).placements.is_empty());
        assert_eq!(store.layer(false, 100, 0, 24).placements.len(), 1);
    }

    #[test]
    fn retransmitting_an_id_replaces_it() {
        let mut store = store();
        run(
            &mut store,
            &format!("Ga=T,f=100,i=4;{}", png_base64(2, 2)),
            &at(0, 0),
        );
        let first = decoded(&store, 4).serial;
        run(
            &mut store,
            &format!("Ga=t,f=100,i=4;{}", png_base64(3, 3)),
            &at(0, 0),
        );
        let second = decoded(&store, 4);
        assert_ne!(first, second.serial);
        assert_eq!(second.width, 3);
        // Its old placement went with the old data.
        assert!(store.layer(false, 100, 0, 24).placements.is_empty());
    }

    #[test]
    fn refuses_oversized_and_malformed_images() {
        let mut store = store();
        let big = run(&mut store, "Ga=t,i=1,s=20000,v=1,f=32,o=z;", &at(0, 0));
        assert_eq!(
            big.reply.as_deref(),
            Some("\x1b_Gi=1;EINVAL:image too large\x1b\\")
        );
        let short = run(&mut store, "Ga=t,i=2,s=2,v=2,f=32;AAAA", &at(0, 0));
        assert_eq!(
            short.reply.as_deref(),
            Some("\x1b_Gi=2;ENODATA:insufficient image data\x1b\\")
        );
        let bad = run(&mut store, "Ga=t,i=3,f=100;AAAA", &at(0, 0));
        assert_eq!(
            bad.reply.as_deref(),
            Some("\x1b_Gi=3;EBADPNG:not a PNG\x1b\\")
        );
    }

    #[test]
    fn named_files_must_be_pngs_and_are_never_answered() {
        let dir = std::env::temp_dir();
        let png_path = dir.join(format!("impulse-named-{}.png", std::process::id()));
        std::fs::write(&png_path, png(3, 2, &[9; 24])).unwrap();
        let text_path = dir.join(format!("impulse-named-{}.txt", std::process::id()));
        std::fs::write(&text_path, "not a picture").unwrap();
        let mut store = store();
        let send = |store: &mut ImageStore, id: u32, path: &str| {
            let apc = format!("Ga=T,t=f,f=100,i={id};{}", BASE64.encode(path));
            run(store, &apc, &at(0, 0))
        };
        let shown = send(&mut store, 1, png_path.to_str().unwrap());
        assert_eq!(shown.reply, None, "not even an OK");
        assert!(shown.changed);
        assert_eq!(decoded(&store, 1).width, 3);
        assert!(png_path.exists(), "the program's own file stays");
        for (id, path) in [
            (2, text_path.to_str().unwrap()),
            (3, "/proc/self/status"),
            (4, "/nope"),
        ] {
            let refused = send(&mut store, id, path);
            assert_eq!(refused, Outcome::default(), "{path}");
        }
        // Raw formats aren't read from named files at all.
        let raw = format!(
            "Ga=t,t=f,f=24,s=1,v=1,i=5;{}",
            BASE64.encode(text_path.to_str().unwrap())
        );
        assert_eq!(run(&mut store, &raw, &at(0, 0)), Outcome::default());
        let _ = std::fs::remove_file(&png_path);
        let _ = std::fs::remove_file(&text_path);
    }

    #[test]
    fn temporary_files_must_be_named_for_the_protocol() {
        let dir = std::env::temp_dir();
        let wrong = dir.join(format!("impulse-test-{}.png", std::process::id()));
        std::fs::write(&wrong, png(1, 1, &[0, 0, 0, 255])).unwrap();
        let path = BASE64.encode(wrong.to_str().unwrap());
        let mut store = store();
        let refused = run(&mut store, &format!("Ga=t,t=t,f=100,i=1;{path}"), &at(0, 0));
        assert!(refused.reply.unwrap().contains("EPERM"));
        assert!(wrong.exists(), "a file it may not read isn't deleted");
        let _ = std::fs::remove_file(&wrong);

        let right = dir.join(format!("tty-graphics-protocol-{}.png", std::process::id()));
        std::fs::write(&right, png(1, 1, &[0, 0, 0, 255])).unwrap();
        let path = BASE64.encode(right.to_str().unwrap());
        let taken = run(&mut store, &format!("Ga=t,t=t,f=100,i=2;{path}"), &at(0, 0));
        assert_eq!(taken.reply.as_deref(), Some("\x1b_Gi=2;OK\x1b\\"));
        assert!(!right.exists(), "read and deleted");
    }
}
