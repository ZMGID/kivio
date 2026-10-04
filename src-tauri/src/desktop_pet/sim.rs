//! Native port of `src/chat/kivioBlobSim.ts` (pose, cycles, blink, wink, hop,
//! poke heat, idle antics, springs, capped dt, reduced motion).
//!
//! Geometry stays in the 240×240 viewBox ([`super::shapes`]). [`sample`] applies the
//! SVG rig (squash, then rotate around [`CX`]/[`CY`], then translate) and scales by
//! [`SIZE`]` / 240` into the top-left 128×128 frame. Full `f64` is kept; the TS paint
//! path's `toFixed` is only SVG serialization.
//!
//! # Deterministic draws
//!
//! [`BlobSim::new`] draws **one** `u64` from `rand::random` and never touches `rand`
//! again. [`BlobSim::with_seed`] runs **SplitMix64** (Steele / Vigna) and maps each
//! draw to `[0, 1)` with the top 53 bits:
//!
//! ```text
//! state = (state + 0x9E3779B97F4A7C15) mod 2^64   // seed is the initial state
//! z = state
//! z = (z xor (z >> 30)) * 0xBF58476D1CE4E5B9  mod 2^64
//! z = (z xor (z >> 27)) * 0x94D049BB133111EB  mod 2^64
//! u = z xor (z >> 31)
//! random() = (u >> 11) / 2^53
//! ```
//!
//! JavaScript (BigInt), same sequence as `this.random()` in the TS class:
//!
//! ```js
//! const MASK = (1n << 64n) - 1n;
//! let state = BigInt(seed) & MASK;
//! function random() {
//!   state = (state + 0x9E3779B97F4A7C15n) & MASK;
//!   let z = state;
//!   z = ((z ^ (z >> 30n)) * 0xBF58476D1CE4E5B9n) & MASK;
//!   z = ((z ^ (z >> 27n)) * 0x94D049BB133111EB) & MASK;
//!   const u = (z ^ (z >> 31n)) & MASK;
//!   return Number(u >> 11n) / 9007199254740992;
//! }
//! ```
//!
//! Call order matches `KivioBlobSim` for `setMood` (only when the mood actually
//! changes — repeated `set_mood` is a no-op, unlike the TS method), `poke`, and
//! `sample`. The first `sample` repeats initialization just like the source,
//! including its initial mood rolls and wink deadline. `Math.sin` / `f64::sin`
//! may differ by a few ulps; integer SplitMix draws are bit-identical.
//!
//! All eight source moods are represented. [`Mood::Searching`] is source `search`
//! (wide face, ±16° sweep, hop every 4–7s), not thinking or working. Which tool
//! names count as search stays with the parent.
//!
//! No per-frame heap: morph buffers, the body playlist, and the blink queue are
//! fixed arrays. The blink queue holds 128 keys; further keys are dropped.

use std::f64::consts::PI;

use super::shapes::{body_points, face_blinks, face_points, BodyShape, FaceName, CX, CY};
use super::visual::{Color, Visual, SIZE};
use super::Mood;

const DT: f64 = 1.0 / 120.0;
const BLINK_CAP: usize = 128;
const HOP_H: [f64; 4] = [48.0, 28.0, 14.0, 6.0];
const HOP_D: [f64; 4] = [0.5, 0.382, 0.27, 0.177];
const HOP_DUR: f64 = 0.5 + 0.382 + 0.27 + 0.177;

const BLUE_RGB: [u8; 3] = [0x1d, 0x6b, 0xf0];
const RED_RGB: [u8; 3] = [0xe2, 0x3b, 0x2e];
const ERROR_RGB: [u8; 3] = [0xc4, 0x5c, 0x2a];
const EYE_RGB: [u8; 3] = [0xf3, 0xef, 0xe6];

const fn rgb(c: [u8; 3]) -> Color {
    Color {
        r: c[0] as f64 / 255.0,
        g: c[1] as f64 / 255.0,
        b: c[2] as f64 / 255.0,
        a: 1.0,
    }
}

const BLUE: Color = rgb(BLUE_RGB);
const EYE: Color = rgb(EYE_RGB);
const ERROR: Color = rgb(ERROR_RGB);

#[derive(Clone, Copy)]
struct SpringGain {
    spin: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
    squash: [f64; 2],
    blink: [f64; 2],
    gaze: [f64; 2],
    morph: [f64; 2],
    body: [f64; 2],
    boost: [f64; 2],
}

const ACTIVE: SpringGain = SpringGain {
    spin: [5.0, 0.9],
    x: [3.5, 1.0],
    y: [4.0, 1.0],
    squash: [10.0, 0.8],
    blink: [26.0, 1.0],
    gaze: [13.0, 1.0],
    morph: [7.0, 1.0],
    body: [6.0, 0.92],
    boost: [9.0, 0.85],
};

const IDLE: SpringGain = SpringGain {
    spin: [2.0, 1.0],
    x: [1.6, 1.0],
    y: [1.8, 1.0],
    squash: [3.0, 1.0],
    blink: [9.0, 1.0],
    gaze: [2.6, 1.0],
    morph: [2.2, 1.0],
    body: [1.9, 1.0],
    boost: [3.2, 0.9],
};

const IDLE_ANTICS: [BodyShape; 4] = [
    BodyShape::Squircle,
    BodyShape::Cloud,
    BodyShape::Pebble,
    BodyShape::Bean,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlobMood {
    Idle,
    Think,
    Search,
    Work,
    Speak,
    Error,
    Done,
    Wait,
}

fn map_mood(mood: Mood) -> BlobMood {
    match mood {
        Mood::Idle => BlobMood::Idle,
        Mood::Thinking => BlobMood::Think,
        Mood::Searching => BlobMood::Search,
        Mood::Working => BlobMood::Work,
        Mood::Speaking => BlobMood::Speak,
        Mood::Waiting => BlobMood::Wait,
        Mood::Done => BlobMood::Done,
        Mood::Error => BlobMood::Error,
    }
}

fn face_play(mood: BlobMood) -> &'static [FaceName] {
    use FaceName::*;
    match mood {
        BlobMood::Idle => &[
            Neutral, Dots, Neutral, Smirk, Neutral, Peek, Sleepy, Neutral, Hmm,
        ],
        BlobMood::Think => &[LookUp, Hmm, Lines, Neutral, LookUp, Focus],
        BlobMood::Search => &[Wide, Peek, Dots, Wide, Neutral],
        BlobMood::Work => &[Focus, Lines, Focus, Neutral, Dots],
        BlobMood::Speak => &[Neutral, Dots, Happy, Neutral, Hmm],
        BlobMood::Error => &[Dizzy, Worry, Tiny, Dizzy, Lines],
        BlobMood::Done => &[Happy, Sparkle, Happy, Content],
        BlobMood::Wait => &[Wide, Neutral, Wide, Hmm],
    }
}

fn face_hold(mood: BlobMood) -> [f64; 2] {
    match mood {
        BlobMood::Idle => [5_000.0, 18_000.0],
        BlobMood::Think => [2_000.0, 3_600.0],
        BlobMood::Search => [1_000.0, 1_800.0],
        BlobMood::Work => [1_800.0, 3_200.0],
        BlobMood::Speak => [2_800.0, 5_000.0],
        BlobMood::Error => [2_200.0, 3_800.0],
        BlobMood::Done => [700.0, 1_100.0],
        BlobMood::Wait => [2_600.0, 4_800.0],
    }
}

fn body_play(mood: BlobMood) -> &'static [BodyShape] {
    use BodyShape::*;
    match mood {
        BlobMood::Idle | BlobMood::Search | BlobMood::Done | BlobMood::Wait => &[Circle],
        BlobMood::Think => &[Cloud, Cloud, Circle, Cloud],
        BlobMood::Work => &[Squircle, Squircle, Circle, Squircle],
        BlobMood::Speak => &[Bubble, Circle, Bubble, Bubble],
        BlobMood::Error => &[Puddle],
    }
}

