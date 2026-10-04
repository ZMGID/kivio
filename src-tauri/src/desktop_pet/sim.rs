//! Eight-state Momo sampled from the approved round preview.
//!
//! The body is the circle at viewBox `(158, 101)` with radius `53`. Eyes and props
//! are filled polygons in that same 240 space, then scaled into [`SIZE`]. Props stay
//! outside the body rig: a state may lean, breathe, lift, or shake the face without
//! moving the keyboard, glass, bubble, or badges. Rings use one even-odd contour.
//!
//! Time is the desktop pet's monotonic milliseconds. Looping motion follows that
//! clock. Done and Error store the moment the mood actually changed, so a repeated
//! [`BlobSim::set_mood`] does not replay them. Reduced motion keeps each state's
//! geometry and drops every transform, blink, pulse, and poke.
//!
//! [`BlobSim::sample`] builds one [`Visual`] from static outlines plus a few pose
//! sines. It does not allocate. [`BlobSim::wants_high_fps`] is what the frame loop
//! reads: continuous work stays on the 33 ms cadence, a settled Done or Error does
//! not, and Idle wakes only for a blink or a poke.

use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::sync::OnceLock;

use super::visual::{Color, Point, Visual, SIZE};
use super::Mood;

const VIEW: f64 = 240.0;
const SCALE: f64 = SIZE / VIEW;
const PIVOT_X: f64 = 157.0;
const PIVOT_Y: f64 = 154.0;
const GLASS_X: f64 = 76.0;
const GLASS_Y: f64 = 141.0;
const POKE_MS: f64 = 280.0;
const DONE_MS: f64 = 750.0;
const ERROR_MS: f64 = 400.0;

const fn rgba(r: u8, g: u8, b: u8, a: f64) -> Color {
    Color {
        r: r as f64 / 255.0,
        g: g as f64 / 255.0,
        b: b as f64 / 255.0,
        a,
    }
}

const BLUE: Color = rgba(0x34, 0x77, 0xeb, 1.0);
const CREAM: Color = rgba(0xf5, 0xf3, 0xea, 1.0);
const PROP: Color = rgba(0xb8, 0xc9, 0xdf, 1.0);
const INK: Color = rgba(0x24, 0x30, 0x44, 1.0);
const KEY: Color = rgba(0xbd, 0xcd, 0xe3, 1.0);
const BOARD: Color = rgba(0x56, 0x67, 0x82, 1.0);
const AMBER: Color = rgba(0xe9, 0xbc, 0x6c, 1.0);
const AMBER_INK: Color = rgba(0x5a, 0x42, 0x22, 1.0);
const GREEN: Color = rgba(0xa9, 0xcc, 0x9c, 1.0);
const GREEN_INK: Color = rgba(0x35, 0x57, 0x31, 1.0);
const CORAL: Color = rgba(0xe8, 0xa1, 0x99, 1.0);
const CORAL_INK: Color = rgba(0x68, 0x36, 0x30, 1.0);
const SHADOW: Color = rgba(0x20, 0x2e, 0x45, 0.25);

