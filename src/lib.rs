use image::{ImageBuffer, Rgba, ImageEncoder, codecs::png::PngEncoder};
use serde::Deserialize;

const IMAGE_SIZE: f64 = 576.0;
const BITCOIN_ORANGE: Rgba<u8> = Rgba([247, 147, 26, 255]);
const BACKGROUND: Rgba<u8> = Rgba([0, 0, 0, 0]);

#[derive(Deserialize)]
pub struct Block {
    pub tx: Vec<Transaction>,
}

#[derive(Deserialize)]
pub struct Transaction {
    pub vout: Vec<Vout>,
}

#[derive(Deserialize)]
pub struct Vout {
    pub value: f64,
}

/// Value-based sizing: ceil(log10(sats)) - 5, clamped to [1, ∞)
fn tx_size_by_value(total_sats: u64) -> usize {
    if total_sats <= 100_000 {
        return 1;
    }
    let scale = (total_sats as f64).log10().ceil() as i32 - 5;
    scale.max(1) as usize
}

// ── Mondrian bin-packing layout (port of bitfeed's TxMondrianPoolScene.js) ───

#[derive(Clone, Debug)]
struct Slot {
    x: usize,
    y: usize,
    r: usize,
}

struct MondrianRow {
    slots: Vec<Slot>,
}

impl MondrianRow {
    fn new() -> Self {
        MondrianRow { slots: Vec::new() }
    }

    fn find_idx(&self, x: usize) -> Option<usize> {
        self.slots.iter().position(|s| s.x == x)
    }

    fn has(&self, x: usize) -> bool {
        self.slots.iter().any(|s| s.x == x)
    }

    fn add(&mut self, slot: Slot) {
        if slot.r == 0 { return; }
        if let Some(idx) = self.find_idx(slot.x) {
            if slot.r > self.slots[idx].r {
                self.slots[idx].r = slot.r;
            }
            return;
        }
        let pos = self.slots.iter().position(|s| s.x > slot.x).unwrap_or(self.slots.len());
        self.slots.insert(pos, slot);
    }

    fn remove(&mut self, x: usize) {
        if let Some(idx) = self.find_idx(x) {
            self.slots.remove(idx);
        }
    }
}

struct PlacedSquare {
    x: usize,
    y: usize,
    r: usize,
}

struct MondrianLayout {
    width: usize,
    rows: Vec<MondrianRow>,
}

impl MondrianLayout {
    fn new(width: usize) -> Self {
        MondrianLayout { width, rows: Vec::new() }
    }

    fn row_exists(&self, y: usize) -> bool {
        y < self.rows.len()
    }

    fn add_row(&mut self) -> usize {
        let idx = self.rows.len();
        self.rows.push(MondrianRow::new());
        idx
    }

    fn fill_slot(&mut self, slot_x: usize, slot_y: usize, slot_r: usize, sq_size: usize) -> PlacedSquare {
        let sq_right = slot_x + sq_size;
        let sq_top = slot_y + sq_size;

        if self.row_exists(slot_y) {
            self.rows[slot_y].remove(slot_x);
        }

        for row_y in slot_y..sq_top {
            if self.row_exists(row_y) {
                let mut collisions: Vec<(usize, usize)> = Vec::new();
                let mut max_excess: usize = 0;

                for s in &self.rows[row_y].slots {
                    let s_right = s.x + s.r;
                    if !(s_right <= slot_x || s.x >= sq_right) {
                        collisions.push((s.x, s.r));
                        let slot_right_edge = slot_x + slot_r;
                        let excess = if s_right > slot_right_edge { s_right - slot_right_edge } else { 0 };
                        max_excess = max_excess.max(excess);
                    }
                }

                let rhs_r = slot_r.saturating_sub(sq_size) + max_excess;
                if sq_right < self.width && !self.rows[row_y].has(sq_right) && rhs_r > 0 {
                    self.rows[row_y].add(Slot { x: sq_right, y: row_y, r: rhs_r });
                }

                for (cx, _) in collisions {
                    let new_r = slot_x.saturating_sub(cx);
                    if new_r > 0 {
                        if let Some(idx) = self.rows[row_y].find_idx(cx) {
                            self.rows[row_y].slots[idx].r = new_r;
                        }
                    } else {
                        self.rows[row_y].remove(cx);
                    }
                }
            } else {
                self.add_row();
                if slot_x > 0 {
                    self.rows[row_y].add(Slot { x: 0, y: row_y, r: slot_x });
                }
                if sq_right < self.width {
                    self.rows[row_y].add(Slot { x: sq_right, y: row_y, r: self.width - sq_right });
                }
            }
        }

        let check_from = slot_y.saturating_sub(sq_size);
        for row_y in check_from..slot_y {
            if !self.row_exists(row_y) { continue; }

            let mut to_adjust: Vec<(usize, usize)> = Vec::new();
            for s in &self.rows[row_y].slots {
                let s_right = s.x + s.r;
                if s.x < sq_right && s_right > slot_x && s.y + s.r >= slot_y {
                    to_adjust.push((s.x, s.r));
                }
            }

            for (sx, old_r) in to_adjust {
                let new_r = slot_y.saturating_sub(row_y);
                if new_r > 0 {
                    if let Some(idx) = self.rows[row_y].find_idx(sx) {
                        self.rows[row_y].slots[idx].r = new_r;
                    }
                    let mut rem_x = sx + new_r;
                    let mut rem_y = row_y;
                    let mut rem_w = old_r.saturating_sub(new_r);
                    let mut rem_h = new_r;
                    while rem_w > 0 && rem_h > 0 {
                        while !self.row_exists(rem_y) { self.add_row(); }
                        if rem_w <= rem_h {
                            self.rows[rem_y].add(Slot { x: rem_x, y: rem_y, r: rem_w });
                            rem_y += rem_w;
                            rem_h = rem_h.saturating_sub(rem_w);
                        } else {
                            self.rows[rem_y].add(Slot { x: rem_x, y: rem_y, r: rem_h });
                            rem_x += rem_h;
                            rem_w = rem_w.saturating_sub(rem_h);
                        }
                    }
                } else {
                    self.rows[row_y].remove(sx);
                }
            }
        }

        PlacedSquare { x: slot_x, y: slot_y, r: sq_size }
    }