fn body_chance(mood: BlobMood) -> f64 {
    match mood {
        BlobMood::Idle | BlobMood::Error | BlobMood::Done => 1.0,
        BlobMood::Think => 0.35,
        BlobMood::Search | BlobMood::Work => 0.4,
        BlobMood::Speak => 0.25,
        BlobMood::Wait => 0.5,
    }
}

fn body_hold(mood: BlobMood) -> [f64; 2] {
    match mood {
        BlobMood::Idle | BlobMood::Error | BlobMood::Done => [1.0e9, 1.0e9],
        BlobMood::Think => [3_500.0, 7_000.0],
        BlobMood::Search => [2_500.0, 5_000.0],
        BlobMood::Work => [3_000.0, 6_500.0],
        BlobMood::Speak => [3_000.0, 6_000.0],
        BlobMood::Wait => [4_000.0, 8_000.0],
    }
}

fn blink_cadence(mood: BlobMood) -> Option<[f64; 2]> {
    match mood {
        BlobMood::Idle => Some([4_000.0, 16_000.0]),
        BlobMood::Think => Some([3_500.0, 7_000.0]),
        BlobMood::Search => Some([1_600.0, 4_000.0]),
        BlobMood::Work => Some([2_800.0, 5_500.0]),
        BlobMood::Speak => Some([3_000.0, 7_000.0]),
        BlobMood::Error => Some([3_500.0, 7_000.0]),
        BlobMood::Done => None,
        BlobMood::Wait => Some([2_200.0, 4_800.0]),
    }
}

fn hop_cadence(mood: BlobMood) -> Option<[f64; 2]> {
    match mood {
        BlobMood::Idle => Some([22_000.0, 56_000.0]),
        BlobMood::Search => Some([4_000.0, 7_000.0]),
        BlobMood::Work => Some([6_000.0, 9_000.0]),
        BlobMood::Think => Some([8_000.0, 12_000.0]),
        _ => None,
    }
}

fn wink_mood(mood: BlobMood) -> bool {
    matches!(mood, BlobMood::Idle | BlobMood::Speak | BlobMood::Done)
}

/// SplitMix64. `state` is the seed before the first gamma add.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }
}

#[derive(Clone, Copy)]
struct Spring {
    x: f64,
    v: f64,
    t: f64,
}

impl Spring {
    fn new(x: f64) -> Self {
        Self { x, v: 0.0, t: x }
    }

    fn step(&mut self, freq: f64, damp: f64, dt: f64) {
        self.v += (-2.0 * damp * freq * self.v - freq * freq * (self.x - self.t)) * dt;
        self.x += self.v * dt;
        if !self.x.is_finite() || !self.v.is_finite() {
            self.x = self.t;
            self.v = 0.0;
        }
    }
}

struct BlinkQueue {
    at: [f64; BLINK_CAP],
    v: [f64; BLINK_CAP],
    head: usize,
    len: usize,
}