#[derive(Clone, Copy)]
struct Curve {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

const EASE: Curve = Curve {
    x1: 0.25,
    y1: 0.1,
    x2: 0.25,
    y2: 1.0,
};
const EASE_IN_OUT: Curve = Curve {
    x1: 0.42,
    y1: 0.0,
    x2: 0.58,
    y2: 1.0,
};
const EASE_OUT: Curve = Curve {
    x1: 0.0,
    y1: 0.0,
    x2: 0.58,
    y2: 1.0,
};

fn bezier(t: f64, c1: f64, c2: f64) -> f64 {
    let u = 1.0 - t;
    3.0 * u * u * t * c1 + 3.0 * u * t * t * c2 + t * t * t
}

fn ease(u: f64, curve: Curve) -> f64 {
    if u <= 0.0 {
        return 0.0;
    }
    if u >= 1.0 {
        return 1.0;
    }
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..16 {
        let mid = 0.5 * (lo + hi);
        if bezier(mid, curve.x1, curve.x2) < u {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier(0.5 * (lo + hi), curve.y1, curve.y2)
}

fn track(t: f64, stops: &[(f64, f64)], curve: Curve) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if stops.is_empty() || t <= stops[0].0 {
        return stops.first().map_or(0.0, |stop| stop.1);
    }
    let mut index = 1;
    while index < stops.len() && t > stops[index].0 {
        index += 1;
    }
    if index >= stops.len() {
        return stops[stops.len() - 1].1;
    }
    let (start, from) = stops[index - 1];
    let (end, to) = stops[index];
    let span = end - start;
    let u = if span <= 1.0e-9 {
        1.0
    } else {
        (t - start) / span
    };
    from + (to - from) * ease(u, curve)
}

fn finite(now: f64) -> f64 {
    if now.is_finite() {
        now
    } else {
        0.0
    }
}

fn phase(now: f64, delay: f64, period: f64) -> Option<f64> {
    if period <= 0.0 || now < delay {
        None
    } else {
        Some(((now - delay) / period).fract())
    }
}

fn fade(color: Color, alpha: f64) -> Color {
    Color {
        a: color.a * alpha,
        ..color
    }
}

#[derive(Clone, Copy)]
struct Poly {
    pts: [Point; 48],
    len: usize,
}

impl Poly {
    const EMPTY: Self = Self {
        pts: [[0.0; 2]; 48],
        len: 0,
    };
}

struct Path {
    pts: [Point; 160],
    len: usize,
}

impl Path {
    fn new() -> Self {
        Self {
            pts: [[0.0; 2]; 160],
            len: 0,
        }
    }

    fn push(&mut self, x: f64, y: f64) {
        if self.len > 0 {
            let last = self.pts[self.len - 1];
            if (last[0] - x).hypot(last[1] - y) <= 1.0e-4 {
                return;
            }
        }
        assert!(self.len < self.pts.len(), "pet geometry does not fit");
        self.pts[self.len] = [x, y];
        self.len += 1;
    }

    fn arc(&mut self, cx: f64, cy: f64, radius: f64, start: f64, sweep: f64, steps: usize) {
        let steps = steps.max(1);
        for step in 0..=steps {
            let angle = start + sweep * (step as f64 / steps as f64);
            let (sine, cosine) = angle.sin_cos();
            self.push(cx + radius * cosine, cy + radius * sine);
        }
    }

    fn quad(&mut self, from: Point, control: Point, to: Point, steps: usize) {
        let steps = steps.max(1);
        for step in 0..=steps {
            let t = step as f64 / steps as f64;
            let u = 1.0 - t;
            self.push(
                u * u * from[0] + 2.0 * u * t * control[0] + t * t * to[0],
                u * u * from[1] + 2.0 * u * t * control[1] + t * t * to[1],
            );
        }
    }
}

#[derive(Clone, Copy)]
struct Dir {
    lx: f64,
    ly: f64,
    ux: f64,
    uy: f64,
}

fn direction(dx: f64, dy: f64) -> Dir {
    let len = dx.hypot(dy).max(1.0e-9);
    Dir {
        ux: dx / len,
        uy: dy / len,
        lx: -dy / len,
        ly: dx / len,
    }
}

fn wrap_pi(mut turn: f64) -> f64 {
    while turn <= -PI {
        turn += TAU;
    }
    while turn > PI {
        turn -= TAU;
    }
    turn
}

fn sweep_via(start: f64, via: f64) -> f64 {
    let mut best = PI;
    let mut best_distance = f64::INFINITY;
    for turn in [PI, -PI] {
        let mid = start + turn * 0.5;
        let delta = via - mid;
        let distance = delta.sin().atan2(delta.cos()).abs();
        if distance < best_distance {
            best_distance = distance;
            best = turn;
        }
    }
    best
}

fn stroke(center: &[Point], width: f64, seg: usize) -> Poly {
    let mut out = Path::new();
    if center.len() < 2 || width <= 0.0 {
        return poly_from(&out);
    }
    let mut dirs = [Dir {
        lx: 0.0,
        ly: 0.0,
        ux: 0.0,
        uy: 0.0,
    }; 48];
    let segments = center.len() - 1;
    assert!(segments <= dirs.len(), "stroke centerline is too long");
    for index in 0..segments {
        dirs[index] = direction(
            center[index + 1][0] - center[index][0],
            center[index + 1][1] - center[index][1],
        );
    }
    let radius = width * 0.5;
    let first = dirs[0];
    let left = first.ly.atan2(first.lx);
    out.arc(
        center[0][0],
        center[0][1],
        radius,
        left,
        sweep_via(left, (-first.uy).atan2(-first.ux)),
        seg,
    );
    for index in 0..segments {
        let end = center[index + 1];
        if index + 1 < segments {
            let a0 = (-dirs[index].ly).atan2(-dirs[index].lx);
            let a1 = (-dirs[index + 1].ly).atan2(-dirs[index + 1].lx);
            let sweep = wrap_pi(a1 - a0);
            let steps = ((sweep.abs() / PI) * seg as f64).round().max(1.0) as usize;
            out.arc(end[0], end[1], radius, a0, sweep, steps);
        } else {
            out.push(
                end[0] - dirs[index].lx * radius,
                end[1] - dirs[index].ly * radius,
            );
        }
    }
    let last = dirs[segments - 1];
    let end = center[segments];
    let right = (-last.ly).atan2(-last.lx);
    out.arc(
        end[0],
        end[1],
        radius,
        right,
        sweep_via(right, last.uy.atan2(last.ux)),
        seg,
    );
    for index in (0..segments).rev() {
        let start = center[index];
        if index > 0 {
            let a0 = dirs[index].ly.atan2(dirs[index].lx);
            let a1 = dirs[index - 1].ly.atan2(dirs[index - 1].lx);
            let sweep = wrap_pi(a1 - a0);
            let steps = ((sweep.abs() / PI) * seg as f64).round().max(1.0) as usize;
            out.arc(start[0], start[1], radius, a0, sweep, steps);
        } else {
            out.push(
                start[0] + dirs[index].lx * radius,
                start[1] + dirs[index].ly * radius,
            );
        }
    }
    poly_from(&out)
}

fn poly_from(path: &Path) -> Poly {
    assert!(
        (3..49).contains(&path.len),
        "polygon has {} points",
        path.len
    );
    let mut poly = Poly::EMPTY;
    poly.len = path.len;
    poly.pts[..path.len].copy_from_slice(&path.pts[..path.len]);
    poly
}

fn curve(from: Point, control: Point, to: Point, steps: usize, width: f64, seg: usize) -> Poly {
    let mut center = Path::new();
    center.quad(from, control, to, steps);
    stroke(&center.pts[..center.len], width, seg)
}

fn eye_curve(from: Point, control: Point, to: Point) -> [Point; 48] {
    pad_eye(&curve(from, control, to, 8, 3.5, 3))
}

fn pad_eye(poly: &Poly) -> [Point; 48] {
    let mut eye = poly.pts;
    let last = eye[poly.len - 1];
    for slot in eye.iter_mut().skip(poly.len) {
        *slot = last;
    }
    eye
}

fn ellipse_eye(cx: f64, cy: f64, rx: f64, ry: f64, degrees: f64) -> [Point; 48] {
    let mut eye = [[0.0; 2]; 48];
    let (sine, cosine) = degrees.to_radians().sin_cos();
    const N: usize = 32;
    for index in 0..N {
        let angle = index as f64 * TAU / N as f64;
        let (ys, xs) = angle.sin_cos();
        let x = rx * xs;
        let y = ry * ys;
        eye[index] = [cx + x * cosine - y * sine, cy + x * sine + y * cosine];
    }
    let last = eye[N - 1];
    for slot in eye.iter_mut().skip(N) {
        *slot = last;
    }
    eye
}

fn circle(cx: f64, cy: f64, radius: f64, count: usize) -> Poly {
    let mut poly = Poly::EMPTY;
    let count = count.clamp(3, 48);
    poly.len = count;
    for index in 0..count {
        let angle = index as f64 * TAU / count as f64;
        let (sine, cosine) = angle.sin_cos();
        poly.pts[index] = [cx + radius * cosine, cy + radius * sine];
    }
    poly
}

fn ring(cx: f64, cy: f64, outer: f64, inner: f64, count: usize) -> Poly {
    // Outer loop, reversed inner loop, and the same radial slit out and back.
    // Even-odd cancels the slit, so the lens stays hollow.
    let mut poly = Poly::EMPTY;
    let count = count.clamp(3, 22);
    let shift = TAU / (count as f64 * 2.0);
    let at = |radius: f64, index: usize| {
        let angle = shift + index as f64 * TAU / count as f64;
        let (sine, cosine) = angle.sin_cos();
        [cx + radius * cosine, cy + radius * sine]
    };
    let mut len = 0;
    for index in 0..count {
        poly.pts[len] = at(outer, index);
        len += 1;
    }
    poly.pts[len] = at(outer, 0);
    len += 1;
    poly.pts[len] = at(inner, 0);
    len += 1;
    for index in (1..count).rev() {
        poly.pts[len] = at(inner, index);
        len += 1;
    }
    poly.pts[len] = at(inner, 0);
    len += 1;
    poly.len = len;
    poly
}

fn round_rect(x: f64, y: f64, w: f64, h: f64, radius: f64, seg: usize) -> Poly {
    let mut path = Path::new();
    let radius = radius.min(w * 0.5).min(h * 0.5);
    let corners = [
        (x + radius, y + radius, PI, FRAC_PI_2),
        (x + w - radius, y + radius, PI * 1.5, FRAC_PI_2),
        (x + w - radius, y + h - radius, 0.0, FRAC_PI_2),
        (x + radius, y + h - radius, FRAC_PI_2, FRAC_PI_2),
    ];
    for (cx, cy, start, sweep) in corners {
        path.arc(cx, cy, radius, start, sweep, seg);
    }
    poly_from(&path)
}

struct Caps {
    left: [[f64; 2]; 5],
    right: [[f64; 2]; 5],
}

impl Caps {
    fn build() -> Self {
        let mut caps = Self {
            left: [[0.0; 2]; 5],
            right: [[0.0; 2]; 5],
        };
        for index in 0..5 {
            let along = index as f64 / 4.0;
            let (sine, cosine) = (FRAC_PI_2 + PI * along).sin_cos();
            caps.left[index] = [cosine, sine];
            let (sine, cosine) = (-FRAC_PI_2 + PI * along).sin_cos();
            caps.right[index] = [cosine, sine];
        }
        caps
    }
}

fn capsule(x0: f64, x1: f64, y: f64, radius: f64, caps: &Caps) -> Poly {
    let mut poly = Poly::EMPTY;
    poly.len = 10;
    for index in 0..5 {
        poly.pts[index] = [
            x0 + caps.left[index][0] * radius,
            y + caps.left[index][1] * radius,
        ];
        poly.pts[5 + index] = [
            x1 + caps.right[index][0] * radius,
            y + caps.right[index][1] * radius,
        ];
    }
    poly
}

struct Art {
    body: [Point; 72],
    eyes_idle: [[Point; 48]; 2],
    eyes_think: [[Point; 48]; 2],
    eyes_base: [[Point; 48]; 2],
    eyes_done: [[Point; 48]; 2],
    eyes_error: [[Point; 48]; 2],
    dots: [Poly; 3],
    glass: Poly,
    handle: Poly,
    board: Poly,
    shadow: Poly,
    keys: [Poly; 14],
    space: Poly,
    bubble: Poly,
    caps: Caps,
    badge: Poly,
    question: Poly,
    qdot: Poly,
    check: Poly,
    bang: Poly,
    edot: Poly,
}

impl Art {
    fn build() -> Self {
        let caps = Caps::build();
        let mut question = Path::new();
        question.quad([80.0, 146.0], [80.0, 139.0], [86.0, 140.0], 4);
        question.quad([86.0, 140.0], [95.0, 142.0], [86.0, 149.0], 4);
        question.push(86.0, 152.0);
        let mut bubble = Path::new();
        bubble.push(48.0, 134.0);
        bubble.push(91.0, 134.0);
        bubble.quad([91.0, 134.0], [98.0, 134.0], [98.0, 141.0], 3);
        bubble.push(98.0, 158.0);
        bubble.quad([98.0, 158.0], [98.0, 165.0], [91.0, 165.0], 3);
        bubble.push(75.0, 165.0);
        bubble.push(66.0, 172.0);
        bubble.push(66.0, 165.0);
        bubble.push(48.0, 165.0);
        bubble.quad([48.0, 165.0], [41.0, 165.0], [41.0, 158.0], 3);
        bubble.push(41.0, 141.0);
        bubble.quad([41.0, 141.0], [41.0, 134.0], [48.0, 134.0], 3);
        let mut keys = [Poly::EMPTY; 14];
        for index in 0..14 {
            let column = (index % 7) as f64;
            let row = (index / 7) as f64;
            keys[index] = round_rect(9.0 + column * 12.0, 7.0 + row * 12.0, 8.0, 7.0, 1.5, 2);
        }
        Self {
            body: circle72(158.0, 101.0, 53.0),
            eyes_idle: [
                eye_curve([116.0, 114.0], [120.0, 119.0], [124.0, 115.0]),
                eye_curve([132.0, 119.0], [138.0, 125.0], [144.0, 120.0]),
            ],
            eyes_think: [
                ellipse_eye(120.0, 107.0, 4.0, 6.0, 0.0),
                ellipse_eye(139.0, 111.0, 6.0, 7.0, 0.0),
            ],
            eyes_base: [
                ellipse_eye(120.0, 114.0, 4.0, 7.0, 18.0),
                ellipse_eye(139.0, 119.0, 6.0, 8.0, 18.0),
            ],
            eyes_done: [
                eye_curve([116.0, 116.0], [120.0, 108.0], [124.0, 116.0]),
                eye_curve([132.0, 121.0], [138.0, 111.0], [144.0, 121.0]),
            ],
            eyes_error: [
                pad_eye(&stroke(&[[116.0, 114.0], [124.0, 117.0]], 3.5, 3)),
                pad_eye(&stroke(&[[133.0, 119.0], [145.0, 122.0]], 3.5, 3)),
            ],
            dots: [
                circle(83.0, 74.0, 4.0, 14),
                circle(97.0, 69.0, 4.0, 14),
                circle(111.0, 65.0, 4.0, 14),
            ],
            glass: ring(78.0, 144.0, 20.0, 14.0, 16),
            handle: stroke(&[[66.0, 157.0], [54.0, 169.0]], 6.0, 3),
            board: round_rect(0.0, 0.0, 100.0, 43.0, 4.0, 3),
            shadow: capsule(5.0, 95.0, 39.0, 1.0, &caps),
            keys,
            space: round_rect(29.0, 32.0, 43.0, 5.0, 1.5, 2),
            bubble: poly_from(&bubble),
            caps,
            badge: circle(85.0, 150.0, 17.0, 28),
            question: stroke(&question.pts[..question.len], 2.7, 2),
            qdot: circle(86.0, 158.0, 1.5, 8),
            check: stroke(&[[77.0, 150.0], [82.0, 155.0], [93.0, 143.0]], 3.0, 3),
            bang: stroke(&[[85.0, 141.0], [85.0, 151.0]], 3.0, 3),
            edot: circle(85.0, 158.0, 1.5, 8),
        }
    }
}

fn circle72(cx: f64, cy: f64, radius: f64) -> [Point; 72] {
    let mut body = [[0.0; 2]; 72];
    for index in 0..72 {
        let angle = index as f64 * TAU / 72.0;
        let (sine, cosine) = angle.sin_cos();
        body[index] = [cx + radius * cosine, cy + radius * sine];
    }
    body
}

fn art() -> &'static Art {
    static ART: OnceLock<Art> = OnceLock::new();
    ART.get_or_init(Art::build)
}

fn rig(src: &[Point], dst: &mut [Point], tx: f64, ty: f64, rot: f64) {
    let (sine, cosine) = rot.sin_cos();
    for (src, dst) in src.iter().zip(dst.iter_mut()) {
        let x = src[0] - PIVOT_X;
        let y = src[1] - PIVOT_Y;
        *dst = [
            ((x * cosine - y * sine) + PIVOT_X + tx) * SCALE,
            ((x * sine + y * cosine) + PIVOT_Y + ty) * SCALE,
        ];
    }
}

fn push_poly<F>(visual: &mut Visual, poly: &Poly, color: Color, map: F)
where
    F: Fn(Point) -> Point,
{
    if poly.len < 3 || color.a <= 0.0 {
        return;
    }
    let mut mapped = [[0.0; 2]; 48];
    let len = poly.len.min(48);
    for index in 0..len {
        let point = map(poly.pts[index]);
        mapped[index] = [point[0] * SCALE, point[1] * SCALE];
    }
    visual.push_polygon(&mapped[..len], color);
}

fn keyboard(point: Point, local_y: f64) -> Point {
    let y = point[1] + local_y;
    [
        0.80 * point[0] - 0.66 * y + 78.0,
        0.32 * point[0] + 0.48 * y + 136.0,
    ]
}

fn glass_at(point: Point, tx: f64, ty: f64, rot: f64) -> Point {
    let (sine, cosine) = rot.sin_cos();
    let x = point[0] - GLASS_X;
    let y = point[1] - GLASS_Y;
    [
        (x * cosine - y * sine) + GLASS_X + tx,
        (x * sine + y * cosine) + GLASS_Y + ty,
    ]
}

struct Pose {
    tx: f64,
    ty: f64,
    rot: f64,
    blink: f64,
}

pub struct BlobSim {
    reduced: bool,
    mood: Mood,
    mood_applied: bool,
    mood_at: f64,
    poke_at: f64,
    poke_until: f64,
    pokes: u32,
    rest_since: f64,
}

impl BlobSim {
    pub fn new(reduced_motion: bool) -> Self {
        Self {
            reduced: reduced_motion,
            mood: Mood::Idle,
            mood_applied: false,
            mood_at: 0.0,
            poke_at: -1.0e9,
            poke_until: 0.0,
            pokes: 0,
            rest_since: 0.0,
        }
    }

