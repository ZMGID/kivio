//! Momo: a compact ink-blue pebble with a front-facing face and small feet.
//! All eight states share one silhouette, facial grid and prop palette. Static
//! outlines are cached once; sampling uses fixed arrays, without frame allocations.
use std::f64::consts::{PI, TAU};
use std::sync::LazyLock;

use super::visual::{Color, Point, Visual};
use super::Mood;

const BLUE: Color = rgb(65, 112, 231);
const FOOT: Color = rgb(51, 91, 199);
const WHITE: Color = rgb(247, 250, 255);
const INK: Color = rgb(51, 65, 96);
const SOFT: Color = rgb(208, 225, 255);
const PROP_EDGE: Color = rgb(163, 185, 222);
const GREEN: Color = rgb(52, 142, 116);
const AMBER: Color = rgb(177, 120, 35);
const RED: Color = rgb(193, 86, 92);
const POKE_MS: f64 = 360.0;
const DONE_MS: f64 = 680.0;
const ERROR_MS: f64 = 380.0;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f64 / 255.0,
        g: g as f64 / 255.0,
        b: b as f64 / 255.0,
        a: 1.0,
    }
}
fn alpha(color: Color, a: f64) -> Color {
    Color { a, ..color }
}
fn finite(t: f64) -> f64 {
    if t.is_finite() {
        t.max(0.0)
    } else {
        0.0
    }
}

#[derive(Clone, Copy)]
struct Poly {
    points: [Point; 48],
    len: usize,
}
impl Poly {
    fn full(points: [Point; 48]) -> Self {
        Self { points, len: 48 }
    }
}
static UNIT: LazyLock<[Point; 48]> = LazyLock::new(|| {
    std::array::from_fn(|i| {
        let (s, c) = (i as f64 * TAU / 48.0).sin_cos();
        [c, s]
    })
});
fn ellipse(x: f64, y: f64, rx: f64, ry: f64) -> Poly {
    Poly::full(UNIT.map(|p| [x + p[0] * rx, y + p[1] * ry]))
}
fn rounded(x: f64, y: f64, w: f64, h: f64, r: f64) -> Poly {
    Poly::full(UNIT.map(|p| {
        [
            x + w * 0.5 + (w * 0.5 - r) * p[0].signum() + r * p[0],
            y + h * 0.5 + (h * 0.5 - r) * p[1].signum() + r * p[1],
        ]
    }))
}
fn line(a: Point, b: Point, width: f64) -> Poly {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let length = dx.hypot(dy).max(1.0e-6);
    Poly::full(UNIT.map(|p| {
        let along = p[0] * width * 0.5 + length * 0.5 * p[0].signum();
        let across = p[1] * width * 0.5;
        [
            (a[0] + b[0]) * 0.5 + (along * dx - across * dy) / length,
            (a[1] + b[1]) * 0.5 + (along * dy + across * dx) / length,
        ]
    }))
}
fn curve(a: Point, b: Point, c: Point, width: f64) -> Poly {
    let mut points = [[0.0; 2]; 48];
    for i in 0..24 {
        let t = i as f64 / 23.0;
        let u = 1.0 - t;
        let x = u * u * a[0] + 2.0 * u * t * b[0] + t * t * c[0];
        let y = u * u * a[1] + 2.0 * u * t * b[1] + t * t * c[1];
        let dx = u * (b[0] - a[0]) + t * (c[0] - b[0]);
        let dy = u * (b[1] - a[1]) + t * (c[1] - b[1]);
        let k = width * 0.5 / dx.hypot(dy).max(1.0e-6);
        points[i] = [x - dy * k, y + dx * k];
        points[47 - i] = [x + dy * k, y - dx * k];
    }
    Poly::full(points)
}
fn index(mood: Mood) -> usize {
    match mood {
        Mood::Idle => 0,
        Mood::Thinking => 1,
        Mood::Searching => 2,
        Mood::Working => 3,
        Mood::Speaking => 4,
        Mood::Waiting => 5,
        Mood::Done => 6,
        Mood::Error => 7,
    }
}

