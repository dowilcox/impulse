//! Images programs show in a terminal (impulse_terminal's kitty graphics
//! support): each decoded image made a GPUI texture once, then painted
//! where the grid's placeholder cells and the direct placements put it.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::{point, px, size, Bounds, Corners, Pixels, Point, RenderImage, Window};
use impulse_terminal::{ImageLayer, ImagePlacement, TerminalBackend, TerminalImage};

/// What an image placeholder cell shows, from the grid snapshot's extras.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ImageCell {
    pub image: u32,
    pub placement: u32,
    /// The cell's row and column in the image.
    pub row: u32,
    pub col: u32,
}

impl ImageCell {
    /// An `EXTRA_IMAGE_CELL` entry's payload.
    pub fn read(payload: &[u8]) -> Option<Self> {
        let u32_at = |at: usize| {
            Some(u32::from_le_bytes(
                payload.get(at..at + 4)?.try_into().ok()?,
            ))
        };
        let u16_at = |at: usize| {
            Some(u16::from_le_bytes(
                payload.get(at..at + 2)?.try_into().ok()?,
            ))
        };
        Some(ImageCell {
            image: u32_at(0)?,
            placement: u32_at(4)?,
            row: u32::from(u16_at(8)?),
            col: u32::from(u16_at(10)?),
        })
    }

    /// Whether `next`, the cell to the right, continues this one's row of
    /// the image.
    fn continued_by(&self, next: &ImageCell, cells: u32) -> bool {
        next.image == self.image
            && next.placement == self.placement
            && next.row == self.row
            && next.col == self.col + cells
    }
}

/// The images a frame paints.
pub(crate) struct FrameImages {
    pub layer: ImageLayer,
    pub textures: HashMap<u32, Arc<RenderImage>>,
    /// The window's scale factor: an image at its own size is its pixels
    /// over this.
    pub scale: f32,
}

/// Each image on screen as a GPUI texture, made again only when its data
/// changes.
#[derive(Default)]
pub(crate) struct Textures {
    made: HashMap<u32, (u64, Arc<RenderImage>)>,
    /// Textures no longer shown, for the next paint to give back to the GPU.
    stale: Vec<Arc<RenderImage>>,
}

impl Textures {
    /// The textures for the images `ids`, from the backend's pixels where
    /// they're new; the rest go stale.
    pub fn update(
        &mut self,
        backend: &TerminalBackend,
        ids: impl IntoIterator<Item = u32>,
    ) -> HashMap<u32, Arc<RenderImage>> {
        let mut shown = HashMap::new();
        for id in ids {
            if shown.contains_key(&id) {
                continue;
            }
            // Not decoded yet (it redraws when it is), or gone.
            let Some(image) = backend.image(id) else {
                continue;
            };
            let texture = match self.made.get(&id) {
                Some((serial, texture)) if *serial == image.serial => texture.clone(),
                _ => {
                    let Some(texture) = texture(&image) else {
                        continue;
                    };
                    if let Some((_, old)) = self.made.insert(id, (image.serial, texture.clone())) {
                        self.stale.push(old);
                    }
                    texture
                }
            };
            shown.insert(id, texture);
        }
        let gone: Vec<u32> = self
            .made
            .keys()
            .filter(|id| !shown.contains_key(id))
            .copied()
            .collect();
        for id in gone {
            if let Some((_, texture)) = self.made.remove(&id) {
                self.stale.push(texture);
            }
        }
        shown
    }

    pub fn take_stale(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.stale)
    }
}