    /// Records `now_ms` only when `mood` differs from the mood already showing.
    pub fn set_mood(&mut self, mood: Mood, now_ms: f64) {
        if self.mood_applied && self.mood == mood {
            return;
        }
        self.mood = mood;
        self.mood_applied = true;
        self.mood_at = finite(now_ms);
        self.rest_since = self.mood_at;
    }

    /// Short rigid lift of the face. `look_x` is ignored: gaze stays with desktop
    /// behavior, and the active prop is left where [`BlobSim::sample`] put it.
    pub fn poke(&mut self, now_ms: f64, look_x: Option<f64>) -> u32 {
        let _ = look_x;
        let now = finite(now_ms);
        self.rest_since = now;
        self.pokes = self.pokes.saturating_add(1);
        self.poke_at = now;
        self.poke_until = now + POKE_MS;
        self.pokes
    }

    pub(super) fn wake(&mut self, now_ms: f64) {
        self.rest_since = finite(now_ms);
    }

    fn rest_phase(&self, now: f64) -> f64 {
        if self.reduced || self.mood != Mood::Idle {
            return 0.0;
        }
        ((now - self.rest_since).max(0.0) / 1000.0).rem_euclid(90.0)
    }

    pub fn sample(&mut self, now_ms: f64) -> Visual {
        let now = finite(now_ms);
        let drawn = art();
        let pose = self.pose(now);
        let mut local_eyes = self.eyes(drawn);
        blink_pair(&mut local_eyes, pose.blink);
        let rest = self.rest_phase(now);
        if (18.0..24.0).contains(&rest) {
            for eye in &mut local_eyes {
                for point in eye {
                    point[0] += 2.5 * (TAU * (rest - 18.0) / 6.0).sin();
                }
            }
        }
        let mut body = [[0.0; 2]; 72];
        let mut eyes = [[[0.0; 2]; 48]; 2];
        rig(&drawn.body, &mut body, pose.tx, pose.ty, pose.rot);
        for index in 0..2 {
            rig(
                &local_eyes[index],
                &mut eyes[index],
                pose.tx,
                pose.ty,
                pose.rot,
            );
        }
        let mut visual = Visual::new(body, eyes, BLUE, CREAM);
        self.push_props(&mut visual, drawn, now);
        visual
    }

