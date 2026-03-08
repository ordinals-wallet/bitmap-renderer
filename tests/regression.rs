use bitmap_renderer::{Block, render_bitmap};
use image::ImageReader;
use std::io::Cursor;
use std::path::PathBuf;

/// Structural similarity: compares orange pixel coverage and spatial distribution.
/// Returns a score from 0.0 (completely different) to 1.0 (identical coverage).
fn compare_bitmaps(ours_png: &[u8], reference_path: &str) -> CompareResult {
    let ours_img = ImageReader::new(Cursor::new(ours_png))
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap()
        .to_rgb8();

    let ref_img = ImageReader::open(reference_path)
        .unwrap()
        .decode()
        .unwrap()
        .to_rgb8();

    let ours_w = ours_img.width();
    let ours_h = ours_img.height();
    let ref_w = ref_img.width();
    let ref_h = ref_img.height();

    // Count orange vs white pixels in both images using a grid of cells
    // to compare spatial distribution (not pixel-exact, since sizes differ)
    let grid = 32; // compare in a 32x32 grid

    let ours_grid = coverage_grid(&ours_img, ours_w, ours_h, grid);
    let ref_grid = coverage_grid(&ref_img, ref_w, ref_h, grid);

    // Compare grid cells
    let mut matches = 0;
    let mut total = 0;
    let mut ours_filled = 0;
    let mut ref_filled = 0;

    for y in 0..grid {
        for x in 0..grid {
            let o = ours_grid[y * grid + x];
            let r = ref_grid[y * grid + x];
            total += 1;

            if o > 0.5 { ours_filled += 1; }
            if r > 0.5 { ref_filled += 1; }

            // Both filled or both empty = match; partial = weighted
            let diff = (o - r).abs();
            if diff < 0.3 {
                matches += 1;
            }
        }
    }

    let spatial_score = matches as f64 / total as f64;
    let coverage_ratio = if ref_filled > 0 {
        (ours_filled as f64 / ref_filled as f64).min(1.0 / (ours_filled as f64 / ref_filled as f64).max(0.01))
    } else {
        0.0
    };

    // Pixel-level IoU: compare orange pixels directly (scaling reference to 576x576)
    let mut both_orange = 0u64;
    let mut ours_extra = 0u64;
    let mut ref_extra = 0u64;

    for y in 0..ours_h {
        for x in 0..ours_w {
            let op = ours_img.get_pixel(x, y);
            let o = is_orange(op[0], op[1], op[2]);

            let rx = (x as u64 * ref_w as u64 / ours_w as u64) as u32;
            let ry = (y as u64 * ref_h as u64 / ours_h as u64) as u32;
            let rp = ref_img.get_pixel(rx.min(ref_w - 1), ry.min(ref_h - 1));
            let r = is_orange(rp[0], rp[1], rp[2]);

            if o && r { both_orange += 1; }
            if o && !r { ours_extra += 1; }
            if !o && r { ref_extra += 1; }
        }
    }

    let pixel_iou = if both_orange + ours_extra + ref_extra > 0 {
        both_orange as f64 / (both_orange + ours_extra + ref_extra) as f64
    } else {
        1.0
    };

    CompareResult {
        spatial_score,
        coverage_ratio,
        pixel_iou,
        pixel_extra_ours: ours_extra,
        pixel_extra_ref: ref_extra,
        ours_size: (ours_w, ours_h),
        ref_size: (ref_w, ref_h),
        ours_filled_cells: ours_filled,
        ref_filled_cells: ref_filled,
    }
}

fn is_orange(r: u8, g: u8, b: u8) -> bool {
    r > 200 && g > 100 && g < 200 && b < 80
}

/// Returns a grid of coverage values [0.0, 1.0] for each cell
fn coverage_grid(img: &image::RgbImage, w: u32, h: u32, grid: usize) -> Vec<f64> {
    let mut result = vec![0.0; grid * grid];

    for gy in 0..grid {
        for gx in 0..grid {
            let x0 = (gx as u32 * w) / grid as u32;
            let x1 = ((gx as u32 + 1) * w) / grid as u32;
            let y0 = (gy as u32 * h) / grid as u32;
            let y1 = ((gy as u32 + 1) * h) / grid as u32;

            let mut orange_count = 0u32;
            let mut total = 0u32;

            for py in y0..y1 {
                for px in x0..x1 {
                    if px < w && py < h {
                        let p = img.get_pixel(px, py);
                        total += 1;
                        if is_orange(p[0], p[1], p[2]) {
                            orange_count += 1;
                        }
                    }
                }
            }

            result[gy * grid + gx] = if total > 0 { orange_count as f64 / total as f64 } else { 0.0 };
        }
    }

    result
}

struct CompareResult {
    spatial_score: f64,
    coverage_ratio: f64,
    pixel_iou: f64,
    pixel_extra_ours: u64,
    pixel_extra_ref: u64,
    ours_size: (u32, u32),
    ref_size: (u32, u32),
    ours_filled_cells: usize,
    ref_filled_cells: usize,
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(name)
}

fn load_block(block_num: u64) -> Block {
    let path = fixture_path(&format!("fixtures/{block_num}.json"));
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Missing fixture {}: {e}", path.display()));
    serde_json::from_str(&data)
        .unwrap_or_else(|e| panic!("Bad fixture JSON {}: {e}", path.display()))
}

fn reference_path(block_num: u64) -> String {
    fixture_path(&format!("references/{block_num}.png"))
        .to_string_lossy()
        .to_string()
}

