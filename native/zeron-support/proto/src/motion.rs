//! Loader motion — the pure math behind zeron's loading indicators.
//!
//! These are the curves and constants the gpui viewport animates with
//! (`zeron-ui/src/motion.rs`, `zeron-ui/src/loaders.rs`), lifted here so any
//! surface animates the *same* loaders rather than inventing its own spinner.
//! A loading indicator is a brand surface; two of them that disagree read as
//! two products.
//!
//! Everything is a pure function of a phase in `0..1`, so a caller can drive
//! it from a frame delta or from wall-clock elapsed time and get identical
//! output.

/// Zeron loader pulse period.
pub const ZERON_PULSE_MS: u64 = 2_400;
/// Gradient matrix spinner wave period.
pub const GRADIENT_SPIN_MS: u64 = 750;

/// Cells in the zeron wave loader.
pub const ZERON_CELLS: usize = 5;
/// Side length of the gradient spinner matrix.
pub const MATRIX_SIDE: usize = 3;

/// Zeron loader cells rest at this opacity between pulses.
pub const PULSE_MIN_OPACITY: f32 = 0.08;
/// …and at this scale.
pub const PULSE_MIN_SCALE: f32 = 0.9;
/// Per-cell stagger, as a fraction of the pulse period (0.15s of 2.4s).
pub const PULSE_STAGGER: f32 = 0.15 / 2.4;

/// Per-row tints of the gradient matrix spinner — zeron's "sunrise" gradient
/// sampled at each row: cool blue at the top, through amber, to pink.
pub const GSPIN_ROW_TINTS: [u32; MATRIX_SIDE] = [0xB6D3EF, 0xEDB185, 0xF888A0];
/// Opacity a gradient-spinner cell rests at between pulses.
pub const GSPIN_DIM: f32 = 0.1;

/// Clockwise ring position of each `(row, col)` cell of the 2×3 mini spinner,
/// top-left first: (0,0) → (0,1) → (1,1) → (2,1) → (2,0) → (1,0). Every cell of
/// a 2×3 grid is on the ring, so the brightness chases around it.
pub const MINI_RING: [[usize; 2]; 3] = [[0, 1], [5, 2], [4, 3]];
/// Cells in the mini spinner's ring.
pub const MINI_RING_LEN: f32 = 6.0;

/// Linear interpolation.
pub fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// A cell's phase, given the loader's raw phase and the cell's index.
pub fn staggered_phase(raw_delta: f32, index: usize, stagger: f32) -> f32 {
    (raw_delta - index as f32 * stagger).rem_euclid(1.0)
}

/// Cosine pulse: 0 at phase 0, 1 at phase 0.5, back to 0 at phase 1.
pub fn pulse_wave(phase: f32) -> f32 {
    0.5 - 0.5 * (phase * std::f32::consts::TAU).cos()
}

/// Zeron loader cell opacity for a phase: 0.08 → 1 → 0.08.
pub fn pulse_opacity(phase: f32) -> f32 {
    PULSE_MIN_OPACITY + (1.0 - PULSE_MIN_OPACITY) * pulse_wave(phase)
}

/// Zeron loader cell scale for a phase: 0.9 → 1 → 0.9.
pub fn pulse_scale(phase: f32) -> f32 {
    PULSE_MIN_SCALE + (1.0 - PULSE_MIN_SCALE) * pulse_wave(phase)
}

/// Gradient-spin cell opacity for a local phase `t` (0..1 of the period),
/// ported from zeron's `gradient-spin-pulse` keyframes: full at the cycle
/// start, easing down to `dim` by 45%, resting at `dim` until 92%, then rising
/// back to full — the per-cell phase offset sweeps this pulse across the grid.
pub fn gspin_opacity(t: f32, dim: f32) -> f32 {
    let t = t.rem_euclid(1.0);
    if t < 0.45 {
        lerp(1.0, dim, t / 0.45)
    } else if t < 0.92 {
        dim
    } else {
        lerp(dim, 1.0, (t - 0.92) / 0.08)
    }
}

/// The phase offset of a `(row, col)` cell in the 3×3 gradient spinner: the
/// pulse enters at the bottom edge and converges toward the top-centre cell, so
/// the wave reads as travelling upward.
pub fn gspin_cell_phase(row: usize, col: usize) -> f32 {
    let centre = (MATRIX_SIDE as f32 - 1.0) / 2.0;
    let max = MATRIX_SIDE as f32 - 1.0 + centre;
    let d = MATRIX_SIDE as f32 - 1.0 - row as f32 + (col as f32 - centre).abs();
    if max == 0.0 { 0.0 } else { d / (max + 1.0) }
}

/// The zeron mark's pixels — `[x, y]` of each 100×100 cell on the 820×940
/// canvas (zeron's `logo.tsx` CELLS), shared by the static mark and the
/// animated loader.
#[rustfmt::skip]
pub const MARK_CELLS: [(f32, f32); 34] = [
    (0., 600.), (0., 720.), (240., 840.), (240., 720.), (120., 840.), (120., 600.), (240., 600.),
    (0., 480.), (0., 360.), (480., 840.), (480., 720.), (120., 360.), (120., 240.), (240., 360.),
    (600., 720.), (480., 600.), (360., 360.), (240., 240.), (600., 600.), (720., 600.), (720., 480.),
    (240., 120.), (600., 380.), (720., 240.), (720., 0.), (480., 240.), (480., 0.), (120., 480.),
    (240., 480.), (360., 840.), (360., 720.), (360., 600.), (360., 480.), (120., 720.),
];