    /// Frame-loop hint. False under reduced motion, for a quiet Idle, and after
    /// Done or Error have finished. True while work, search, speech, thought,
    /// or the confirmation pulse is on screen, and during a blink or poke.
    pub fn wants_high_fps(&self, now_ms: f64) -> bool {
        if self.reduced {
            return false;
        }
        let now = finite(now_ms);
        if now < self.poke_until || blink_moving(now) {
            return true;
        }
        match self.mood {
            Mood::Idle => {
                (18.0..24.0).contains(&self.rest_phase(now))
                    || (75.0..77.0).contains(&self.rest_phase(now))
            }
            Mood::Done => now < self.mood_at + DONE_MS,
            Mood::Error => now < self.mood_at + ERROR_MS,
            Mood::Thinking | Mood::Searching | Mood::Working | Mood::Speaking | Mood::Waiting => {
                true
            }
        }
    }

    fn pose(&self, now: f64) -> Pose {
        if self.reduced {
            return Pose {
                tx: 0.0,
                ty: 0.0,
                rot: 0.0,
                blink: 1.0,
            };
        }
        let elapsed = (now - self.mood_at).max(0.0);
        let (tx, ty, rot) = match self.mood {
            Mood::Idle => (0.0, breathe(now, 4_000.0), 0.0),
            Mood::Thinking => (0.0, 0.0, think(now).to_radians()),
            Mood::Searching | Mood::Waiting => (0.0, 0.0, 0.0),
            Mood::Working => work(now),
            Mood::Speaking => (0.0, breathe(now, 2_800.0), 0.0),
            Mood::Done => (0.0, celebrate(elapsed), 0.0),
            Mood::Error => (shake(elapsed), 0.0, 0.0),
        };
        let rest = self.rest_phase(now);
        let bounce = if (75.0..77.0).contains(&rest) {
            -3.0 * (PI * (rest - 75.0)).sin().abs()
        } else {
            0.0
        };
        Pose {
            tx,
            ty: ty + self.poke_lift(now) + bounce,
            rot,
            blink: if (45.0..60.0).contains(&rest) {
                0.12
            } else {
                blink_scale(now)
            },
        }
    }