/// An image as GPUI draws it: BGRA.
fn texture(image: &TerminalImage) -> Option<Arc<RenderImage>> {
    let mut bgra = image.rgba.to_vec();
    for [r, _, b, _] in bgra.as_chunks_mut::<4>().0 {
        std::mem::swap(r, b);
    }
    let buffer = image::RgbaImage::from_raw(image.width, image.height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// The viewport rows a direct placement covers, so the rows under it
/// count as content (not blank padding to collapse or cut off).
pub(crate) fn covered_rows(layer: &ImageLayer, rows: usize) -> Vec<bool> {
    let mut covered = vec![false; rows];
    for p in &layer.placements {
        let span = (p.offset_y + p.height).ceil().max(1.0) as i32;
        for row in p.row.max(0)..(p.row + span).min(rows as i32) {
            covered[row as usize] = true;
        }
    }
    covered
}

/// Paint a direct placement whose first cell's top left is `at`, clipped
/// to `clip`.
pub(crate) fn paint_placement(
    p: &ImagePlacement,
    images: &FrameImages,
    at: Point<Pixels>,
    cell: (f32, f32),
    clip: Bounds<Pixels>,
    window: &mut Window,
) {
    let (Some(texture), Some(&(image_w, image_h))) = (
        images.textures.get(&p.image_id),
        images.layer.sizes.get(&p.image_id),
    ) else {
        return;
    };
    let (cell_w, cell_h) = cell;
    let x = f32::from(at.x) + cell_w * p.offset_x;
    let y = f32::from(at.y) + cell_h * p.offset_y;
    let (w, h) = (cell_w * p.width, cell_h * p.height);
    let [src_x, src_y, src_w, src_h] = p.source.map(|v| v as f32);
    // The whole image, scaled so its source part fills the placement.
    let (scale_x, scale_y) = (w / src_w.max(1.0), h / src_h.max(1.0));
    let image_bounds = Bounds::new(
        point(px(x - src_x * scale_x), px(y - src_y * scale_y)),
        size(px(image_w as f32 * scale_x), px(image_h as f32 * scale_y)),
    );
    let bounds = Bounds::new(point(px(x), px(y)), size(px(w), px(h))).intersect(&clip);
    let _ = window.paint_image(
        bounds,
        image_bounds,
        Corners::default(),
        texture.clone(),
        0,
        false,
    );
}

/// Paint `cells` placeholder cells in a row, the first showing `first` and
/// with its top left at `at`: their part of the image, fitted into its
/// placement's cells and centered, as kitty draws it.
pub(crate) fn paint_placeholders(
    first: &ImageCell,
    cells: usize,
    images: &FrameImages,
    at: Point<Pixels>,
    cell: (f32, f32),
    clip: Bounds<Pixels>,
    window: &mut Window,
) {
    let (Some(texture), Some(&(image_w, image_h))) = (
        images.textures.get(&first.image),
        images.layer.sizes.get(&first.image),
    ) else {
        return;
    };
    let (cell_w, cell_h) = cell;
    let virtuals = &images.layer.virtuals;
    // The placement's size in cells: its own, another of the image's, or
    // the image's own size.
    let (cols, rows) = virtuals
        .get(&(first.image, first.placement))
        .or_else(|| {
            virtuals
                .iter()
                .find(|((image, _), _)| *image == first.image)
                .map(|(_, size)| size)
        })
        .copied()
        .unwrap_or_else(|| {
            let scale = images.scale.max(0.1);
            (
                (image_w as f32 / scale / cell_w).ceil().max(1.0) as u32,
                (image_h as f32 / scale / cell_h).ceil().max(1.0) as u32,
            )
        });
    let (box_w, box_h) = (cols as f32 * cell_w, rows as f32 * cell_h);
    let fit = (box_w / image_w.max(1) as f32).min(box_h / image_h.max(1) as f32);
    let (draw_w, draw_h) = (image_w as f32 * fit, image_h as f32 * fit);
    let box_x = f32::from(at.x) - first.col as f32 * cell_w;
    let box_y = f32::from(at.y) - first.row as f32 * cell_h;
    let image_bounds = Bounds::new(
        point(
            px(box_x + (box_w - draw_w) / 2.),
            px(box_y + (box_h - draw_h) / 2.),
        ),
        size(px(draw_w), px(draw_h)),
    );
    let bounds = Bounds::new(at, size(px(cells as f32 * cell_w), px(cell_h))).intersect(&clip);
    let _ = window.paint_image(
        bounds,
        image_bounds,
        Corners::default(),
        texture.clone(),
        0,
        false,
    );
}

/// The runs of placeholder cells in a row of `cells` (`None` where a cell
/// isn't one): each run's first column, its length and what it starts
/// with.
pub(crate) fn placeholder_runs(cells: &[Option<ImageCell>]) -> Vec<(usize, usize, ImageCell)> {
    let mut runs = Vec::new();
    let mut col = 0;
    while col < cells.len() {
        let Some(first) = cells[col] else {
            col += 1;
            continue;
        };
        let mut len = 1;
        while cells
            .get(col + len)
            .copied()
            .flatten()
            .is_some_and(|next| first.continued_by(&next, len as u32))
        {
            len += 1;
        }
        runs.push((col, len, first));
        col += len;
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(image: u32, row: u32, col: u32) -> Option<ImageCell> {
        Some(ImageCell {
            image,
            placement: 0,
            row,
            col,
        })
    }

    #[test]
    fn reads_an_image_cell() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&7u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&3u16.to_le_bytes());
        payload.extend_from_slice(&4u16.to_le_bytes());
        assert_eq!(
            ImageCell::read(&payload),
            Some(ImageCell {
                image: 7,
                placement: 2,
                row: 3,
                col: 4
            })
        );
        assert_eq!(ImageCell::read(&payload[..11]), None);
    }

    #[test]
    fn runs_break_where_the_image_does() {
        let row = [
            None,
            cell(1, 0, 0),
            cell(1, 0, 1),
            cell(1, 0, 5),
            cell(2, 0, 6),
            None,
        ];
        assert_eq!(
            placeholder_runs(&row)
                .into_iter()
                .map(|(col, len, _)| (col, len))
                .collect::<Vec<_>>(),
            [(1, 2), (3, 1), (4, 1)]
        );
    }

    #[test]
    fn placements_cover_their_rows() {
        let layer = ImageLayer {
            placements: vec![ImagePlacement {
                image_id: 1,
                row: -1,
                col: 0,
                width: 2.0,
                height: 2.5,
                offset_x: 0.0,
                offset_y: 0.0,
                source: [0, 0, 1, 1],
                z: 0,
            }],
            ..Default::default()
        };
        assert_eq!(covered_rows(&layer, 4), [true, true, false, false]);
    }
}