/// Run a regression test for a single block.
/// Checks spatial similarity, coverage ratio, and pixel-level IoU.
fn assert_block_matches(block_num: u64, min_spatial: f64, min_coverage: f64, min_pixel_iou: f64) {
    let block = load_block(block_num);
    let png = render_bitmap(&block);

    let ref_path = reference_path(block_num);
    assert!(
        std::path::Path::new(&ref_path).exists(),
        "Reference image missing: {ref_path}. Download it first."
    );

    let result = compare_bitmaps(&png, &ref_path);

    // Save our output for manual inspection on failure
    let out_path = fixture_path(&format!("output/{block_num}.png"));
    std::fs::create_dir_all(out_path.parent().unwrap()).ok();
    std::fs::write(&out_path, &png).ok();

    println!("Block {block_num}:");
    println!("  Ours:      {}x{}", result.ours_size.0, result.ours_size.1);
    println!("  Reference: {}x{}", result.ref_size.0, result.ref_size.1);
    println!("  Spatial:   {:.1}% ({}/{} cells match)",
        result.spatial_score * 100.0,
        (result.spatial_score * 1024.0) as usize, 1024);
    println!("  Coverage:  ours={} ref={} ratio={:.2}",
        result.ours_filled_cells, result.ref_filled_cells, result.coverage_ratio);
    println!("  Pixel IoU: {:.2}% (+{} ours, -{} ref)",
        result.pixel_iou * 100.0, result.pixel_extra_ours, result.pixel_extra_ref);
    println!("  Output:    {}", out_path.display());

    assert!(
        result.spatial_score >= min_spatial,
        "Block {block_num} REGRESSION: spatial score {:.1}% < {:.1}% threshold",
        result.spatial_score * 100.0,
        min_spatial * 100.0,
    );
    assert!(
        result.coverage_ratio >= min_coverage,
        "Block {block_num} REGRESSION: coverage ratio {:.2} < {:.2} threshold",
        result.coverage_ratio,
        min_coverage,
    );
    assert!(
        result.pixel_iou >= min_pixel_iou,
        "Block {block_num} REGRESSION: pixel IoU {:.2}% < {:.2}% threshold",
        result.pixel_iou * 100.0,
        min_pixel_iou * 100.0,
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Test cases — add new blocks here!
//
// To add a new test case:
//   1. Save the ME reference:
//      curl -o tests/references/BLOCK.png \
//        "https://bitmap-img.magiceden.dev/v1/INSCRIPTION_ID"
//
//   2. Save the block fixture (requires RPC access):
//      curl -s -u USER:PASS http://localhost:8332 \
//        -d '{"jsonrpc":"1.0","id":"1","method":"getblockhash","params":[BLOCK]}' \
//        | ... (see save_fixture.sh)
//      Or run: cargo test -- --ignored save_fixture_BLOCK
//
//   3. Add a test below:
//      #[test]
//      fn block_BLOCK() { assert_block_matches(BLOCK, 0.70, 0.50, 0.50); }
// ══════════════════════════════════════════════════════════════════════════════

// Thresholds: (min_spatial, min_coverage, min_pixel_iou)
// 13/14 blocks at 100% spatial. Pixel IoU catches sub-pixel rounding diffs.

// ── Early / small blocks (pixel-perfect) ──────────────────────────────────
#[test]
fn block_2() { assert_block_matches(2, 1.00, 1.00, 1.00); }           // Letters 1 — 1 tx

#[test]
fn block_340() { assert_block_matches(340, 1.00, 1.00, 1.00); }       // Letters 3 — 1 tx

#[test]
fn block_8112() { assert_block_matches(8112, 1.00, 1.00, 1.00); }     // Patoshi — 1 tx

#[test]
fn block_57019() { assert_block_matches(57019, 1.00, 1.00, 1.00); }   // Pizza Day — 1 tx

// ── Punks (rare low-tx blocks) ──────────────────────────────────────────────
#[test]
fn block_75546() { assert_block_matches(75546, 1.00, 1.00, 0.99); }   // 5tx Punk — 5 txs

#[test]
fn block_78319() { assert_block_matches(78319, 1.00, 1.00, 1.00); }   // Pristine Punk — 2 txs

#[test]
fn block_84534() { assert_block_matches(84534, 1.00, 1.00, 1.00); }   // Wide Neck Punk — 2 txs

#[test]
fn block_127332() { assert_block_matches(127332, 1.00, 1.00, 0.99); } // Grid Punk — 22 txs

#[test]
fn block_699412() { assert_block_matches(699412, 1.00, 1.00, 1.00); } // Community Punks — 7 txs

// ── Medium blocks ───────────────────────────────────────────────────────────
#[test]
fn block_163284() { assert_block_matches(163284, 1.00, 0.97, 0.99); } // 34 txs

#[test]
fn block_258502() { assert_block_matches(258502, 1.00, 1.00, 0.99); } // 128 txs

// ── Large blocks ────────────────────────────────────────────────────────────
#[test]
fn block_474712() { assert_block_matches(474712, 1.00, 1.00, 0.99); } // Range 400k — 2620 txs

#[test]
fn block_677633() { assert_block_matches(677633, 1.00, 0.99, 0.95); } // 2177 txs

#[test]
fn block_802383() { assert_block_matches(802383, 1.00, 0.89, 0.97); } // Range 800k — 1966 txs