struct Art {
    body: [Point; 72],
    eyes: [[[Point; 48]; 2]; 8],
    mouths: [Poly; 8],
    feet: [Poly; 2],
    gleam: Poly,
    card: Poly,
    card_edge: Poly,
    dot: Poly,
    lens: Poly,
    lens_hole: Poly,
    lens_handle: Poly,
    keyboard: Poly,
    keyboard_edge: Poly,
    keys: [Poly; 6],
    space: Poly,
    hands: [Poly; 2],
    question: [Poly; 3],
    check: [Poly; 2],
    bang: [Poly; 2],
    speech_lines: [Poly; 2],
}
static ART: LazyLock<Art> = LazyLock::new(|| {
    let open = [
        ellipse(76.5, 53.0, 2.8, 4.5).points,
        ellipse(91.5, 53.0, 2.8, 4.5).points,
    ];
    let smile = curve([81.0, 64.0], [84.0, 67.0], [87.0, 64.0], 1.5);
    Art {
        body: std::array::from_fn(|i| {
            let (s, c) = (i as f64 * TAU / 72.0).sin_cos();
            // A broad, soft belly rather than a perfect disk or a hard square.
            [
                84.0 + 28.0 * c.signum() * c.abs().powf(0.88) * (1.0 + 0.04 * s),
                55.0 + 27.0 * s.signum() * s.abs().powf(0.88),
            ]
        }),
        eyes: [
            open,
            [
                ellipse(77.5, 51.5, 2.8, 4.0).points,
                ellipse(92.5, 51.0, 2.8, 3.0).points,
            ],
            [
                ellipse(77.5, 53.0, 3.0, 4.9).points,
                ellipse(92.5, 53.0, 3.0, 4.9).points,
            ],
            [
                ellipse(76.5, 55.0, 2.8, 3.4).points,
                ellipse(91.5, 55.0, 2.8, 3.4).points,
            ],
            open,
            [
                ellipse(76.5, 52.5, 3.1, 5.0).points,
                ellipse(91.5, 52.5, 3.1, 5.0).points,
            ],
            [
                curve([73.5, 54.0], [76.5, 48.0], [79.5, 54.0], 2.2).points,
                curve([88.5, 54.0], [91.5, 48.0], [94.5, 54.0], 2.2).points,
            ],
            [
                line([74.0, 52.0], [79.0, 50.5], 2.0).points,
                line([89.0, 50.5], [94.0, 52.0], 2.0).points,
            ],
        ],
        mouths: [
            smile,
            ellipse(84.0, 64.0, 1.5, 1.8),
            smile,
            line([82.0, 65.0], [86.0, 65.0], 1.4),
            ellipse(84.0, 64.5, 2.4, 2.7),
            ellipse(84.0, 65.0, 1.7, 2.0),
            curve([79.5, 63.0], [84.0, 69.0], [88.5, 63.0], 1.9),
            curve([81.0, 66.0], [84.0, 62.5], [87.0, 66.0], 1.6),
        ],
        feet: [ellipse(72.5, 82.0, 7.0, 4.0), ellipse(95.5, 82.0, 7.0, 4.0)],
        gleam: curve([65.0, 45.0], [68.0, 36.0], [78.0, 34.5], 2.2),
        card: rounded(47.0, 68.0, 23.0, 23.0, 8.0),
        card_edge: rounded(46.25, 67.25, 24.5, 24.5, 8.75),
        dot: ellipse(0.0, 0.0, 1.5, 1.5),
        lens: ellipse(58.0, 77.0, 6.5, 6.5),
        lens_hole: ellipse(58.0, 77.0, 4.6, 4.6),
        lens_handle: line([62.0, 82.0], [66.0, 86.0], 2.2),
        keyboard: rounded(54.0, 78.0, 60.0, 19.0, 6.0),
        keyboard_edge: rounded(53.25, 77.25, 61.5, 20.5, 6.75),
        keys: std::array::from_fn(|i| rounded(61.0 + i as f64 * 8.0, 83.0, 5.0, 3.5, 1.3)),
        space: rounded(75.0, 90.0, 19.0, 2.5, 1.2),
        hands: [
            ellipse(67.0, 79.0, 5.2, 3.5),
            ellipse(101.0, 79.0, 5.2, 3.5),
        ],
        question: [
            curve([55.0, 75.0], [59.0, 70.5], [62.0, 75.0], 1.8),
            curve([62.0, 75.0], [63.0, 78.0], [58.5, 80.5], 1.8),
            ellipse(58.5, 85.0, 1.0, 1.0),
        ],
        check: [
            line([53.5, 79.0], [57.2, 82.5], 2.1),
            line([57.2, 82.5], [64.0, 75.5], 2.1),
        ],
        bang: [
            line([58.5, 73.5], [58.5, 80.0], 2.1),
            ellipse(58.5, 84.5, 1.1, 1.1),
        ],
        speech_lines: [
            line([53.5, 76.5], [64.0, 76.5], 1.6),
            line([53.5, 82.0], [61.0, 82.0], 1.6),
        ],
    }
});