    fn poke_lift(&self, now: f64) -> f64 {
        if now < self.poke_at || now >= self.poke_until {
            return 0.0;
        }
        let t = (now - self.poke_at) / POKE_MS;
        track(t, &[(0.0, 0.0), (0.4, -3.5), (1.0, 0.0)], EASE_OUT)
    }

    fn eyes(&self, drawn: &Art) -> [[Point; 48]; 2] {
        match self.mood {
            Mood::Idle => drawn.eyes_idle,
            Mood::Thinking => drawn.eyes_think,
            Mood::Done => drawn.eyes_done,
            Mood::Error => drawn.eyes_error,
            Mood::Searching | Mood::Working | Mood::Speaking | Mood::Waiting => drawn.eyes_base,
        }
    }

    fn push_props(&self, visual: &mut Visual, drawn: &Art, now: f64) {
        match self.mood {
            Mood::Idle => {}
            Mood::Thinking => {
                let delays = [0.0, 250.0, 500.0];
                for (dot, delay) in drawn.dots.iter().zip(delays) {
                    push_poly(
                        visual,
                        dot,
                        fade(PROP, dot_alpha(self.reduced, now, delay)),
                        |p| p,
                    );
                }
            }
            Mood::Searching => {
                let (tx, ty, rot) = search_pose(self.reduced, now);
                push_poly(visual, &drawn.glass, PROP, |point| {
                    glass_at(point, tx, ty, rot)
                });
                push_poly(visual, &drawn.handle, PROP, |point| {
                    glass_at(point, tx, ty, rot)
                });
            }
            Mood::Working => {
                push_poly(visual, &drawn.board, BOARD, |point| keyboard(point, 0.0));
                push_poly(visual, &drawn.shadow, SHADOW, |point| keyboard(point, 0.0));
                for (index, key) in drawn.keys.iter().enumerate() {
                    let amount = key_press(self.reduced, now, index);
                    push_poly(visual, key, fade(KEY, 1.0 - 0.5 * amount), |point| {
                        keyboard(point, 1.5 * amount)
                    });
                }
                push_poly(visual, &drawn.space, KEY, |point| keyboard(point, 0.0));
            }
            Mood::Speaking => {
                push_poly(visual, &drawn.bubble, PROP, |point| point);
                push_line(visual, drawn, 51.0, 37.0, 145.0, self.reduced, now, 0.0);
                push_line(visual, drawn, 51.0, 26.0, 155.0, self.reduced, now, 400.0);
            }
            Mood::Waiting => {
                let alpha = wait_alpha(self.reduced, now);
                push_poly(visual, &drawn.badge, fade(AMBER, alpha), |point| point);
                push_poly(visual, &drawn.question, fade(AMBER_INK, alpha), |point| {
                    point
                });
                push_poly(visual, &drawn.qdot, fade(AMBER_INK, alpha), |point| point);
            }
            Mood::Done => {
                push_poly(visual, &drawn.badge, GREEN, |point| point);
                push_poly(visual, &drawn.check, GREEN_INK, |point| point);
            }
            Mood::Error => {
                push_poly(visual, &drawn.badge, CORAL, |point| point);
                push_poly(visual, &drawn.bang, CORAL_INK, |point| point);
                push_poly(visual, &drawn.edot, CORAL_INK, |point| point);
            }
        }
    }
}

fn breathe(now: f64, period: f64) -> f64 {
    let Some(t) = phase(now, 0.0, period) else {
        return 0.0;
    };
    track(t, &[(0.0, 0.0), (0.5, -2.0), (1.0, 0.0)], EASE_IN_OUT)
}

fn think(now: f64) -> f64 {
    let Some(t) = phase(now, 0.0, 2_400.0) else {
        return 0.0;
    };
    track(t, &[(0.0, 0.0), (0.5, -2.0), (1.0, 0.0)], EASE_IN_OUT)
}

fn work(now: f64) -> (f64, f64, f64) {
    let Some(t) = phase(now, 0.0, 1_200.0) else {
        return (0.0, 0.0, 0.0);
    };
    let tx = track(
        t,
        &[(0.0, 0.0), (0.35, -1.0), (0.65, -0.5), (1.0, 0.0)],
        EASE_IN_OUT,
    );
    let ty = track(
        t,
        &[(0.0, 0.0), (0.35, 1.0), (0.65, 0.5), (1.0, 0.0)],
        EASE_IN_OUT,
    );
    let degrees = track(
        t,
        &[(0.0, 0.0), (0.35, -1.0), (0.65, 0.0), (1.0, 0.0)],
        EASE_IN_OUT,
    );
    (tx, ty, degrees.to_radians())
}

fn search_pose(reduced: bool, now: f64) -> (f64, f64, f64) {
    if reduced {
        return (0.0, 0.0, 0.0);
    }
    let Some(t) = phase(now, 0.0, 2_200.0) else {
        return (0.0, 0.0, 0.0);
    };
    let tx = track(t, &[(0.0, 0.0), (0.5, -6.0), (1.0, 0.0)], EASE_IN_OUT);
    let ty = track(t, &[(0.0, 0.0), (0.5, 4.0), (1.0, 0.0)], EASE_IN_OUT);
    let degrees = track(t, &[(0.0, -8.0), (0.5, 8.0), (1.0, -8.0)], EASE_IN_OUT);
    (tx, ty, degrees.to_radians())
}

fn celebrate(elapsed: f64) -> f64 {
    let t = (elapsed / DONE_MS).clamp(0.0, 1.0);
    track(
        t,
        &[(0.0, 0.0), (0.4, -6.0), (0.7, 1.0), (1.0, 0.0)],
        EASE_OUT,
    )
}

fn shake(elapsed: f64) -> f64 {
    let t = (elapsed / ERROR_MS).clamp(0.0, 1.0);
    track(
        t,
        &[
            (0.0, 0.0),
            (0.25, -3.0),
            (0.5, 3.0),
            (0.75, -1.0),
            (1.0, 0.0),
        ],
        EASE_OUT,
    )
}

fn blink_scale(now: f64) -> f64 {
    let Some(t) = phase(now, 0.0, 5_000.0) else {
        return 1.0;
    };
    track(
        t,
        &[
            (0.0, 1.0),
            (0.45, 1.0),
            (0.47, 0.1),
            (0.49, 1.0),
            (1.0, 1.0),
        ],
        EASE,
    )
}

fn blink_moving(now: f64) -> bool {
    phase(now, 0.0, 5_000.0).is_some_and(|t| (0.44..0.50).contains(&t))
}

fn blink_pair(eyes: &mut [[Point; 48]; 2], scale: f64) {
    if (scale - 1.0).abs() < 1.0e-4 {
        return;
    }
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for eye in eyes.iter() {
        for point in eye {
            min_y = min_y.min(point[1]);
            max_y = max_y.max(point[1]);
        }
    }
    let center = 0.5 * (min_y + max_y);
    for eye in eyes {
        for point in eye {
            point[1] = center + (point[1] - center) * scale;
        }
    }
}

fn dot_alpha(reduced: bool, now: f64, delay: f64) -> f64 {
    if reduced {
        return 1.0;
    }
    let Some(t) = phase(now, delay, 1_500.0) else {
        return 1.0;
    };
    track(
        t,
        &[(0.0, 0.35), (0.35, 1.0), (0.80, 0.35), (1.0, 0.35)],
        EASE,
    )
}

fn key_press(reduced: bool, now: f64, index: usize) -> f64 {
    if reduced {
        return 0.0;
    }
    let delay = if index == 11 { -300.0 } else { 0.0 };
    if index != 3 && index != 11 {
        return 0.0;
    }
    let Some(t) = phase(now, delay, 600.0) else {
        return 0.0;
    };
    track(t, &[(0.0, 0.0), (0.30, 1.0), (0.65, 0.0), (1.0, 0.0)], EASE)
}

fn wait_alpha(reduced: bool, now: f64) -> f64 {
    if reduced {
        return 1.0;
    }
    let Some(t) = phase(now, 0.0, 2_400.0) else {
        return 1.0;
    };
    track(t, &[(0.0, 1.0), (0.5, 0.6), (1.0, 1.0)], EASE_IN_OUT)
}

fn push_line(
    visual: &mut Visual,
    drawn: &Art,
    x: f64,
    full: f64,
    y: f64,
    reduced: bool,
    now: f64,
    delay: f64,
) {
    let length = if reduced {
        full
    } else if let Some(t) = phase(now, delay, 2_400.0) {
        let offset = track(t, &[(0.0, 48.0), (0.65, 0.0), (1.0, 0.0)], EASE_IN_OUT);
        (48.0 - offset).clamp(0.0, full)
    } else {
        full
    };
    if length < 0.75 {
        return;
    }
    let poly = capsule(x, x + length, y, 1.5, &drawn.caps);
    push_poly(visual, &poly, INK, |point| point);
}

#[cfg(test)]
mod tests {
    use super::super::visual::{Point, Visual, SIZE};
    use super::super::Mood;
    use super::{BlobSim, AMBER, BLUE, BOARD, CORAL, CREAM, GREEN, SCALE};

