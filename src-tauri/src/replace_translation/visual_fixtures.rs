//! Real-pipeline regression gate over the committed visual fixtures.
//!
//! Unlike the frontend metric unit tests (which feed hand-made numbers into the
//! metric functions), this module runs each fixture's fixed ground-truth OCR
//! leaves and its `source.png` through the REAL layout and mask pipeline
//! (`filter_replaceable_spans` + `build_replace_geometry` +
//! `analyze_text_regions`) and compares the actual render-slot anchors against
//! `expected_geometry.json`.
//!
//! The expected anchors are produced by the fixture generator from the exact
//! drawing coordinates (see `scripts/generate-replace-visual-fixtures.mjs`), not
//! from the pipeline, so a genuine layout regression — a whole-block upward
//! shift, a cross-line/cross-column merge, or a moved first-line anchor — makes
//! this test fail instead of silently agreeing with itself.
//!
//! Fully deterministic: no OCR model, no ONNX runtime, no network.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::layout::{build_replace_geometry, filter_replaceable_spans};
use super::mask::analyze_text_regions;
use crate::rapidocr::{RapidOcrLine, RapidOcrPoint};

#[derive(Deserialize)]
struct CaseFile {
    scene: String,
    #[serde(rename = "sourceImage")]
    source_image: String,
    leaves: String,
    #[serde(rename = "expectedGeometry")]
    expected_geometry: String,
}

#[derive(Deserialize)]
struct LeavesFile {
    leaves: Vec<LeafRecord>,
}

#[derive(Deserialize)]
struct LeafRecord {
    id: String,
    text: String,
    quad: Vec<[f32; 2]>,
}

#[derive(Deserialize)]
struct GeometryFile {
    slots: Vec<ExpectedSlot>,
}

#[derive(Deserialize)]
struct ExpectedSlot {
    id: String,
    anchor: Anchor,
}

#[derive(Deserialize)]
struct Anchor {
    x: f32,
    y: f32,
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("replace-translation")
        .join("v1")
}