    fn place(&mut self, size: usize) -> PlacedSquare {
        for row_idx in 0..self.rows.len() {
            let matching = self.rows[row_idx].slots.iter()
                .find(|s| s.r >= size)
                .map(|s| (s.x, s.y, s.r));

            if let Some((sx, sy, sr)) = matching {
                return self.fill_slot(sx, sy, sr, size);
            }
        }

        let new_y = self.add_row();
        self.rows[new_y].add(Slot { x: 0, y: new_y, r: self.width });
        self.fill_slot(0, new_y, self.width, size)
    }
}

/// Render a block's transactions as a Mondrian-style bitmap PNG.
///
/// Fixed 576x576 output. Each tx becomes an orange square sized by total output
/// value (log10 scale), packed with Mondrian bin-packing.
///
/// Rendering scales to fill the image:
///   gridSize = IMAGE_SIZE / (max_extent - 0.5)
///   unitPadding = gridSize / 4
///   Content is centered in both axes.
pub fn render_bitmap(block: &Block) -> Vec<u8> {
    let tx_sizes: Vec<usize> = block
        .tx
        .iter()
        .map(|tx| {
            let sats: u64 = tx.vout.iter().map(|o| (o.value * 100_000_000.0) as u64).sum();
            tx_size_by_value(sats.max(1))
        })
        .collect();

    // blockWidth = ceil(sqrt(total_area))
    let total_area: usize = tx_sizes.iter().map(|&s| s * s).sum();
    let block_width = ((total_area as f64).sqrt().ceil() as usize).max(1);

    let mut layout = MondrianLayout::new(block_width);
    let mut squares: Vec<PlacedSquare> = Vec::new();

    for &size in &tx_sizes {
        let sq = layout.place(size);
        squares.push(sq);
    }

    if squares.is_empty() {
        squares.push(PlacedSquare { x: 0, y: 0, r: 1 });
    }

    // Measure actual content extent
    let content_w = squares.iter().map(|sq| sq.x + sq.r).max().unwrap_or(1);
    let content_h = squares.iter().map(|sq| sq.y + sq.r).max().unwrap_or(1);
    let max_extent = content_w.max(content_h) as f64;

    // Scale grid to fill the image: gridSize chosen so content spans full IMAGE_SIZE
    let grid_size = IMAGE_SIZE / (max_extent - 0.5);
    let unit_padding = (grid_size / 4.0).round();

    // Center horizontally based on content width; vertically use max_extent
    // so content is always top-aligned (matching ME's renderer behavior).
    let rendered_w = content_w as f64 * grid_size - 2.0 * unit_padding;
    let rendered_h = max_extent * grid_size - 2.0 * unit_padding;
    let offset_x = (IMAGE_SIZE - rendered_w) / 2.0;
    let offset_y = (IMAGE_SIZE - rendered_h) / 2.0;

    let img_size = IMAGE_SIZE as u32;
    let mut img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(img_size, img_size, BACKGROUND);

    for sq in &squares {
        let col = sq.x as f64;
        let row = sq.y as f64;
        let size = sq.r as f64;

        let x0 = (offset_x + col * grid_size).floor() as u32;
        let y0 = (offset_y + row * grid_size).floor() as u32;
        let x1 = (offset_x + (col + size) * grid_size - 2.0 * unit_padding).ceil().min(IMAGE_SIZE) as u32;
        let y1 = (offset_y + (row + size) * grid_size - 2.0 * unit_padding).ceil().min(IMAGE_SIZE) as u32;

        for y in y0..y1 {
            for x in x0..x1 {
                if x < img_size && y < img_size {
                    img.put_pixel(x, y, BITCOIN_ORANGE);
                }
            }
        }
    }

    let mut buf = Vec::new();
    PngEncoder::new(&mut buf)
        .write_image(img.as_raw(), img_size, img_size, image::ExtendedColorType::Rgba8)
        .unwrap();
    buf
}