    const MOODS: [Mood; 8] = [
        Mood::Idle,
        Mood::Thinking,
        Mood::Searching,
        Mood::Working,
        Mood::Speaking,
        Mood::Waiting,
        Mood::Done,
        Mood::Error,
    ];

    fn pose(reduced: bool, mood: Mood, now: f64) -> Visual {
        let mut sim = BlobSim::new(reduced);
        sim.set_mood(mood, 0.0);
        sim.sample(now)
    }

    #[test]
    fn idle_nap_wakes_on_interaction_without_deforming_the_body() {
        let mut sim = BlobSim::new(false);
        sim.set_mood(Mood::Idle, 0.0);
        let asleep = sim.sample(50_000.0);
        sim.wake(50_000.0);
        let awake = sim.sample(50_000.0);
        assert_eq!(asleep.body, awake.body);
        assert_ne!(asleep.eyes, awake.eyes);
        for mood in [Mood::Working, Mood::Waiting] {
            sim.set_mood(mood, 0.0);
            let before = sim.sample(50_000.0);
            sim.wake(50_000.0);
            assert_eq!(before, sim.sample(50_000.0));
        }
        let mut reduced = BlobSim::new(true);
        assert_eq!(reduced.sample(0.0), reduced.sample(50_000.0));
    }

