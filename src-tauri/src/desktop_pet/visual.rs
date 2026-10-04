//! Illustrated Momo: tinted contour, narrow soft rim, restrained body shading.
pub const SIZE: f64 = 128.0;
pub type Point = [f64; 2];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visual {
    pub body: [Point; 72],
    pub eyes: [[Point; 48]; 2],
    pub body_color: Color,
    pub eye_color: Color,
}

/// Only the visible silhouette captures input, including concave shapes.
pub fn contains(points: &[Point], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut previous = points[points.len() - 1];
    for &point in points {
        if (point[1] > y) != (previous[1] > y)
            && x < (previous[0] - point[0]) * (y - point[1]) / (previous[1] - point[1]) + point[0]
        {
            inside = !inside;
        }
        previous = point;
    }
    inside
}

/// Shared layer geometry keeps both native renderers on the same art direction.
/// Insets stay inside the existing silhouette: no shadow or larger input region.
fn layers(visual: &Visual, mut paint: impl FnMut(&[Point], Color, bool)) {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in &visual.body {
        for axis in 0..2 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
    let inset = |scale: f64, dy: f64| {
        visual.body.map(|p| {
            [
                center[0] + (p[0] - center[0]) * scale,
                center[1] + (p[1] - center[1]) * scale + dy,
            ]
        })
    };
    // About 1.4 logical pixels of colored ink on the resting 83px body.
    paint(&visual.body, tint(visual.body_color, -0.32), false);
    paint(&inset(0.966, -0.35), tint(visual.body_color, 0.22), false);
    // A narrow upper rim and slightly heavier lower contour suggest softness,
    // without a specular spot, spherical normal lighting, or a floor shadow.
    paint(&inset(0.943, 0.05), visual.body_color, true);
    for eye in &visual.eyes {
        paint(eye, visual.eye_color, false);
    }
}

fn tint(color: Color, amount: f64) -> Color {
    let channel = |c: f64| {
        if amount >= 0.0 {
            c + (1.0 - c) * amount
        } else {
            c * (1.0 + amount)
        }
    };
    Color {
        r: channel(color.r),
        g: channel(color.g),
        b: channel(color.b),
        a: color.a,
    }
}

#[cfg(any(target_os = "windows", test))]
fn face_tint(color: Color, t: f64) -> Color {
    let amount = if t < 0.45 {
        0.06 * (1.0 - t / 0.45)
    } else if t > 0.55 {
        -0.08 * ((t - 0.55) / 0.45)
    } else {
        0.0
    };
    tint(color, amount)
}

/// Scanline coverage into the existing premultiplied BGRA DIB, no per-frame heap.
#[cfg(any(target_os = "windows", test))]
pub fn rasterize(visual: &Visual, pixels: &mut [u8], width: usize, height: usize) {
    assert_eq!(pixels.len(), width * height * 4);
    pixels.fill(0);
    if width == 0 || height == 0 {
        return;
    }
    let sx = width as f64 / SIZE;
    let sy = height as f64 / SIZE;
    layers(visual, |points, color, shaded| {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for point in points {
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        let top = (min[1] * sy).floor().max(0.0) as usize;
        let bottom = (max[1] * sy).ceil().min(height as f64).max(0.0) as usize;
        let left = (min[0] * sx).floor().max(0.0) as usize;
        let right = (max[0] * sx).ceil().min(width as f64).max(0.0) as usize;
        for y in top..bottom {
            let color = if shaded {
                face_tint(
                    color,
                    (((y as f64 + 0.5) / sy - min[1]) / (max[1] - min[1]).max(f64::EPSILON))
                        .clamp(0.0, 1.0),
                )
            } else {
                color
            };
            let mut edges = [[0.0_f64; 72]; 2];
            let mut lengths = [0; 2];
            for (sample, dy) in [0.25, 0.75].into_iter().enumerate() {
                let scan_y = (y as f64 + dy) / sy;
                let mut previous = points[points.len() - 1];
                for &point in points {
                    if (point[1] > scan_y) != (previous[1] > scan_y) {
                        edges[sample][lengths[sample]] = (point[0]
                            + (scan_y - point[1]) * (previous[0] - point[0])
                                / (previous[1] - point[1]))
                            * sx;
                        lengths[sample] += 1;
                    }
                    previous = point;
                }
                edges[sample][..lengths[sample]].sort_unstable_by(f64::total_cmp);
            }
            let mut cursors = [0; 2];
            for x in left..right {
                let mut coverage = 0.0;
                for sample in 0..2 {
                    let row = &edges[sample];
                    while cursors[sample] + 1 < lengths[sample]
                        && row[cursors[sample] + 1] <= x as f64
                    {
                        cursors[sample] += 2;
                    }
                    let mut k = cursors[sample];
                    while k + 1 < lengths[sample] && row[k] < (x + 1) as f64 {
                        coverage +=
                            (row[k + 1].min((x + 1) as f64) - row[k].max(x as f64)).max(0.0) * 0.5;
                        k += 2;
                    }
                }
                let alpha = color.a * coverage.min(1.0);
                if alpha == 0.0 {
                    continue;
                }
                let index = (y * width + x) * 4;
                let inv = 1.0 - alpha;
                for (channel, value) in [color.b, color.g, color.r].into_iter().enumerate() {
                    pixels[index + channel] = (value * alpha * 255.0
                        + pixels[index + channel] as f64 * inv)
                        .round() as u8;
                }
                pixels[index + 3] = (alpha * 255.0 + pixels[index + 3] as f64 * inv).round() as u8;
            }
        }
    });
}

#[cfg(target_os = "macos")]
pub fn paint_macos(ctx: &core_graphics::context::CGContext, visual: &Visual) {
    use core_graphics::{
        color_space::CGColorSpace,
        geometry::{CGPoint, CGRect, CGSize},
        gradient::{CGGradient, CGGradientDrawingOptions},
    };
    thread_local! {
        static SHADING: CGGradient = CGGradient::create_with_color_components(
            &CGColorSpace::create_device_rgb(),
            &[1.0,1.0,1.0,0.06, 1.0,1.0,1.0,0.0,
              0.0,0.0,0.0,0.0, 0.0,0.0,0.0,0.08],
            &[0.0,0.45,0.55,1.0], 4,
        );
    }
    layers(visual, |points, color, shaded| {
        ctx.set_rgb_fill_color(color.r, color.g, color.b, color.a);
        ctx.begin_path();
        ctx.move_to_point(points[0][0], points[0][1]);
        for point in &points[1..] {
            ctx.add_line_to_point(point[0], point[1]);
        }
        ctx.close_path();
        if shaded {
            let top = points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
            let bottom = points
                .iter()
                .map(|p| p[1])
                .fold(f64::NEG_INFINITY, f64::max);
            ctx.save();
            ctx.clip();
            ctx.fill_rect(CGRect::new(
                &CGPoint::new(0.0, 0.0),
                &CGSize::new(SIZE, SIZE),
            ));
            SHADING.with(|gradient| {
                ctx.draw_linear_gradient(
                    gradient,
                    CGPoint::new(0.0, top),
                    CGPoint::new(0.0, bottom),
                    CGGradientDrawingOptions::empty(),
                )
            });
            ctx.restore();
        } else {
            ctx.fill_path();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concave_outline_does_not_capture_its_empty_notch() {
        let points = [[0., 0.], [10., 0.], [10., 10.], [5., 5.], [0., 10.]];
        assert!(contains(&points, 5., 2.));
        assert!(!contains(&points, 5., 8.));
        assert!(!contains(&points, -1., 2.));
    }
    #[test]
    fn raster_preserves_transparent_background_and_premultiplied_edges() {
        let mut sim = super::super::sim::BlobSim::new(true);
        let visual = sim.sample(0.0);
        let mut pixels = vec![0; 128 * 128 * 4];
        rasterize(&visual, &mut pixels, 128, 128);
        assert!(pixels[..128 * 4].iter().all(|value| *value == 0));
        assert!(pixels[112 * 128 * 4..].iter().all(|value| *value == 0));
        assert!(pixels.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
        assert!(pixels
            .chunks_exact(4)
            .all(|p| p[..3].iter().all(|c| *c <= p[3])));
    }
}