#[derive(Clone, Copy)]
struct Pose {
    lift: f64,
    sine: f64,
    cosine: f64,
    blink: f64,
}
fn transform(p: Point, pose: Pose) -> Point {
    let (s, c) = (pose.sine, pose.cosine);
    [
        84.0 + (p[0] - 84.0) * c - (p[1] - 82.0) * s,
        82.0 + (p[0] - 84.0) * s + (p[1] - 82.0) * c + pose.lift,
    ]
}
fn push(visual: &mut Visual, poly: &Poly, color: Color, offset: Point) {
    let points = poly.points.map(|p| [p[0] + offset[0], p[1] + offset[1]]);
    visual.push_polygon(&points[..poly.len], color);
}
fn push_on_body(visual: &mut Visual, poly: &Poly, color: Color, pose: Pose) {
    let points = poly.points.map(|p| transform(p, pose));
    visual.push_polygon(&points[..poly.len], color);
}

pub struct BlobSim {
    reduced: bool,
    mood: Mood,
    mood_applied: bool,
    mood_at: f64,
    poke_at: f64,
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
            pokes: 0,
            rest_since: 0.0,
        }
    }
    pub fn set_mood(&mut self, mood: Mood, now_ms: f64) {
        if self.mood_applied && self.mood == mood {
            return;
        }
        self.mood = mood;
        self.mood_applied = true;
        self.mood_at = finite(now_ms);
        self.rest_since = self.mood_at;
    }
    pub fn poke(&mut self, now_ms: f64, _look_x: Option<f64>) -> u32 {
        self.poke_at = finite(now_ms);
        self.rest_since = self.poke_at;
        self.pokes = self.pokes.saturating_add(1);
        self.pokes
    }
    pub(super) fn wake(&mut self, now_ms: f64) {
        self.rest_since = finite(now_ms);
    }
    fn rest(&self, now: f64) -> f64 {
        ((now - self.rest_since).max(0.0) / 1000.0) % 90.0
    }
    fn pose(&self, now: f64) -> Pose {
        if self.reduced {
            return Pose {
                lift: 0.0,
                sine: 0.0,
                cosine: 1.0,
                blink: 1.0,
            };
        }
        let elapsed = (now - self.mood_at).max(0.0);
        let mut lift = match self.mood {
            Mood::Idle | Mood::Speaking => -0.65 * (1.0 - (now * TAU / 4200.0).cos()),
            Mood::Done if elapsed < DONE_MS => -3.0 * (PI * elapsed / DONE_MS).sin(),
            _ => 0.0,
        };
        let turn = match self.mood {
            Mood::Thinking => 0.035 * (now * TAU / 3600.0).sin(),
            Mood::Working => 0.012 * (now * TAU / 900.0).sin(),
            Mood::Error if elapsed < ERROR_MS => {
                0.035 * (elapsed * TAU / 190.0).sin() * (1.0 - elapsed / ERROR_MS)
            }
            _ => 0.0,
        };
        let poke = now - self.poke_at;
        if (0.0..POKE_MS).contains(&poke) {
            lift -= 2.5 * (PI * poke / POKE_MS).sin();
        }
        let blink_phase = now.rem_euclid(5200.0);
        let blink = if self.mood == Mood::Idle && (45.0..60.0).contains(&self.rest(now)) {
            0.12
        } else if (2320.0..2500.0).contains(&blink_phase) {
            1.0 - 0.9 * (PI * (blink_phase - 2320.0) / 180.0).sin()
        } else {
            1.0
        };
        let (sine, cosine) = turn.sin_cos();
        Pose {
            lift,
            sine,
            cosine,
            blink,
        }
    }
    pub fn sample(&mut self, now_ms: f64) -> Visual {
        let now = finite(now_ms);
        let a = &ART;
        let pose = self.pose(now);
        let mut eyes = a.eyes[index(self.mood)];
        let look = if !self.reduced && self.mood == Mood::Searching {
            1.2 * (now * TAU / 2200.0).sin()
        } else {
            0.0
        };
        let centers = match self.mood {
            Mood::Thinking => [51.5, 51.0],
            Mood::Working => [55.0, 55.0],
            Mood::Waiting => [52.5, 52.5],
            Mood::Error => [51.25, 51.25],
            _ => [53.0, 53.0],
        };
        for (eye, cy) in eyes.iter_mut().zip(centers) {
            for p in eye {
                p[1] = cy + (p[1] - cy) * pose.blink;
                p[0] += look;
                *p = transform(*p, pose);
            }
        }
        let mut visual = Visual::new(a.body.map(|p| transform(p, pose)), eyes, BLUE, WHITE);
        // The first two props are feet; DesktopBehavior gives them a walking step.
        for foot in &a.feet {
            push_on_body(&mut visual, foot, FOOT, pose);
        }
        push_on_body(&mut visual, &a.gleam, alpha(WHITE, 0.18), pose);
        let mut mouth = a.mouths[index(self.mood)];
        if !self.reduced && self.mood == Mood::Speaking {
            let scale = 0.65 + 0.35 * (now * TAU / 650.0).sin().abs();
            for p in &mut mouth.points {
                p[1] = 64.5 + (p[1] - 64.5) * scale;
            }
        }
        push_on_body(&mut visual, &mouth, WHITE, pose);
        match self.mood {
            Mood::Idle => {}
            Mood::Thinking => {
                for i in 0..3 {
                    let lift = if self.reduced {
                        0.0
                    } else {
                        (now * TAU / 1600.0 - i as f64 * 0.7).sin().max(0.0) * -1.8
                    };
                    push(
                        &mut visual,
                        &a.dot,
                        BLUE,
                        [75.5 + i as f64 * 8.5, 21.0 + lift],
                    );
                }
            }
            Mood::Working => {
                push(&mut visual, &a.keyboard_edge, PROP_EDGE, [0.0, 0.0]);
                push(&mut visual, &a.keyboard, WHITE, [0.0, 0.0]);
                for (i, key) in a.keys.iter().enumerate() {
                    let pressed = !self.reduced && ((now / 180.0) as usize % 6 == i);
                    push(
                        &mut visual,
                        key,
                        if pressed { BLUE } else { PROP_EDGE },
                        [0.0, 0.0],
                    );
                }
                push(&mut visual, &a.space, PROP_EDGE, [0.0, 0.0]);
                for (i, hand) in a.hands.iter().enumerate() {
                    let tap = if self.reduced {
                        0.0
                    } else {
                        (now * TAU / 450.0 + i as f64 * PI).sin().max(0.0) * 1.5
                    };
                    push(&mut visual, hand, BLUE, [0.0, tap]);
                }
            }
            mood => {
                push(&mut visual, &a.card_edge, PROP_EDGE, [0.0, 0.0]);
                push(&mut visual, &a.card, WHITE, [0.0, 0.0]);
                match mood {
                    Mood::Searching => {
                        push(&mut visual, &a.lens, INK, [0.0, 0.0]);
                        push(&mut visual, &a.lens_hole, SOFT, [0.0, 0.0]);
                        push(&mut visual, &a.lens_handle, INK, [0.0, 0.0]);
                    }
                    Mood::Speaking => {
                        for line in &a.speech_lines {
                            push(&mut visual, line, INK, [0.0, 0.0]);
                        }
                    }
                    Mood::Waiting => {
                        for part in &a.question {
                            push(&mut visual, part, AMBER, [0.0, 0.0]);
                        }
                    }
                    Mood::Done => {
                        for part in &a.check {
                            push(&mut visual, part, GREEN, [0.0, 0.0]);
                        }
                    }
                    Mood::Error => {
                        for part in &a.bang {
                            push(&mut visual, part, RED, [0.0, 0.0]);
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        visual
    }
    pub fn wants_high_fps(&self, now_ms: f64) -> bool {
        if self.reduced {
            return false;
        }
        let now = finite(now_ms);
        if (0.0..POKE_MS).contains(&(now - self.poke_at)) {
            return true;
        }
        let blink = now.rem_euclid(5200.0);
        if (2300.0..2520.0).contains(&blink) {
            return true;
        }
        match self.mood {
            Mood::Done => now - self.mood_at < DONE_MS,
            Mood::Error => now - self.mood_at < ERROR_MS,
            Mood::Idle | Mood::Waiting => false,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    #[test]
    fn every_pose_and_prop_stays_inside_native_hit_surface() {
        for mood in MOODS {
            let mut sim = BlobSim::new(false);
            sim.set_mood(mood, 0.0);
            sim.poke(1000.0, None);
            for t in [0.0, 160.0, 350.0, 680.0, 1100.0, 2400.0, 50000.0] {
                let v = sim.sample(t);
                for p in v.body.iter().chain(v.eyes.iter().flatten()).chain(
                    v.props[..v.prop_count]
                        .iter()
                        .flat_map(|s| &s.points[..s.len]),
                ) {
                    assert!(
                        p.iter().all(|n| n.is_finite() && (0.0..=128.0).contains(n)),
                        "{mood:?} @{t}: {p:?}"
                    );
                }
                assert!(v.contains(84.0, 55.0));
                assert!(!v.contains(0.0, 0.0));
                assert!(!v.contains(127.0, 127.0));
            }
        }
    }
    #[test]
    fn reduced_motion_retains_state_but_freezes_animation_and_pokes() {
        for mood in MOODS {
            let mut sim = BlobSim::new(true);
            sim.set_mood(mood, 0.0);
            let initial = sim.sample(0.0);
            sim.poke(100.0, None);
            assert_eq!(initial, sim.sample(200.0));
            assert_eq!(initial, sim.sample(50000.0));
            assert!(!sim.wants_high_fps(200.0));
            for other in MOODS.into_iter().filter(|other| *other != mood) {
                sim.set_mood(other, 0.0);
                assert_ne!(initial, sim.sample(0.0), "{mood:?} / {other:?}");
            }
        }
    }
    #[test]
    fn completed_feedback_does_not_restart_on_repeated_status() {
        for mood in [Mood::Done, Mood::Error] {
            let mut sim = BlobSim::new(false);
            sim.set_mood(mood, 0.0);
            assert!(sim.wants_high_fps(100.0));
            let settled = sim.sample(1000.0);
            sim.set_mood(mood, 1000.0);
            assert_eq!(settled, sim.sample(1100.0));
            assert!(!sim.wants_high_fps(1100.0));
        }
    }
    #[test]
    fn interaction_wakes_a_nap_without_changing_body_geometry() {
        let mut sim = BlobSim::new(false);
        let asleep = sim.sample(50000.0);
        sim.wake(50000.0);
        let awake = sim.sample(50000.0);
        assert_eq!(asleep.body, awake.body);
        assert_ne!(asleep.eyes, awake.eyes);
        sim.set_mood(Mood::Working, 50000.0);
        let work = sim.sample(50000.0);
        sim.wake(50000.0);
        assert_eq!(work, sim.sample(50000.0));
    }
}