    fn centroid(body: &[[f64; 2]; 72]) -> [f64; 2] {
        let mut center = [0.0; 2];
        for point in body {
            center[0] += point[0];
            center[1] += point[1];
        }
        [center[0] / 72.0, center[1] / 72.0]
    }

    fn radius(body: &[[f64; 2]; 72]) -> f64 {
        let center = centroid(body);
        (body[0][0] - center[0]).hypot(body[0][1] - center[1])
    }

    fn assert_round(body: &[[f64; 2]; 72], mood: Mood, now: f64) {
        let center = centroid(body);
        let expected = 53.0 * SCALE;
        let got = radius(body);
        assert!(
            (got - expected).abs() < 1.0e-5,
            "{mood:?} @{now} radius {got}"
        );
        for point in body {
            let gap = (point[0] - center[0]).hypot(point[1] - center[1]) - got;
            assert!(gap.abs() < 1.0e-5, "{mood:?} @{now} off-round {gap}");
        }
    }

    fn assert_inside(visual: &Visual, mood: Mood, now: f64) {
        let mut check = |point: Point| {
            assert!(
                (0.0..=SIZE).contains(&point[0]) && (0.0..=SIZE).contains(&point[1]),
                "{mood:?} @{now} leaves the frame at {point:?}"
            );
        };
        for point in visual.body {
            check(point);
        }
        for eye in visual.eyes {
            for point in eye {
                check(point);
            }
        }
        for prop in &visual.props[..visual.prop_count] {
            for point in &prop.points[..prop.len] {
                check(*point);
            }
        }
    }

    fn prop_points(visual: &Visual, index: usize) -> &[Point] {
        let prop = &visual.props[index];
        &prop.points[..prop.len]
    }

    #[test]
    fn body_stays_circular_and_inside_the_frame() {
        let times = [
            0.0, 90.0, 180.0, 300.0, 420.0, 600.0, 750.0, 1_100.0, 1_400.0, 1_600.0, 2_200.0,
            4_000.0,
        ];
        for mood in MOODS {
            for now in times {
                let frame = pose(false, mood, now);
                assert_round(&frame.body, mood, now);
                assert_inside(&frame, mood, now);
            }
            let mut sim = BlobSim::new(false);
            sim.set_mood(mood, 0.0);
            sim.poke(1_000.0, None);
            let nudged = sim.sample(1_120.0);
            assert_round(&nudged.body, mood, 1_120.0);
            assert_inside(&nudged, mood, 1_120.0);
        }

        let idle_rest = pose(false, Mood::Idle, 0.0);
        let idle_high = pose(false, Mood::Idle, 2_000.0);
        assert!(centroid(&idle_high.body)[1] < centroid(&idle_rest.body)[1] - 0.5);
        assert_eq!(idle_rest.prop_count, 0);

        let think_rest = pose(false, Mood::Thinking, 0.0);
        let think_tilt = pose(false, Mood::Thinking, 1_200.0);
        assert_ne!(think_rest.body, think_tilt.body);
        assert_eq!(think_rest.prop_count, think_tilt.prop_count);
        for index in 0..think_rest.prop_count {
            assert_eq!(
                prop_points(&think_rest, index),
                prop_points(&think_tilt, index)
            );
        }

        let search_rest = pose(false, Mood::Searching, 0.0);
        let search_sweep = pose(false, Mood::Searching, 1_100.0);
        assert_eq!(search_rest.body, search_sweep.body);
        assert_ne!(search_rest.props, search_sweep.props);

        let work_rest = pose(false, Mood::Working, 0.0);
        let work_lean = pose(false, Mood::Working, 420.0);
        assert_ne!(work_rest.body, work_lean.body);
        assert_eq!(prop_points(&work_rest, 0), prop_points(&work_lean, 0));
        assert_eq!(work_rest.prop_count, 17);

        let speak_rest = pose(false, Mood::Speaking, 0.0);
        let speak_high = pose(false, Mood::Speaking, 1_400.0);
        assert_ne!(speak_rest.body, speak_high.body);
        assert_eq!(prop_points(&speak_rest, 0), prop_points(&speak_high, 0));

        let mut held = BlobSim::new(false);
        held.set_mood(Mood::Working, 0.0);
        let mut poked = BlobSim::new(false);
        poked.set_mood(Mood::Working, 0.0);
        poked.poke(1_000.0, Some(0.4));
        let still = held.sample(1_120.0);
        let moved = poked.sample(1_120.0);
        assert_eq!(still.props, moved.props);
        assert_eq!(still.prop_count, moved.prop_count);
        assert_ne!(still.body, moved.body);
        assert_round(&moved.body, Mood::Working, 1_120.0);

        let mut working = BlobSim::new(false);
        working.set_mood(Mood::Working, 0.0);
        assert!(working.wants_high_fps(20_000.0));
        for mood in [
            Mood::Thinking,
            Mood::Searching,
            Mood::Speaking,
            Mood::Waiting,
        ] {
            let mut sim = BlobSim::new(false);
            sim.set_mood(mood, 0.0);
            assert!(sim.wants_high_fps(20_000.0), "{mood:?}");
        }
    }

