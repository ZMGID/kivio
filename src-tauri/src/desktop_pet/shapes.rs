//! Static Momo polygons taken from `src/chat/kivioBlobShapes.ts`.
//!
//! [`body_points`] and [`face_points`] are that module's tables in the 240×240
//! viewBox (top-left origin, clockwise, first vertex at the top). [`CX`],
//! [`CY`], and [`BODY_R`] are the same constants. Samples that fill a
//! [`super::visual::Visual`] scale by `super::visual::SIZE / 240.0`; these
//! tables stay unscaled so pose math can keep the source center.
//!
//! `npm run momo:generate` rewrites included `momo_geometry.inc.rs`.
//! `npm run momo:check` fails when that file drifts from the TypeScript module.

pub const CX: f64 = 120.0;
pub const CY: f64 = 120.0;
pub const BODY_R: f64 = 78.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyShape {
    Circle,
    Squircle,
    Cloud,
    Bubble,
    Puddle,
    Burst,
    Pebble,
    Bean,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceName {
    Neutral,
    Dots,
    Wide,
    Tiny,
    Lines,
    Focus,
    Worry,
    Smirk,
    Happy,
    Content,
    Sleepy,
    Dizzy,
    Sparkle,
    LookUp,
    Hmm,
    Peek,
    Flat,
}

include!("momo_geometry.inc.rs");