fn fixture_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap_or_else(|error| panic!("read fixtures root {}: {error}", root.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("case.json").is_file())
        .collect();
    dirs.sort();
    dirs
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn leaf_to_line(leaf: &LeafRecord) -> RapidOcrLine {
    let min_x = leaf
        .quad
        .iter()
        .map(|point| point[0])
        .fold(f32::INFINITY, f32::min);
    let min_y = leaf
        .quad
        .iter()
        .map(|point| point[1])
        .fold(f32::INFINITY, f32::min);
    let max_x = leaf
        .quad
        .iter()
        .map(|point| point[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = leaf
        .quad
        .iter()
        .map(|point| point[1])
        .fold(f32::NEG_INFINITY, f32::max);
    RapidOcrLine {
        id: leaf.id.clone(),
        text: leaf.text.clone(),
        points: leaf
            .quad
            .iter()
            .map(|point| RapidOcrPoint {
                x: point[0],
                y: point[1],
            })
            .collect(),
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    }
}

/// Max acceptable distance (px) between a real render-slot anchor and the
/// ground-truth drawn anchor. Anchors should reproduce the drawn top-left
/// exactly; a couple of pixels of slack absorbs float noise while still failing
/// on any real shift. Photo text keeps a slightly wider budget.
fn anchor_tolerance(scene: &str) -> f32 {
    match scene {
        "photo" => 4.0,
        _ => 2.0,
    }
}

#[test]
fn fixtures_reproduce_ground_truth_anchors_and_glyph_masks() {
    let root = fixtures_root();
    let dirs = fixture_dirs(&root);
    assert!(
        !dirs.is_empty(),
        "no replace-translation fixtures found under {}",
        root.display()
    );

    for dir in dirs {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<fixture>")
            .to_string();
        let case: CaseFile = read_json(&dir.join("case.json"));
        let leaves_file: LeavesFile = read_json(&dir.join(&case.leaves));
        let expected: GeometryFile = read_json(&dir.join(&case.expected_geometry));
        let tolerance = anchor_tolerance(&case.scene);

        let image = image::open(dir.join(&case.source_image))
            .unwrap_or_else(|error| panic!("[{name}] open source image: {error}"))
            .to_rgb8();

        let leaves: Vec<RapidOcrLine> = leaves_file.leaves.iter().map(leaf_to_line).collect();
        assert!(!leaves.is_empty(), "[{name}] fixture has no leaves");

        // Run the REAL pipeline read-only.
        let spans = filter_replaceable_spans(image.width(), &leaves);
        let geometry = build_replace_geometry(&image, &spans);
        let analysis = analyze_text_regions(&image, &spans)
            .unwrap_or_else(|error| panic!("[{name}] analyze_text_regions: {error:?}"));

        // Anti-collapse gate: a list/table must not degenerate into one slot,
        // and no source line may be dropped or invented.
        assert_eq!(
            geometry.slots.len(),
            expected.slots.len(),
            "[{name}] slot count changed (cross-line/cross-column merge or drop): actual {}, expected {}",
            geometry.slots.len(),
            expected.slots.len()
        );

        // Every group has exactly one slot in these fixtures; index anchors by
        // group id so the expected ground-truth ids map onto real slots.
        let mut actual_by_group: HashMap<&str, Vec<(f32, f32)>> = HashMap::new();
        for slot in &geometry.slots {
            actual_by_group
                .entry(slot.group_id.as_str())
                .or_default()
                .push((slot.anchor.x, slot.anchor.y));
        }

        // Anti-"整体上移/跨行合并" gate: each expected line keeps its own drawn
        // top-left anchor, so the first line never drifts.
        for slot in &expected.slots {
            let anchors = actual_by_group.get(slot.id.as_str()).unwrap_or_else(|| {
                panic!(
                    "[{name}] expected group {} missing from pipeline output (groups: {:?})",
                    slot.id,
                    actual_by_group.keys().collect::<Vec<_>>()
                )
            });
            assert_eq!(
                anchors.len(),
                1,
                "[{name}] group {} produced {} slots, expected 1",
                slot.id,
                anchors.len()
            );
            let (ax, ay) = anchors[0];
            let drift = ((ax - slot.anchor.x).powi(2) + (ay - slot.anchor.y).powi(2)).sqrt();
            assert!(
                drift <= tolerance,
                "[{name}] group {} anchor drift {drift:.2}px > {tolerance:.1}px (actual ({ax:.1},{ay:.1}) vs ground truth ({:.1},{:.1}))",
                slot.id,
                slot.anchor.x,
                slot.anchor.y
            );
        }

        // Mask sanity: the glyph-derived erase mask must cover the text inside
        // each leaf polygon but must NOT flood the whole polygon rectangle.
        let width = analysis.mask.width as usize;
        for leaf in &spans {
            let x0 = leaf.x.floor().max(0.0) as usize;
            let y0 = leaf.y.floor().max(0.0) as usize;
            let x1 = ((leaf.x + leaf.width).ceil() as usize).min(analysis.mask.width as usize);
            let y1 = ((leaf.y + leaf.height).ceil() as usize).min(analysis.mask.height as usize);
            let mut masked = 0usize;
            let mut total = 0usize;
            for y in y0..y1 {
                for x in x0..x1 {
                    total += 1;
                    if analysis.mask.data[y * width + x] == 255 {
                        masked += 1;
                    }
                }
            }
            assert!(
                total > 0,
                "[{name}] leaf {} has an empty bounding box",
                leaf.id
            );
            let ratio = masked as f32 / total as f32;
            assert!(
                ratio > 0.02,
                "[{name}] leaf {} erase mask covers almost no text ({:.1}%)",
                leaf.id,
                ratio * 100.0
            );
            assert!(
                ratio < 0.98,
                "[{name}] leaf {} erase mask floods the whole polygon rectangle ({:.1}%)",
                leaf.id,
                ratio * 100.0
            );
        }
    }
}