/// Fraction of the pulse cycle the mark's light sweep occupies.
pub const MARK_SPREAD: f32 = 0.55;

/// Per-cell stagger along the zeron's flight axis. The stagger *adds* phase
/// (the original uses a negative CSS delay, starting the cell mid-cycle), so a
/// larger value means the cell is further along and therefore **leads**: the
/// tail tip `(720, 0)` leads at `MARK_SPREAD`, the head `(0, 840)` trails at 0.
pub fn mark_cell_stagger(x: f32, y: f32) -> f32 {
    let t = (820.0 - x + y) / 1660.0;
    (1.0 - t) * MARK_SPREAD
}

/// A mark cell's phase at loader phase `delta`.
pub fn mark_phase(delta: f32, x: f32, y: f32) -> f32 {
    (delta + mark_cell_stagger(x, y)).rem_euclid(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, what: &str) {
        assert!((a - b).abs() < 1e-5, "{what}: {a} vs {b}");
    }

    #[test]
    fn the_pulse_is_a_full_cosine_cycle() {
        close(pulse_wave(0.0), 0.0, "trough at 0");
        close(pulse_wave(0.5), 1.0, "crest at half");
        close(pulse_wave(1.0), 0.0, "trough at 1");
        // Opacity and scale ride the same wave between their own bounds.
        close(pulse_opacity(0.0), PULSE_MIN_OPACITY, "dim rest");
        close(pulse_opacity(0.5), 1.0, "full crest");
        close(pulse_scale(0.0), PULSE_MIN_SCALE, "small rest");
        close(pulse_scale(0.5), 1.0, "full scale");
    }

    #[test]
    fn stagger_offsets_each_cell_and_wraps() {
        close(staggered_phase(0.0, 0, PULSE_STAGGER), 0.0, "cell 0");
        close(
            staggered_phase(0.0, 1, PULSE_STAGGER),
            1.0 - PULSE_STAGGER,
            "cell 1 trails into the previous cycle",
        );
        // Phase is periodic: a whole extra turn changes nothing.
        close(
            staggered_phase(0.3, 2, PULSE_STAGGER),
            staggered_phase(1.3, 2, PULSE_STAGGER),
            "wraps",
        );
        // Always inside the unit interval, for any input.
        for raw in [-4.2f32, -0.1, 0.0, 0.5, 7.9] {
            for index in 0..ZERON_CELLS {
                let phase = staggered_phase(raw, index, PULSE_STAGGER);
                assert!((0.0..1.0).contains(&phase), "{raw} {index} -> {phase}");
            }
        }
    }

    #[test]
    fn gradient_spin_holds_dim_then_snaps_back() {
        close(gspin_opacity(0.0, GSPIN_DIM), 1.0, "starts full");
        close(gspin_opacity(0.45, GSPIN_DIM), GSPIN_DIM, "down by 45%");
        close(gspin_opacity(0.7, GSPIN_DIM), GSPIN_DIM, "rests dim");
        close(gspin_opacity(1.0, GSPIN_DIM), 1.0, "back to full");
        // Never leaves its bounds, at any phase.
        for step in 0..200 {
            let value = gspin_opacity(step as f32 / 100.0, GSPIN_DIM);
            assert!((GSPIN_DIM..=1.0).contains(&value), "{step} -> {value}");
        }
    }

    #[test]
    fn the_gradient_wave_travels_upward() {
        // The bottom row leads and the top-centre cell trails, which is what
        // makes the pulse read as rising.
        let bottom = gspin_cell_phase(MATRIX_SIDE - 1, 1);
        let top = gspin_cell_phase(0, 1);
        assert!(bottom < top, "bottom {bottom} should lead top {top}");
        // Symmetric about the centre column.
        close(gspin_cell_phase(1, 0), gspin_cell_phase(1, 2), "symmetry");
    }

    #[test]
    fn the_mark_sweeps_from_tail_to_head() {
        // The stagger adds phase, so leading means a LARGER value: the tail tip
        // is already mid-cycle while the head is still at zero.
        let tail = mark_cell_stagger(720.0, 0.0);
        let head = mark_cell_stagger(0.0, 840.0);
        assert!(tail > head, "tail {tail} should lead head {head}");
        close(head, 0.0, "the head anchors the sweep");
        close(
            tail,
            MARK_SPREAD * (1.0 - 100.0 / 1660.0),
            "tail leads by the spread",
        );
        // Every cell stays inside the unit interval once phased.
        for (x, y) in MARK_CELLS {
            let phase = mark_phase(0.3, x, y);
            assert!((0.0..1.0).contains(&phase), "({x},{y}) -> {phase}");
        }
        // Every cell's stagger stays inside the sweep window.
        for (x, y) in MARK_CELLS {
            let stagger = mark_cell_stagger(x, y);
            assert!(
                (0.0..=MARK_SPREAD).contains(&stagger),
                "({x},{y}) -> {stagger}"
            );
        }
    }

    #[test]
    fn the_mini_ring_visits_every_cell_once() {
        let mut seen: Vec<usize> = MINI_RING.iter().flatten().copied().collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..MINI_RING_LEN as usize).collect::<Vec<_>>());
    }
}