    #[test]
    fn reduced_motion_freezes_each_distinct_state() {
        let mut frozen = [Visual::new([[0.0; 2]; 72], [[[0.0; 2]; 48]; 2], BLUE, CREAM); 8];
        for (index, mood) in MOODS.into_iter().enumerate() {
            let mut sim = BlobSim::new(true);
            sim.set_mood(mood, 0.0);
            let first = sim.sample(0.0);
            let later = sim.sample(9_000.0);
            assert_eq!(first, later, "{mood:?}");
            assert!(!sim.wants_high_fps(2_350.0), "{mood:?}");
            assert_round(&first.body, mood, 0.0);
            assert_inside(&first, mood, 0.0);
            sim.poke(0.0, Some(-1.0));
            assert_eq!(first, sim.sample(100.0), "{mood:?}");
            frozen[index] = first;
        }

        let idle = &frozen[0];
        let thinking = &frozen[1];
        let searching = &frozen[2];
        let working = &frozen[3];
        let speaking = &frozen[4];
        let waiting = &frozen[5];
        let done = &frozen[6];
        let error = &frozen[7];
        assert_eq!(idle.prop_count, 0);
        assert_eq!(thinking.prop_count, 3);
        assert_eq!(searching.prop_count, 2);
        assert_eq!(working.prop_count, 17);
        assert_eq!(speaking.prop_count, 3);
        assert_eq!(waiting.prop_count, 3);
        assert_eq!(done.prop_count, 2);
        assert_eq!(error.prop_count, 3);
        assert_eq!(working.props[0].color, BOARD);
        assert_eq!(waiting.props[0].color, AMBER);
        assert_eq!(done.props[0].color, GREEN);
        assert_eq!(error.props[0].color, CORAL);
        assert_ne!(idle.eyes, thinking.eyes);
        assert_ne!(idle.eyes, done.eyes);
        assert_ne!(idle.eyes, error.eyes);
        assert_ne!(thinking.eyes, searching.eyes);
        assert_eq!(searching.eyes, working.eyes);
        assert_eq!(working.eyes, speaking.eyes);
        assert_eq!(speaking.eyes, waiting.eyes);
        for index in 0..frozen.len() {
            for other in index + 1..frozen.len() {
                assert_ne!(frozen[index], frozen[other]);
            }
        }

        let center = centroid(&idle.body);
        assert!((center[0] - 158.0 * SCALE).abs() < 1.0e-5);
        assert!((center[1] - 101.0 * SCALE).abs() < 1.0e-5);

        let angle = 20.0_f64.to_radians();
        let rim = [
            (78.0 + 17.0 * angle.cos()) * SCALE,
            (144.0 + 17.0 * angle.sin()) * SCALE,
        ];
        let hole = [78.0 * SCALE, 144.0 * SCALE];
        assert!(searching.contains(rim[0], rim[1]));
        assert!(!searching.contains(hole[0], hole[1]));
        assert!(!idle.contains(rim[0], rim[1]));
        assert!(!idle.contains(hole[0], hole[1]));
        let key = [
            (0.80 * 13.0 - 0.66 * 10.5 + 78.0) * SCALE,
            (0.32 * 13.0 + 0.48 * 10.5 + 136.0) * SCALE,
        ];
        assert!(working.contains(key[0], key[1]));
        assert!(!idle.contains(key[0], key[1]));
        assert!(!searching.contains(key[0], key[1]));
    }

    #[test]
    fn done_and_error_finish_once_and_a_repeat_does_not_restart() {
        let rest = pose(false, Mood::Done, 0.0);
        let mut done = BlobSim::new(false);
        done.set_mood(Mood::Done, 0.0);
        let peak = done.sample(300.0);
        assert!(centroid(&peak.body)[1] < centroid(&rest.body)[1] - 0.5);
        assert_round(&peak.body, Mood::Done, 300.0);
        assert!(done.wants_high_fps(300.0));
        let settled = done.sample(800.0);
        assert_eq!(settled.body, rest.body);
        assert!(!done.wants_high_fps(800.0));
        done.set_mood(Mood::Done, 800.0);
        let held = done.sample(1_100.0);
        assert_eq!(held.body, rest.body);
        assert!(!done.wants_high_fps(1_100.0));
        let mut restarted = BlobSim::new(false);
        restarted.set_mood(Mood::Done, 800.0);
        assert_ne!(held.body, restarted.sample(1_100.0).body);
        assert!(restarted.wants_high_fps(1_100.0));

        let error_rest = pose(false, Mood::Error, 0.0);
        let mut error = BlobSim::new(false);
        error.set_mood(Mood::Error, 0.0);
        let shaken = error.sample(100.0);
        assert!(centroid(&shaken.body)[0] < centroid(&error_rest.body)[0] - 0.4);
        assert_round(&shaken.body, Mood::Error, 100.0);
        assert!(error.wants_high_fps(100.0));
        let still = error.sample(500.0);
        assert_eq!(still.body, error_rest.body);
        assert!(!error.wants_high_fps(500.0));
        error.set_mood(Mood::Error, 500.0);
        let held = error.sample(600.0);
        assert_eq!(held.body, error_rest.body);
        assert!(!error.wants_high_fps(600.0));
        let mut restarted = BlobSim::new(false);
        restarted.set_mood(Mood::Error, 500.0);
        assert_ne!(held.body, restarted.sample(600.0).body);

        let mut idle = BlobSim::new(false);
        idle.set_mood(Mood::Idle, 0.0);
        assert!(!idle.wants_high_fps(0.0));
        assert!(idle.wants_high_fps(2_350.0));
        assert!(!idle.wants_high_fps(3_000.0));
        idle.poke(10_000.0, None);
        assert!(idle.wants_high_fps(10_100.0));
        assert!(!idle.wants_high_fps(10_400.0));
        assert_eq!(idle.poke(10_000.0, None), 2);
    }
}
