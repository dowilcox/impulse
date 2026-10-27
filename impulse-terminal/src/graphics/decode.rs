//! Turning transmitted image data into RGBA pixels, off the reader thread:
//! a big PNG would otherwise hold the terminal (and its lock) while it
//! decodes.

use std::sync::{Arc, OnceLock};

use crossbeam_channel::Sender;

/// An image's pixels: unset while decoding, then RGBA (row-major, 4 bytes
/// a pixel) or `None` when the data was bad.
pub(crate) type Pixels = Arc<OnceLock<Option<Arc<[u8]>>>>;

/// Data to decode into `slot`.
pub(crate) struct Job {
    pub slot: Pixels,
    pub data: Vec<u8>,
    /// 24 (RGB), 32 (RGBA) or 100 (PNG).
    pub format: u32,
    /// zlib-compressed (raw formats only; a PNG is inflated before its
    /// size is read).
    pub compressed: bool,
    pub width: u32,
    pub height: u32,
}

/// One decoding thread per terminal, started with its first image and
/// ending with the store.
pub(crate) struct Decoder {
    tx: Option<Sender<Job>>,
    /// Called after each image is done, to redraw.
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl Decoder {
    pub fn new(notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        Decoder { tx: None, notify }
    }

    pub fn decode(&mut self, job: Job) {
        if self.tx.is_none() {
            let (tx, rx) = crossbeam_channel::unbounded::<Job>();
            let notify = Arc::clone(&self.notify);
            let spawned = std::thread::Builder::new()
                .name("impulse-images".into())
                .spawn(move || {
                    for job in rx {
                        let pixels = decode(&job).map(Arc::from);
                        let _ = job.slot.set(pixels);
                        notify();
                    }
                });
            if spawned.is_err() {
                let _ = job.slot.set(None);
                return;
            }
            self.tx = Some(tx);
        }
        if let Some(tx) = &self.tx {
            if let Err(error) = tx.send(job) {
                let _ = error.0.slot.set(None);
            }
        }
    }
}

/// zlib data, inflated to at most `limit` bytes.
pub(crate) fn inflate(data: &[u8], limit: usize) -> Option<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, limit).ok()
}

/// A PNG's width and height, from its header.
pub(crate) fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if data.len() < 24 || !data.starts_with(SIGNATURE) || &data[12..16] != b"IHDR" {
        return None;
    }
    let be = |at: usize| u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
    Some((be(16), be(20)))
}

/// The job's RGBA pixels, `None` when the data doesn't make the image it
/// says it is.
pub(crate) fn decode(job: &Job) -> Option<Vec<u8>> {
    let pixels = job.width as usize * job.height as usize;
    match job.format {
        24 | 32 => {
            let bytes_per_pixel = if job.format == 24 { 3 } else { 4 };
            let expected = pixels * bytes_per_pixel;
            let inflated;
            let data = if job.compressed {
                inflated = inflate(&job.data, expected)?;
                &inflated
            } else {
                &job.data
            };
            if data.len() < expected {
                return None;
            }
            let data = &data[..expected];
            Some(if bytes_per_pixel == 4 {
                data.to_vec()
            } else {
                let mut rgba = Vec::with_capacity(pixels * 4);
                for [r, g, b] in data.as_chunks::<3>().0 {
                    rgba.extend_from_slice(&[*r, *g, *b, 255]);
                }
                rgba
            })
        }
        100 => decode_png(&job.data, job.width, job.height),
        _ => None,
    }
}

fn decode_png(data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let limits = png::Limits {
        bytes: super::store::MAX_IMAGE_BYTES * 2,
    };
    let mut decoder = png::Decoder::new_with_limits(std::io::Cursor::new(data), limits);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if (info.width, info.height) != (width, height) || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let pixels = width as usize * height as usize;
    let mut rgba = Vec::with_capacity(pixels * 4);
    let line = info.line_size;
    for row in buf.chunks(line).take(height as usize) {
        match info.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(&row[..width as usize * 4]),
            png::ColorType::Rgb => {
                for [r, g, b] in row[..width as usize * 3].as_chunks::<3>().0 {
                    rgba.extend_from_slice(&[*r, *g, *b, 255]);
                }
            }
            png::ColorType::GrayscaleAlpha => {
                for [g, a] in row[..width as usize * 2].as_chunks::<2>().0 {
                    rgba.extend_from_slice(&[*g, *g, *g, *a]);
                }
            }
            png::ColorType::Grayscale => {
                for &g in &row[..width as usize] {
                    rgba.extend_from_slice(&[g, g, g, 255]);
                }
            }
            // Expanded to RGB(A) by normalize_to_color8.
            png::ColorType::Indexed => return None,
        }
    }
    (rgba.len() == pixels * 4).then_some(rgba)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A PNG of `width`×`height` RGBA pixels, made with the png crate.
    pub(crate) fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
        writer.finish().unwrap();
        out
    }

    fn job(data: Vec<u8>, format: u32, compressed: bool, width: u32, height: u32) -> Job {
        Job {
            slot: Pixels::default(),
            data,
            format,
            compressed,
            width,
            height,
        }
    }

    #[test]
    fn decodes_png_and_reads_its_size() {
        let pixels = [255, 0, 0, 255, 0, 255, 0, 128];
        let data = png(2, 1, &pixels);
        assert_eq!(png_size(&data), Some((2, 1)));
        assert_eq!(
            decode(&job(data.clone(), 100, false, 2, 1)),
            Some(pixels.to_vec())
        );
        // A PNG that isn't the size it was said to be.
        assert_eq!(decode(&job(data, 100, false, 1, 1)), None);
        assert_eq!(png_size(b"not a png at all, not at all"), None);
    }

    #[test]
    fn expands_rgb_and_inflates_zlib() {
        let rgb = [1, 2, 3, 4, 5, 6];
        assert_eq!(
            decode(&job(rgb.to_vec(), 24, false, 2, 1)),
            Some(vec![1, 2, 3, 255, 4, 5, 6, 255])
        );
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&rgb, 6);
        assert_eq!(
            decode(&job(compressed, 24, true, 2, 1)),
            Some(vec![1, 2, 3, 255, 4, 5, 6, 255])
        );
        // Too little data.
        assert_eq!(decode(&job(vec![0; 7], 32, false, 2, 1)), None);
    }
}