impl BlinkQueue {
    const fn new() -> Self {
        Self {
            at: [0.0; BLINK_CAP],
            v: [0.0; BLINK_CAP],
            head: 0,
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    fn push(&mut self, at: f64, v: f64) {
        if self.len == BLINK_CAP {
            return;
        }
        let index = (self.head + self.len) % BLINK_CAP;
        self.at[index] = at;
        self.v[index] = v;
        self.len += 1;
    }

    /// Last key with `at <= now`, removing every due key. `None` if none are due.
    fn consume(&mut self, now: f64) -> Option<f64> {
        let mut key = None;
        while self.len > 0 && now >= self.at[self.head] {
            key = Some(self.v[self.head]);
            self.head = (self.head + 1) % BLINK_CAP;
            self.len -= 1;
        }
        key
    }
}

struct Pose {
    spin: f64,
    tx: f64,
    ty: f64,
    squash: f64,
    lid: f64,
    boost: f64,
}

struct PoseCtx {
    nod_until: f64,
    nod_end: f64,
    impulse_at: f64,
    shake_until: f64,
    hop_until: f64,
    bias_until: f64,
    bias_spin: f64,
    bias_tx: f64,
    bias_ty: f64,
    bias_squash: f64,
    antic: Option<BodyShape>,
    antic_until: f64,
}

struct Gaze {
    x: f64,
    y: f64,
    hold: [f64; 2],
}

struct IdleBias {
    spin: f64,
    tx: f64,
    ty: f64,
    squash: f64,
    hold: [f64; 2],
    hop: bool,
    antic: Option<BodyShape>,
}

fn ease(n: f64) -> f64 {
    if n < 0.5 {
        4.0 * n * n * n
    } else {
        let u = -2.0 * n + 2.0;
        1.0 - u * u * u / 2.0
    }
}

fn clamp(n: f64, a: f64, b: f64) -> f64 {
    n.max(a).min(b)
}

fn mix_rgb(a: [u8; 3], b: [u8; 3], t: f64) -> Color {
    let u = clamp(t, 0.0, 1.0);
    let channel = |i: usize| {
        let av = f64::from(a[i]);
        let bv = f64::from(b[i]);
        (av + (bv - av) * u).round() as u8
    };
    rgb([channel(0), channel(1), channel(2)])
}

/// `None` when the hop has finished (caller clears `hop_at`). `Some(0)` when idle.
fn hop_y(hop_at: f64, now: f64, scale: f64, time_scale: f64) -> Option<f64> {
    if hop_at < 0.0 {
        return Some(0.0);
    }
    let et = (now - hop_at) / 1000.0 / time_scale.max(0.01);
    if et >= HOP_DUR {
        return None;
    }
    let mut elapsed = 0.0;
    for i in 0..HOP_D.len() {
        let d = HOP_D[i];
        if et < elapsed + d {
            let bn = (et - elapsed) / d;
            return Some(-4.0 * HOP_H[i] * bn * (1.0 - bn) * 0.5 * scale);
        }
        elapsed += d;
    }
    Some(0.0)
}

fn scale_xy(x: f64, y: f64) -> [f64; 2] {
    let s = SIZE / 240.0;
    [x * s, y * s]
}

/// SVG `translate(tx ty) rotate(spin CX CY)` then uniform scale into the pet.
fn rig(x: f64, y: f64, spin_deg: f64, tx: f64, ty: f64) -> [f64; 2] {
    let rad = spin_deg * PI / 180.0;
    let (sn, cs) = rad.sin_cos();
    let dx = x - CX;
    let dy = y - CY;
    scale_xy(CX + dx * cs - dy * sn + tx, CY + dx * sn + dy * cs + ty)
}

fn centroid(pts: &[[f64; 2]; 48]) -> [f64; 2] {
    let mut x = 0.0;
    let mut y = 0.0;
    for p in pts {
        x += p[0];
        y += p[1];
    }
    [x / 48.0, y / 48.0]
}

fn lerp_body(from: &[[f64; 2]; 72], to: &[[f64; 2]; 72], k: f64, out: &mut [[f64; 2]; 72]) {
    if k >= 1.0 {
        *out = *to;
        return;
    }
    for i in 0..72 {
        out[i] = [
            from[i][0] + (to[i][0] - from[i][0]) * k,
            from[i][1] + (to[i][1] - from[i][1]) * k,
        ];
    }
}

fn lerp_eye(from: &[[f64; 2]; 48], to: &[[f64; 2]; 48], k: f64, out: &mut [[f64; 2]; 48]) {
    if k >= 1.0 {
        *out = *to;
        return;
    }
    for i in 0..48 {
        out[i] = [
            from[i][0] + (to[i][0] - from[i][0]) * k,
            from[i][1] + (to[i][1] - from[i][1]) * k,
        ];
    }
}

pub struct BlobSim {
    reduced: bool,
    rng: SplitMix64,
    mood: BlobMood,
    mood_applied: bool,
    inited: bool,
    spin: Spring,
    tx: Spring,
    ty: Spring,
    squash: Spring,
    blink: Spring,
    gaze_x: Spring,
    gaze_y: Spring,
    boost: Spring,
    face_from: [[[f64; 2]; 48]; 2],
    face_to: [[[f64; 2]; 48]; 2],
    face_key: FaceName,
    face_s: Spring,
    body_from: [[f64; 2]; 72],
    body_to: [[f64; 2]; 72],
    body_key: BodyShape,
    body_s: Spring,
    face_idx: usize,
    body_idx: usize,
    body_list: [BodyShape; 4],
    body_len: usize,
    t0: f64,
    last: f64,
    face_until: f64,
    body_until: f64,
    blink_until: f64,
    gaze_until: f64,
    wink_at: f64,
    wink_eye: usize,
    wink_until: f64,
    wink_dur: f64,
    hop_at: f64,
    poke_heat: f64,
    poke_count: u32,
    last_poke_at: f64,
    heat_hold_until: f64,
    poke_shape: Option<BodyShape>,
    poke_shape_until: f64,
    blinks: BlinkQueue,
    ctx: PoseCtx,
}

impl BlobSim {
    /// One `rand::random::<u64>()` seed, then only SplitMix64.
    pub fn new(reduced_motion: bool) -> Self {
        Self::with_seed(reduced_motion, rand::random::<u64>())
    }

    pub(super) fn with_seed(reduced_motion: bool, seed: u64) -> Self {
        let neutral = *face_points(FaceName::Neutral);
        let circle = *body_points(BodyShape::Circle);
        Self {
            reduced: reduced_motion,
            rng: SplitMix64::new(seed),
            mood: BlobMood::Idle,
            mood_applied: false,
            inited: false,
            spin: Spring::new(0.0),
            tx: Spring::new(0.0),
            ty: Spring::new(0.0),
            squash: Spring::new(1.0),
            blink: Spring::new(1.0),
            gaze_x: Spring::new(0.0),
            gaze_y: Spring::new(0.0),
            boost: Spring::new(1.0),
            face_from: neutral,
            face_to: neutral,
            face_key: FaceName::Neutral,
            face_s: Spring::new(1.0),
            body_from: circle,
            body_to: circle,
            body_key: BodyShape::Circle,
            body_s: Spring::new(1.0),
            face_idx: 0,
            body_idx: 0,
            body_list: [BodyShape::Circle; 4],
            body_len: 1,
            t0: 0.0,
            last: 0.0,
            face_until: 0.0,
            body_until: 0.0,
            blink_until: 0.0,
            gaze_until: 0.0,
            wink_at: -1.0e9,
            wink_eye: 0,
            wink_until: 0.0,
            wink_dur: 320.0,
            hop_at: -1.0,
            poke_heat: 0.0,
            poke_count: 0,
            last_poke_at: -1.0e9,
            heat_hold_until: 0.0,
            poke_shape: None,
            poke_shape_until: 0.0,
            blinks: BlinkQueue::new(),
            ctx: PoseCtx {
                nod_until: 0.0,
                nod_end: 0.0,
                impulse_at: 0.0,
                shake_until: 0.0,
                hop_until: 0.0,
                bias_until: 0.0,
                bias_spin: 0.0,
                bias_tx: 0.0,
                bias_ty: 0.0,
                bias_squash: 1.0,
                antic: None,
                antic_until: 0.0,
            },
        }
    }

    /// Resets cycles, morph targets, and the blink queue only when `mood` differs
    /// from the mood already applied. The first call applies even for [`Mood::Idle`].
    pub fn set_mood(&mut self, mood: Mood, now_ms: f64) {
        let mood = map_mood(mood);
        if self.mood_applied && mood == self.mood {
            return;
        }
        self.apply_mood(mood, now_ms);
    }

    pub fn poke(&mut self, now_ms: f64, look_x: Option<f64>) -> u32 {
        if now_ms - self.last_poke_at > 4200.0 {
            self.poke_count = 0;
        }
        self.poke_count += 1;
        self.last_poke_at = now_ms;
        let add = if self.poke_count < 4 { 0.16 } else { 0.22 };
        self.poke_heat = (self.poke_heat + add).min(1.0);
        self.heat_hold_until = now_ms + 3200.0;
        let stretch = if self.mood == BlobMood::Idle {
            2.6
        } else {
            1.0
        };
        self.queue_blink(now_ms, stretch);
        let dir = if let Some(x) = look_x {
            if x < 0.0 {
                -1.0
            } else {
                1.0
            }
        } else {
            self.sign()
        };
        let glance = if let Some(x) = look_x {
            clamp(x.abs(), 0.35, 1.0)
        } else {
            self.rand(0.45, 1.0)
        };
        let gaze_x = dir * self.rand(8.0, 16.0) * glance;
        let gaze_y = self.rand(-6.0, 4.0);
        let gaze_lo = if self.mood == BlobMood::Idle {
            1400.0
        } else {
            700.0
        };
        let gaze_hi = if self.mood == BlobMood::Idle {
            2800.0
        } else {
            1400.0
        };
        let gaze_for = self.rand(gaze_lo, gaze_hi);
        self.gaze_x.t = gaze_x;
        self.gaze_y.t = gaze_y;
        self.gaze_until = now_ms + gaze_for;
        self.hop_at = now_ms;
        self.wink_at = now_ms;
        self.wink_eye = if self.unit() < 0.5 { 0 } else { 1 };
        self.wink_dur = if self.mood == BlobMood::Idle {
            700.0
        } else {
            320.0
        };
        if self.poke_count >= 7 {
            self.poke_shape = Some(BodyShape::Burst);
            self.poke_shape_until = now_ms + 1400.0;
        } else if self.poke_count >= 4 {
            self.poke_shape = Some(BodyShape::Squircle);
            self.poke_shape_until = now_ms + 1800.0;
        } else {
            self.poke_shape = Some(BodyShape::Puddle);
            self.poke_shape_until = now_ms
                + if self.mood == BlobMood::Idle {
                    520.0
                } else {
                    300.0
                };
        }
        let face = if self.poke_count >= 7 {
            FaceName::Dizzy
        } else if self.poke_count >= 4 {
            FaceName::Lines
        } else if self.poke_count >= 2 {
            FaceName::Smirk
        } else {
            FaceName::Flat
        };
        self.retarget_face(face);
        let face_for = self.rand(1400.0, 2600.0);
        self.face_until = now_ms + face_for;
        if self.poke_count >= 5 {
            self.ctx.shake_until = now_ms + 420.0;
        }
        if self.mood == BlobMood::Idle {
            let spin = dir * self.rand(8.0, 16.0);
            let tx = dir * self.rand(4.0, 9.0);
            let bias_for = self.span(2500.0, 8000.0);
            self.ctx.bias_spin = spin;
            self.ctx.bias_tx = tx;
            self.ctx.bias_ty = -3.0;
            self.ctx.bias_squash = 1.02;
            self.ctx.bias_until = now_ms + bias_for;
            self.ctx.antic = None;
        }
        self.poke_count
    }

    pub fn sample(&mut self, now_ms: f64) -> Visual {
        self.ensure_init(now_ms);
        let dt = self.advance_clock(now_ms);
        let pose = self.apply_pose(now_ms);
        self.assign_targets(&pose);
        let blinkable = self.advance_schedule(now_ms, pose.lid);
        self.integrate(dt);
        if self.reduced {
            self.snap_reduced();
        }
        self.cool_heat(now_ms, dt);
        self.paint(now_ms, blinkable)
    }

    pub fn wants_high_fps(&self, now_ms: f64) -> bool {
        if self.reduced {
            return false;
        }
        if self.blinks.len > 0 {
            return true;
        }
        if now_ms < self.wink_at + self.wink_dur {
            return true;
        }
        if self.hop_at >= 0.0 {
            return true;
        }
        if now_ms < self.ctx.shake_until {
            return true;
        }
        if self.poke_shape.is_some() && now_ms < self.poke_shape_until + 600.0 {
            return true;
        }
        if self.mood == BlobMood::Idle {
            return false;
        }
        if !(self.face_s.x > 0.97) || !(self.body_s.x > 0.97) {
            return true;
        }
        if now_ms < self.ctx.nod_end {
            return true;
        }
        if self.mood == BlobMood::Error {
            return now_ms < self.ctx.shake_until;
        }
        true
    }

    fn ensure_init(&mut self, now: f64) {
        if self.inited {
            return;
        }
        self.t0 = now;
        self.last = now;
        self.apply_mood(self.mood, now);
        let wink = if self.mood == BlobMood::Idle {
            self.span(8_000.0, 22_000.0)
        } else {
            self.rand(4_000.0, 8_000.0)
        };
        self.wink_until = now + wink;
        self.inited = true;
    }

    fn advance_clock(&mut self, now: f64) -> f64 {
        let mut dt = (now - self.last) / 1000.0;
        if dt > 0.08 {
            dt = 0.08;
        }
        self.last = now;
        dt
    }

    fn apply_mood(&mut self, mood: BlobMood, now: f64) {
        self.mood = mood;
        self.mood_applied = true;
        let faces = face_play(mood);
        self.face_idx = 0;
        self.body_idx = 0;
        let rolled = self.unit();
        if rolled < body_chance(mood) {
            self.set_body_list(body_play(mood));
        } else {
            self.body_list[0] = BodyShape::Circle;
            self.body_len = 1;
        }
        let body_shape = self.body_list[0];
        let face0 = faces[0];
        self.retarget_face(face0);
        self.retarget_body(body_shape);
        let face_hold = face_hold(mood);
        let face_delay = if mood == BlobMood::Idle {
            self.span(face_hold[0], face_hold[1])
        } else {
            self.rand(face_hold[0], face_hold[1])
        };
        self.face_until = now + face_delay;
        let body_hold = body_hold(mood);
        let body_delay = self.rand(body_hold[0], body_hold[1]);
        self.body_until = now + body_delay;
        let blink_delay = if mood == BlobMood::Idle {
            self.span(1_800.0, 6_000.0)
        } else {
            self.rand(900.0, 2_800.0)
        };
        self.blink_until = now + blink_delay;
        let gaze_delay = if mood == BlobMood::Idle {
            self.span(800.0, 4_200.0)
        } else {
            self.rand(280.0, 900.0)
        };
        self.gaze_until = now + gaze_delay;
        let hop_until = if let Some(every) = hop_cadence(mood) {
            let delay = if mood == BlobMood::Idle {
                self.span(every[0], every[1])
            } else {
                self.rand(every[0], every[1])
            };
            now + delay
        } else {
            now + 1.0e12
        };
        let bias_until = if mood == BlobMood::Idle {
            now + self.span(3_000.0, 10_000.0)
        } else {
            1.0e12
        };
        self.ctx = PoseCtx {
            nod_until: now + 1_600.0,
            nod_end: 0.0,
            impulse_at: now + 600.0,
            shake_until: 0.0,
            hop_until,
            bias_until,
            bias_spin: 0.0,
            bias_tx: 0.0,
            bias_ty: 0.0,
            bias_squash: 1.0,
            antic: None,
            antic_until: 0.0,
        };
        self.blinks.clear();
        if mood == BlobMood::Done {
            self.hop_at = now;
        } else if face_blinks(face0) {
            let stretch = if mood == BlobMood::Idle { 2.6 } else { 1.0 };
            self.queue_blink(now, stretch);
        }
    }

    fn apply_pose(&mut self, now: f64) -> Pose {
        let mt = (now - self.t0) / 1000.0;
        let mut spin;
        let mut tx = 0.0;
        let mut ty;
        let mut squash = 1.0;
        let mut lid = 1.0;
        let mut boost = 1.0;
        match self.mood {
            BlobMood::Idle => {
                let br = (mt * 0.48).sin() * 0.62 + (mt * 0.91).sin() * 0.38;
                spin = (mt * 0.13).sin() * 2.0 + self.ctx.bias_spin;
                tx = (mt * 0.11).sin() * 1.6 + self.ctx.bias_tx;
                ty = br * 2.2 + self.ctx.bias_ty;
                squash = 1.0 + br * 0.016 + (self.ctx.bias_squash - 1.0);
                if now < self.ctx.shake_until {
                    spin += (now * 0.055).sin() * 8.0;
                    tx += (now * 0.08).sin() * 5.0;
                }
            }
            BlobMood::Think => {
                spin = -6.0 + (mt * 0.35).sin() * 5.0;
                tx = (mt * 0.3).sin() * 8.0;
                ty = -2.0 + (mt * 0.6).sin() * 4.0;
            }
            BlobMood::Search => {
                let et = (mt * 1.3).sin();
                spin = et * 16.0;
                tx = et * 10.0;
                ty = (mt * 1.7).sin() * 4.0;
            }
            BlobMood::Work => {
                let et = (mt * PI * 2.0 * 1.6).sin();
                spin = 5.0 + et * 3.5;
                tx = 4.0;
                ty = 2.0 + et.max(0.0) * 4.0;
                squash = 1.0 - et.max(0.0) * 0.03;
            }
            BlobMood::Speak => {
                spin = 10.0 + (mt * 0.5).sin() * 2.0;
                tx = 3.0;
                ty = -2.5 + (mt * 0.8).sin() * 1.1;
                squash = 1.018;
                boost = 1.05;
                if now >= self.ctx.nod_until {
                    let delay = self.rand(1_800.0, 3_200.0);
                    self.ctx.nod_until = now + delay;
                    self.ctx.nod_end = now + 380.0;
                }
                if now < self.ctx.nod_end {
                    let et = 1.0 - (self.ctx.nod_end - now) / 380.0;
                    let s = (et * PI).sin();
                    ty += s * 6.0;
                    spin += s * 3.0;
                }
            }
            BlobMood::Done => {
                let et = (mt * 2.4).sin();
                spin = (mt * 1.2).sin() * 4.0;
                tx = (mt * 1.1).sin() * 2.5;
                ty = -et.abs() * 3.5;
                squash = 1.0 + et * 0.025;
                boost = 1.06;
            }
            BlobMood::Wait => {
                spin = 13.0 + (mt * 0.45).sin() * 3.0;
                tx = 3.0;
                ty = -1.0 + (mt * 0.7).sin() * 0.8;
                squash = 1.01;
                boost = 1.03;
            }
            BlobMood::Error => {
                if now >= self.ctx.impulse_at {
                    let delay = self.rand(1_800.0, 3_200.0);
                    self.ctx.shake_until = now + 420.0;
                    self.ctx.impulse_at = now + delay;
                }
                spin = if now < self.ctx.shake_until {
                    (now * 0.05).sin() * 6.0
                } else {
                    0.0
                };
                ty = 4.0;
                squash = 0.972;
                lid = 0.92;
            }
        }
        Pose {
            spin,
            tx,
            ty,
            squash,
            lid,
            boost,
        }
    }

    fn assign_targets(&mut self, pose: &Pose) {
        self.spin.t = pose.spin;
        self.tx.t = pose.tx;
        self.ty.t = pose.ty;
        self.squash.t = pose.squash;
        self.boost.t = pose.boost;
        if self.mood == BlobMood::Idle {
            self.spin.t += self.gaze_x.t * 0.48;
            self.tx.t += self.gaze_x.t * 0.3;
            self.ty.t += self.gaze_y.t * 0.16;
        }
    }

    fn advance_schedule(&mut self, now: f64, pose_lid: f64) -> bool {
        if now >= self.face_until {
            let list = face_play(self.mood);
            self.face_idx = (self.face_idx + 1) % list.len();
            let face = list[self.face_idx];
            self.retarget_face(face);
            let hold = face_hold(self.mood);
            let delay = if self.mood == BlobMood::Idle {
                self.span(hold[0], hold[1])
            } else {
                self.rand(hold[0], hold[1])
            };
            self.face_until = now + delay;
        }
        if now >= self.body_until {
            self.body_idx = (self.body_idx + 1) % self.body_len;
            let hold = body_hold(self.mood);
            let delay = self.rand(hold[0], hold[1]);
            self.body_until = now + delay;
        }
        if self.poke_shape.is_some() && now >= self.poke_shape_until {
            self.poke_shape = None;
        }
        if self.ctx.antic.is_some() && now >= self.ctx.antic_until {
            self.ctx.antic = None;
        }
        let body_shape = self.body_target(now);
        self.retarget_body(body_shape);

        let blinkable = face_blinks(self.face_key);
        if let Some(cad) = blink_cadence(self.mood) {
            if now >= self.blink_until {
                if blinkable {
                    let stretch = if self.mood == BlobMood::Idle {
                        2.6
                    } else {
                        1.0
                    };
                    self.queue_blink(now, stretch);
                }
                let delay = if self.mood == BlobMood::Idle {
                    self.span(cad[0], cad[1])
                } else {
                    self.rand(cad[0], cad[1])
                };
                self.blink_until = now + delay;
            }
        }
        let key = self.blinks.consume(now);
        if blinkable {
            if let Some(v) = key {
                self.blink.t = v;
            } else if self.blinks.len == 0 {
                self.blink.t = pose_lid;
            }
        } else {
            self.blink.t = 1.0;
        }

        if now >= self.gaze_until {
            let gz = self.next_gaze();
            let delay = if self.mood == BlobMood::Idle {
                self.span(gz.hold[0], gz.hold[1])
            } else {
                self.rand(gz.hold[0], gz.hold[1])
            };
            self.gaze_x.t = gz.x;
            self.gaze_y.t = gz.y;
            self.gaze_until = now + delay;
        }
        if wink_mood(self.mood) && now >= self.wink_until {
            if blinkable {
                self.wink_at = now;
                self.wink_eye = if self.unit() < 0.5 { 0 } else { 1 };
                self.wink_dur = if self.mood == BlobMood::Idle {
                    700.0
                } else {
                    320.0
                };
            }
            let delay = if self.mood == BlobMood::Idle {
                self.span(8_000.0, 24_000.0)
            } else {
                self.rand(4_500.0, 10_000.0)
            };
            self.wink_until = now + delay;
        }
        if let Some(every) = hop_cadence(self.mood) {
            if now >= self.ctx.hop_until && self.hop_at < 0.0 {
                self.hop_at = now;
                let delay = if self.mood == BlobMood::Idle {
                    self.span(every[0], every[1])
                } else {
                    self.rand(every[0], every[1])
                };
                self.ctx.hop_until = now + delay;
            }
        }
        if self.mood == BlobMood::Idle && !self.reduced && now >= self.ctx.bias_until {
            let next = self.next_idle_bias();
            let bias_for = self.span(next.hold[0], next.hold[1]);
            self.ctx.bias_spin = next.spin;
            self.ctx.bias_tx = next.tx;
            self.ctx.bias_ty = next.ty;
            self.ctx.bias_squash = next.squash;
            self.ctx.bias_until = now + bias_for;
            if next.hop && self.hop_at < 0.0 {
                self.hop_at = now;
            }
            if let Some(shape) = next.antic {
                let antic_for = self.rand(next.hold[0], next.hold[1]);
                self.ctx.antic = Some(shape);
                self.ctx.antic_until = now + antic_for;
            }
        }
        blinkable
    }

    fn integrate(&mut self, dt: f64) {
        let n = (dt / DT).ceil().max(1.0);
        let n = if n.is_finite() { n as usize } else { 1 };
        let step = dt / n as f64;
        let spr = if self.mood == BlobMood::Idle {
            IDLE
        } else {
            ACTIVE
        };
        let body = if self.poke_shape.is_some() {
            ACTIVE.body
        } else {
            spr.body
        };
        for _ in 0..n {
            self.spin.step(spr.spin[0], spr.spin[1], step);
            self.tx.step(spr.x[0], spr.x[1], step);
            self.ty.step(spr.y[0], spr.y[1], step);
            self.squash.step(spr.squash[0], spr.squash[1], step);
            self.blink.step(spr.blink[0], spr.blink[1], step);
            self.gaze_x.step(spr.gaze[0], spr.gaze[1], step);
            self.gaze_y.step(spr.gaze[0], spr.gaze[1], step);
            self.face_s.step(spr.morph[0], spr.morph[1], step);
            self.body_s.step(body[0], body[1], step);
            self.boost.step(spr.boost[0], spr.boost[1], step);
        }
    }

    fn snap_reduced(&mut self) {
        self.spin.x = 0.0;
        self.tx.x = 0.0;
        self.ty.x = 0.0;
        self.squash.x = 1.0;
        self.blink.x = 1.0;
        self.gaze_x.x = 0.0;
        self.gaze_y.x = 0.0;
        self.boost.x = 1.0;
        self.hop_at = -1.0;
        let face = face_play(self.mood)[0];
        self.snap_face(face);
        self.snap_body(BodyShape::Circle);
    }

    fn cool_heat(&mut self, now: f64, dt: f64) {
        if now > self.heat_hold_until && self.poke_heat > 0.0 {
            self.poke_heat = (self.poke_heat - dt * 0.32).max(0.0);
            if self.poke_heat <= 0.01 {
                self.poke_heat = 0.0;
                self.poke_count = 0;
            }
        }
    }

    fn paint(&mut self, now: f64, blinkable: bool) -> Visual {
        let scale = if self.mood == BlobMood::Idle {
            0.36 + self.poke_heat * 0.5
        } else if self.mood == BlobMood::Done {
            0.7
        } else {
            1.0
        };
        let time_scale = if self.mood == BlobMood::Idle {
            2.1
        } else {
            1.0
        };
        let hop = match hop_y(self.hop_at, now, scale, time_scale) {
            Some(v) => v,
            None => {
                self.hop_at = -1.0;
                0.0
            }
        };
        let spin = self.spin.x;
        let tx = self.tx.x;
        let ty = self.ty.x + hop;
        let squash = self.squash.x;
        let face_k = ease(self.face_s.x.clamp(0.0, 1.0));
        let body_k = ease(self.body_s.x.clamp(0.0, 1.0));
        let pulse = 1.0
            + (if self.mood == BlobMood::Idle {
                0.03
            } else {
                0.07
            }) * (face_k * PI).sin();
        let boost = self.boost.x;
        let gx = self.gaze_x.x;
        let gy = self.gaze_y.x;
        let amp = if self.mood == BlobMood::Idle {
            0.28
        } else {
            1.0
        };
        let mut visual = Visual {
            body: [[0.0; 2]; 72],
            eyes: [[[0.0; 2]; 48]; 2],
            body_color: self.body_color(),
            eye_color: EYE,
        };
        lerp_body(&self.body_from, &self.body_to, body_k, &mut visual.body);
        for p in &mut visual.body {
            let y = CY + (p[1] - CY) * squash;
            *p = rig(p[0], y, spin, tx, ty);
        }
        for eye in 0..2 {
            lerp_eye(
                &self.face_from[eye],
                &self.face_to[eye],
                face_k,
                &mut visual.eyes[eye],
            );
            let mut lid = self.blink.x;
            if blinkable && eye == self.wink_eye && now < self.wink_at + self.wink_dur {
                let xr = (now - self.wink_at) / self.wink_dur;
                let fr = if xr < 0.42 {
                    1.0 - xr / 0.42
                } else {
                    (xr - 0.42) / 0.58
                };
                lid = lid.min(fr.max(0.04));
            }
            let wob_x = ((now * 42e-5 + eye as f64).sin() * 3.6
                + (now * 0.001 + eye as f64 * 2.0).sin() * 1.3)
                * amp;
            let wob_y = (now * 58e-5 + eye as f64).sin() * 2.2 * amp;
            let look_x = gx + wob_x;
            let look_y = gy + wob_y;
            let sx = (1.0 - gx.abs() * 0.012) * boost * pulse;
            let sy = clamp(lid, 0.04, 1.2) * boost * pulse;
            let c = centroid(&visual.eyes[eye]);
            for p in &mut visual.eyes[eye] {
                let x = c[0] + (p[0] - c[0]) * sx + look_x * 0.55;
                let y = c[1] + (p[1] - c[1]) * sy + look_y * 0.45;
                *p = rig(x, y, spin, tx, ty);
            }
        }
        visual
    }

    fn body_color(&self) -> Color {
        if self.mood == BlobMood::Error {
            return ERROR;
        }
        if self.poke_heat <= 0.0 {
            return BLUE;
        }
        mix_rgb(BLUE_RGB, RED_RGB, self.poke_heat)
    }

    fn body_target(&self, now: f64) -> BodyShape {
        if let Some(shape) = self.poke_shape {
            return shape;
        }
        if self.mood == BlobMood::Error {
            return if now < self.ctx.shake_until {
                BodyShape::Burst
            } else {
                BodyShape::Puddle
            };
        }
        if self.mood == BlobMood::Idle {
            return self.ctx.antic.unwrap_or(BodyShape::Circle);
        }
        self.body_list[self.body_idx % self.body_len]
    }

    fn retarget_face(&mut self, face: FaceName) {
        if face == self.face_key {
            return;
        }
        let k = ease(self.face_s.x.clamp(0.0, 1.0));
        for eye in 0..2 {
            for i in 0..48 {
                let a = self.face_from[eye][i];
                let b = self.face_to[eye][i];
                self.face_from[eye][i] = [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k];
            }
        }
        self.face_to = *face_points(face);
        self.face_key = face;
        self.face_s.x = 0.0;
        self.face_s.v = 0.0;
        self.face_s.t = 1.0;
    }

    fn retarget_body(&mut self, shape: BodyShape) {
        if shape == self.body_key {
            return;
        }
        let k = ease(self.body_s.x.clamp(0.0, 1.0));
        for i in 0..72 {
            let a = self.body_from[i];
            let b = self.body_to[i];
            self.body_from[i] = [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k];
        }
        self.body_to = *body_points(shape);
        self.body_key = shape;
        self.body_s.x = 0.0;
        self.body_s.v = 0.0;
        self.body_s.t = 1.0;
    }

    fn snap_face(&mut self, face: FaceName) {
        let pts = *face_points(face);
        self.face_key = face;
        self.face_from = pts;
        self.face_to = pts;
        self.face_s.x = 1.0;
        self.face_s.v = 0.0;
        self.face_s.t = 1.0;
    }

    fn snap_body(&mut self, shape: BodyShape) {
        let pts = *body_points(shape);
        self.body_key = shape;
        self.body_from = pts;
        self.body_to = pts;
        self.body_s.x = 1.0;
        self.body_s.v = 0.0;
        self.body_s.t = 1.0;
    }

    fn set_body_list(&mut self, shapes: &[BodyShape]) {
        debug_assert!(!shapes.is_empty() && shapes.len() <= 4);
        self.body_len = shapes.len();
        for (i, shape) in shapes.iter().enumerate() {
            self.body_list[i] = *shape;
        }
    }

    fn queue_blink(&mut self, now: f64, stretch: f64) {
        let s = stretch;
        self.blinks.push(now, 0.05);
        self.blinks.push(now + 70.0 * s, 0.05);
        self.blinks.push(now + 150.0 * s, 1.08);
        self.blinks.push(now + 300.0 * s, 1.0);
        if self.unit() < 0.14 {
            self.blinks.push(now + 370.0 * s, 0.05);
            self.blinks.push(now + 480.0 * s, 1.0);
        }
    }

    fn next_gaze(&mut self) -> Gaze {
        match self.mood {
            BlobMood::Idle => {
                if self.rand(0.0, 1.0) < 0.52 {
                    let d = self.sign();
                    Gaze {
                        x: d * self.rand(0.22, 0.72) * 11.0,
                        y: self.rand(-0.4, 0.32) * 7.0,
                        hold: [2_000.0, 10_000.0],
                    }
                } else {
                    Gaze {
                        x: 0.0,
                        y: 0.0,
                        hold: [3_000.0, 14_000.0],
                    }
                }
            }
            BlobMood::Think => {
                let d = self.sign();
                Gaze {
                    x: d * self.rand(0.5, 1.0) * 16.0,
                    y: -self.rand(0.4, 1.0) * 10.0,
                    hold: [1_500.0, 2_800.0],
                }
            }
            BlobMood::Search => {
                let d = self.sign();
                Gaze {
                    x: d * self.rand(0.7, 1.0) * 16.0,
                    y: self.rand(-1.0, 1.0) * 10.0,
                    hold: [550.0, 1_150.0],
                }
            }
            BlobMood::Work => Gaze {
                x: self.rand(-0.4, 0.4) * 16.0,
                y: self.rand(0.4, 1.0) * 10.0,
                hold: [1_200.0, 2_400.0],
            },
            BlobMood::Speak => Gaze {
                x: self.rand(-0.3, 0.3) * 16.0,
                y: self.rand(-0.25, 0.25) * 10.0,
                hold: [2_200.0, 4_200.0],
            },
            BlobMood::Done => Gaze {
                x: self.rand(-0.3, 0.3) * 16.0,
                y: -self.rand(0.1, 0.5) * 10.0,
                hold: [900.0, 1_600.0],
            },
            BlobMood::Wait => Gaze {
                x: self.rand(-0.15, 0.15) * 16.0,
                y: -self.rand(0.5, 0.9) * 10.0,
                hold: [1_800.0, 3_600.0],
            },
            BlobMood::Error => Gaze {
                x: self.rand(-0.2, 0.2) * 16.0,
                y: 2.0,
                hold: [1_800.0, 3_200.0],
            },
        }
    }

    fn next_idle_bias(&mut self) -> IdleBias {
        let r = self.rand(0.0, 1.0);
        if r < 0.38 {
            return IdleBias {
                spin: 0.0,
                tx: 0.0,
                ty: 0.0,
                squash: 1.0,
                hold: [4_000.0, 14_000.0],
                hop: false,
                antic: None,
            };
        }
        if r < 0.66 {
            let d = self.sign();
            return IdleBias {
                spin: d * self.rand(6.0, 14.0),
                tx: d * self.rand(2.5, 7.0),
                ty: self.rand(-1.5, 1.2),
                squash: 1.0,
                hold: [3_500.0, 12_000.0],
                hop: false,
                antic: None,
            };
        }
        if r < 0.78 {
            return IdleBias {
                spin: self.rand(-3.0, 3.0),
                tx: 0.0,
                ty: -self.rand(1.5, 4.0),
                squash: 1.02,
                hold: [2_800.0, 9_000.0],
                hop: false,
                antic: None,
            };
        }
        if r < 0.86 {
            return IdleBias {
                spin: 0.0,
                tx: 0.0,
                ty: self.rand(1.5, 4.0),
                squash: 0.984,
                hold: [3_500.0, 11_000.0],
                hop: false,
                antic: None,
            };
        }
        if r < 0.94 {
            let d = self.sign();
            return IdleBias {
                spin: d * self.rand(4.0, 10.0),
                tx: d * self.rand(2.0, 5.0),
                ty: -2.0,
                squash: 1.0,
                hold: [4_000.0, 13_000.0],
                hop: true,
                antic: None,
            };
        }
        let n = IDLE_ANTICS.len();
        let raw = self.rand(0.0, n as f64).floor();
        let shape = IDLE_ANTICS[(raw as usize).min(n - 1)];
        let spin = if shape == BodyShape::Cloud {
            self.rand(-4.0, 4.0)
        } else {
            let d = self.sign();
            d * self.rand(3.0, 8.0)
        };
        IdleBias {
            spin,
            tx: 0.0,
            ty: if shape == BodyShape::Cloud { -3.0 } else { 0.0 },
            squash: 1.0,
            hold: [3_500.0, 7_500.0],
            hop: false,
            antic: Some(shape),
        }
    }

    fn unit(&mut self) -> f64 {
        self.rng.next_f64()
    }

    fn rand(&mut self, a: f64, b: f64) -> f64 {
        a + self.unit() * (b - a)
    }

    fn span(&mut self, a: f64, b: f64) -> f64 {
        let u = self.unit();
        let t = 1.0 - (1.0 - u) * (1.0 - u);
        a + (b - a) * t
    }

    fn sign(&mut self) -> f64 {
        if self.unit() < 0.5 {
            -1.0
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blink_key(sim: &BlobSim, index: usize) -> (f64, f64) {
        let slot = (sim.blinks.head + index) % BLINK_CAP;
        (sim.blinks.at[slot], sim.blinks.v[slot])
    }

    fn mean_y(pts: &[[f64; 2]; 72]) -> f64 {
        pts.iter().map(|p| p[1]).sum::<f64>() / 72.0
    }

    #[test]
    fn splitmix64_seed_42_matches_the_53_bit_draw() {
        let mut rng = SplitMix64::new(42);
        assert_eq!(rng.next_u64() >> 11, 6_679_422_623_415_661);
        let unit = (6_679_422_623_415_661u64 as f64) / ((1u64 << 53) as f64);
        let mut rng = SplitMix64::new(42);
        assert_eq!(rng.next_f64(), unit);
    }

    #[test]
    fn same_seed_replays_and_a_different_seed_changes_the_poke_gaze() {
        fn drive(seed: u64) -> (Visual, f64) {
            let mut sim = BlobSim::with_seed(false, seed);
            let mut frame = sim.sample(0.0);
            for step in 1..=40 {
                let now = step as f64 * 16.0;
                let mood = if step < 30 {
                    Mood::Idle
                } else {
                    Mood::Thinking
                };
                sim.set_mood(mood, now);
                if step == 20 {
                    sim.poke(now, Some(-0.8));
                }
                frame = sim.sample(now);
            }
            (frame, sim.gaze_x.t)
        }
        let (a, gaze_a) = drive(42);
        let (b, gaze_b) = drive(42);
        let (c, gaze_c) = drive(99);
        assert_eq!(a, b);
        assert_eq!(gaze_a, gaze_b);
        assert_ne!(gaze_a, gaze_c);
        assert!(a.body_color.a == 1.0 && a.eye_color.a == 1.0);
    }

    #[test]
    fn repeated_set_mood_does_not_restart_the_morph_spring() {
        let mut sim = BlobSim::with_seed(false, 9);
        sim.set_mood(Mood::Thinking, 0.0);
        let until = sim.face_until;
        assert_eq!(sim.face_key, FaceName::LookUp);
        assert_eq!(sim.face_s.x, 0.0);
        sim.set_mood(Mood::Thinking, 50.0);
        assert_eq!(sim.face_until, until);
        assert_eq!(sim.face_s.x, 0.0);
        sim.sample(0.0);
        for step in 1..=20 {
            let now = step as f64 * 16.0;
            sim.set_mood(Mood::Thinking, now);
            sim.sample(now);
        }
        assert!(sim.face_s.x > 0.15);
        assert!(sim.spin.x < -2.0);
    }

    #[test]
    fn reduced_motion_is_a_scaled_circle_and_neutral_eyes_plus_wobble() {
        let mut sim = BlobSim::with_seed(true, 1);
        let frame = sim.sample(0.0);
        assert_eq!(sim.spin.x, 0.0);
        assert_eq!(sim.tx.x, 0.0);
        assert_eq!(sim.ty.x, 0.0);
        assert_eq!(sim.squash.x, 1.0);
        assert_eq!(sim.blink.x, 1.0);
        assert_eq!(sim.body_key, BodyShape::Circle);
        assert_eq!(sim.face_key, FaceName::Neutral);
        assert_eq!(frame.body_color, BLUE);
        assert_eq!(frame.eye_color, EYE);
        let scale = SIZE / 240.0;
        let body = body_points(BodyShape::Circle);
        for i in 0..72 {
            assert!((frame.body[i][0] - body[i][0] * scale).abs() < 1e-9);
            assert!((frame.body[i][1] - body[i][1] * scale).abs() < 1e-9);
        }
        let eyes = face_points(FaceName::Neutral);
        for i in 0..48 {
            assert!((frame.eyes[0][i][0] - eyes[0][i][0] * scale).abs() < 1e-9);
            assert!((frame.eyes[0][i][1] - eyes[0][i][1] * scale).abs() < 1e-9);
        }
        let amp = 0.28;
        let wob_x = (1.0_f64.sin() * 3.6 + 2.0_f64.sin() * 1.3) * amp;
        let wob_y = 1.0_f64.sin() * 2.2 * amp;
        let mut cx = 0.0;
        let mut cy = 0.0;
        for p in &eyes[1] {
            cx += p[0];
            cy += p[1];
        }
        cx /= 48.0;
        cy /= 48.0;
        for i in 0..48 {
            let x = cx + (eyes[1][i][0] - cx) + wob_x * 0.55;
            let y = cy + (eyes[1][i][1] - cy) + wob_y * 0.45;
            assert!((frame.eyes[1][i][0] - x * scale).abs() < 1e-9);
            assert!((frame.eyes[1][i][1] - y * scale).abs() < 1e-9);
        }
    }

    #[test]
    fn reduced_motion_pins_pose_and_cancels_hop() {
        let mut sim = BlobSim::with_seed(true, 5);
        sim.set_mood(Mood::Thinking, 0.0);
        let frame = sim.sample(800.0);
        assert_eq!(sim.spin.x, 0.0);
        assert_eq!(sim.blink.x, 1.0);
        assert_eq!(sim.face_key, FaceName::LookUp);
        assert_eq!(sim.body_key, BodyShape::Circle);
        assert!(!sim.wants_high_fps(800.0));
        let scale = SIZE / 240.0;
        let body = body_points(BodyShape::Circle);
        assert!((frame.body[0][0] - body[0][0] * scale).abs() < 1e-9);

        let mut sim = BlobSim::with_seed(true, 5);
        sim.set_mood(Mood::Done, 0.0);
        let a = sim.sample(0.0);
        let b = sim.sample(250.0);
        assert_eq!(a.body, b.body);
        assert_eq!(sim.hop_at, -1.0);
        assert_eq!(sim.face_key, FaceName::Happy);
        assert_eq!(sim.blink.x, 1.0);
    }

    #[test]
    fn poke_heat_shape_and_gap_follow_the_source_thresholds() {
        let mut sim = BlobSim::with_seed(false, 4);
        sim.sample(0.0);
        assert_eq!(sim.sample(10.0).body_color, BLUE);
        assert_eq!(sim.poke(100.0, None), 1);
        assert!(sim.poke_heat > 0.0);
        assert_ne!(sim.sample(120.0).body_color, BLUE);
        assert!(sim.wants_high_fps(120.0));
        for i in 1..6 {
            sim.poke(100.0 + i as f64 * 80.0, None);
        }
        assert_eq!(sim.poke_count, 6);
        assert_eq!(sim.poke_heat, 1.0);
        assert_eq!(sim.sample(600.0).body_color, rgb(RED_RGB));
        assert_eq!(sim.poke(6000.0, None), 1);

        let mut sim = BlobSim::with_seed(false, 4);
        sim.set_mood(Mood::Idle, 0.0);
        sim.sample(0.0);
        sim.poke(100.0, Some(-0.8));
        assert!(sim.gaze_x.t < 0.0);
        assert_eq!(sim.face_key, FaceName::Flat);
        sim.sample(116.0);
        assert_eq!(sim.body_key, BodyShape::Puddle);
        for i in 1..4 {
            sim.poke(100.0 + i as f64 * 80.0, Some(-0.8));
        }
        sim.sample(400.0);
        assert_eq!(sim.body_key, BodyShape::Squircle);
        assert_eq!(sim.face_key, FaceName::Lines);
        for i in 4..7 {
            sim.poke(100.0 + i as f64 * 80.0, Some(0.4));
        }
        assert!(sim.gaze_x.t > 0.0);
        sim.sample(700.0);
        assert_eq!(sim.body_key, BodyShape::Burst);
        assert_eq!(sim.face_key, FaceName::Dizzy);
        let mut t = 716.0;
        while t <= 3000.0 {
            sim.sample(t);
            t += 16.0;
        }
        assert_eq!(sim.body_key, BodyShape::Circle);
    }

    #[test]
    fn capped_dt_limits_heat_decay_and_keeps_the_spring_finite() {
        let mut sim = BlobSim::with_seed(false, 3);
        sim.sample(0.0);
        sim.poke(100.0, Some(0.0));
        assert!(sim.gaze_x.t > 0.0);
        sim.sample(10_100.0);
        assert_eq!(sim.poke_heat, 0.16 - 0.08 * 0.32);

        let mut sim = BlobSim::with_seed(false, 3);
        sim.set_mood(Mood::Thinking, 0.0);
        sim.sample(0.0);
        sim.sample(5_000.0);
        assert!(sim.spin.x.is_finite());
        assert!(sim.spin.x.abs() < 30.0);
    }

    #[test]
    fn mood_entry_faces_error_burst_and_work_retarget() {
        let face = |mood| {
            let mut sim = BlobSim::with_seed(false, 1);
            sim.set_mood(mood, 0.0);
            sim.face_key
        };
        assert_eq!(face(Mood::Thinking), FaceName::LookUp);
        assert_eq!(face(Mood::Searching), FaceName::Wide);
        assert_eq!(face(Mood::Working), FaceName::Focus);
        assert_eq!(face(Mood::Speaking), FaceName::Neutral);
        assert_eq!(face(Mood::Waiting), FaceName::Wide);
        assert_eq!(face(Mood::Done), FaceName::Happy);
        assert_eq!(face(Mood::Error), FaceName::Dizzy);

        let mut sim = BlobSim::with_seed(false, 1);
        sim.set_mood(Mood::Error, 0.0);
        assert_eq!(sim.body_key, BodyShape::Puddle);
        sim.sample(0.0);
        assert_eq!(sim.sample(600.0).body_color, ERROR);
        assert_eq!(sim.body_key, BodyShape::Burst);

        let mut rolled = None;
        for seed in 0..64 {
            let mut sim = BlobSim::with_seed(false, seed);
            sim.set_mood(Mood::Working, 0.0);
            if sim.body_key == BodyShape::Squircle {
                rolled = Some(seed);
                break;
            }
        }
        let seed = rolled.expect("work body chance");
        let mut sim = BlobSim::with_seed(false, seed);
        sim.set_mood(Mood::Working, 0.0);
        assert_eq!(sim.body_s.x, 0.0);
        assert_eq!(sim.body_from, *body_points(BodyShape::Circle));
        assert_eq!(sim.body_to, *body_points(BodyShape::Squircle));
        sim.sample(0.0);
        sim.sample(32.0);
        assert!(sim.body_s.x > 0.0 && sim.body_s.x < 0.97);
    }

    #[test]
    fn blink_keys_match_the_source_schedule_and_idle_fps_drops() {
        let mut saw_short = false;
        let mut saw_long = false;
        for seed in 0..80 {
            let mut sim = BlobSim::with_seed(false, seed);
            sim.set_mood(Mood::Speaking, 1_000.0);
            assert_eq!(blink_key(&sim, 0), (1_000.0, 0.05));
            assert_eq!(blink_key(&sim, 1), (1_070.0, 0.05));
            assert_eq!(blink_key(&sim, 2), (1_150.0, 1.08));
            assert_eq!(blink_key(&sim, 3), (1_300.0, 1.0));
            match sim.blinks.len {
                4 => saw_short = true,
                6 => {
                    saw_long = true;
                    assert_eq!(blink_key(&sim, 4), (1_370.0, 0.05));
                    assert_eq!(blink_key(&sim, 5), (1_480.0, 1.0));
                }
                other => panic!("blink len {other}"),
            }
        }
        assert!(saw_short && saw_long);

        let mut sim = BlobSim::with_seed(false, 2);
        sim.set_mood(Mood::Idle, 0.0);
        assert_eq!(blink_key(&sim, 2).0, 150.0 * 2.6);

        let mut sim = BlobSim::with_seed(false, 2);
        for t in (0..=1696).step_by(16) {
            sim.sample(t as f64);
        }
        assert!(!sim.wants_high_fps(1700.0));
        sim.set_mood(Mood::Thinking, 1700.0);
        assert!(sim.wants_high_fps(1700.0));
    }

    #[test]
    fn done_hops_up_and_closed_eyes_stay_open_scale() {
        let mut sim = BlobSim::with_seed(false, 6);
        sim.sample(0.0);
        sim.set_mood(Mood::Done, 1_000.0);
        assert_eq!(sim.hop_at, 1_000.0);
        assert!(sim.wants_high_fps(1_000.0));
        let y0 = mean_y(&sim.sample(1_000.0).body);
        let y1 = mean_y(&sim.sample(1_250.0).body);
        assert!(y1 < y0 - 1.0);
        assert_eq!(sim.blink.t, 1.0);
        for t in (1_266..=1_600).step_by(16) {
            sim.sample(t as f64);
        }
        assert!((sim.blink.x - 1.0).abs() < 1e-6);
    }

    #[test]
    fn searching_is_the_source_search_sweep_not_thinking() {
        let mut search = BlobSim::with_seed(false, 11);
        let mut think = BlobSim::with_seed(false, 11);
        search.set_mood(Mood::Searching, 0.0);
        think.set_mood(Mood::Thinking, 0.0);
        assert_eq!(search.face_key, FaceName::Wide);
        assert_eq!(think.face_key, FaceName::LookUp);
        assert_eq!(search.body_len, 1);
        assert_eq!(search.body_list[0], BodyShape::Circle);
        assert!((4_000.0..7_000.0).contains(&search.ctx.hop_until));
        assert!((8_000.0..12_000.0).contains(&think.ctx.hop_until));
        for step in 0..=80 {
            let now = f64::from(step) * 16.0;
            search.sample(now);
            think.sample(now);
        }
        // mt = 1.28s: search target is sin(1.28*1.3)*16 ≈ 15.9; think stays negative.
        assert!(search.spin.t > 14.0, "search spin {}", search.spin.t);
        assert!(think.spin.t < -2.0, "think spin {}", think.spin.t);
    }
}
