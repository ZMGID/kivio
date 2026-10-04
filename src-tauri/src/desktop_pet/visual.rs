//! Flat filled polygons shared by the macOS and Windows desktop pet.
//! Paint order is body, eyes, then props. No contour, tint, or shading.
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
pub struct Shape {
    pub points: [Point; 48],
    pub len: usize,
    pub color: Color,
}

impl Shape {
    pub const EMPTY: Self = Self {
        points: [[0.0; 2]; 48],
        len: 0,
        color: Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        },
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visual {
    pub body: [Point; 72],
    pub eyes: [[Point; 48]; 2],
    pub body_color: Color,
    pub eye_color: Color,
    pub props: [Shape; 24],
    pub prop_count: usize,
}

impl Visual {
    pub fn new(
        body: [Point; 72],
        eyes: [[Point; 48]; 2],
        body_color: Color,
        eye_color: Color,
    ) -> Self {
        Self {
            body,
            eyes,
            body_color,
            eye_color,
            props: [Shape::EMPTY; 24],
            prop_count: 0,
        }
    }

    /// Copies one closed polygon into the fixed prop list.
    pub fn push_polygon(&mut self, points: &[Point], color: Color) {
        assert!(self.prop_count < self.props.len());
        assert!(points.len() <= Shape::EMPTY.points.len());
        let slot = &mut self.props[self.prop_count];
        *slot = Shape::EMPTY;
        slot.len = points.len();
        slot.color = color;
        slot.points[..points.len()].copy_from_slice(points);
        self.prop_count += 1;
    }

    /// Visible body and props. Transparent ink and empty gaps miss.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        if visible(self.body_color) && contains(&self.body, x, y) {
            return true;
        }
        let count = self.prop_count.min(self.props.len());
        for prop in &self.props[..count] {
            if prop.len >= 3
                && prop.len <= prop.points.len()
                && visible(prop.color)
                && contains(&prop.points[..prop.len], x, y)
            {
                return true;
            }
        }
        false
    }
}

fn visible(color: Color) -> bool {
    color.a > 0.0
}

/// Even-odd test. Concave notches miss; fewer than three points miss.
pub fn contains(points: &[Point], x: f64, y: f64) -> bool {
    if points.len() < 3 {
        return false;
    }
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

/// Shared layer list so both native renderers fill the same flat geometry.
fn layers(visual: &Visual, mut paint: impl FnMut(&[Point], Color)) {
    if visible(visual.body_color) {
        paint(&visual.body, visual.body_color);
    }
    if visible(visual.eye_color) {
        for eye in &visual.eyes {
            paint(eye, visual.eye_color);
        }
    }
    let count = visual.prop_count.min(visual.props.len());
    for prop in &visual.props[..count] {
        if prop.len >= 3 && prop.len <= prop.points.len() && visible(prop.color) {
            paint(&prop.points[..prop.len], prop.color);
        }
    }
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
    layers(visual, |points, color| {
        if points.len() < 3 {
            return;
        }
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for point in points {
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        if !(min[0].is_finite() && min[1].is_finite() && max[0].is_finite() && max[1].is_finite()) {
            return;
        }
        let top = (min[1] * sy).floor().max(0.0) as usize;
        let bottom = (max[1] * sy).ceil().min(height as f64).max(0.0) as usize;
        let left = (min[0] * sx).floor().max(0.0) as usize;
        let right = (max[0] * sx).ceil().min(width as f64).max(0.0) as usize;
        for y in top..bottom {
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
    layers(visual, |points, color| {
        ctx.set_rgb_fill_color(color.r, color.g, color.b, color.a);
        ctx.begin_path();
        ctx.move_to_point(points[0][0], points[0][1]);
        for point in &points[1..] {
            ctx.add_line_to_point(point[0], point[1]);
        }
        ctx.close_path();
        // Even-odd matches the Windows scanline pairs, so joined rings stay hollow.
        ctx.eo_fill_path();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(r: f64, g: f64, b: f64, a: f64) -> Color {
        Color { r, g, b, a }
    }

    fn square_body(x0: f64, y0: f64, x1: f64, y1: f64) -> [Point; 72] {
        let mut body = [[0.0; 2]; 72];
        let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        for i in 0..72 {
            let side = i / 18;
            let t = (i % 18) as f64 / 18.0;
            let a = corners[side];
            let b = corners[(side + 1) % 4];
            body[i] = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        }
        body
    }

    fn square_eye(x0: f64, y0: f64, x1: f64, y1: f64) -> [Point; 48] {
        let mut eye = [[0.0; 2]; 48];
        let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        for i in 0..48 {
            let side = i / 12;
            let t = (i % 12) as f64 / 12.0;
            let a = corners[side];
            let b = corners[(side + 1) % 4];
            eye[i] = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        }
        eye
    }

    fn sample_visual() -> Visual {
        let mut eyes = [[[0.0; 2]; 48]; 2];
        eyes[0] = square_eye(80.0, 80.0, 96.0, 96.0);
        let mut visual = Visual::new(
            square_body(16.0, 16.0, 48.0, 48.0),
            eyes,
            color(0.2, 0.4, 0.8, 1.0),
            color(1.0, 1.0, 1.0, 1.0),
        );
        visual.push_polygon(
            &[
                [60.0, 60.0],
                [70.0, 60.0],
                [70.0, 70.0],
                [65.0, 65.0],
                [60.0, 70.0],
            ],
            color(0.0, 1.0, 0.0, 1.0),
        );
        visual.push_polygon(
            &[[60.0, 16.0], [90.0, 16.0], [90.0, 40.0], [60.0, 40.0]],
            color(1.0, 0.0, 0.0, 0.5),
        );
        visual.push_polygon(
            &[[100.0, 16.0], [120.0, 16.0], [120.0, 40.0], [100.0, 40.0]],
            color(1.0, 1.0, 1.0, 0.0),
        );
        visual.push_polygon(&[[4.0, 80.0], [30.0, 80.0]], color(1.0, 1.0, 1.0, 1.0));
        visual
    }

    #[test]
    fn concave_outline_does_not_capture_its_empty_notch() {
        let points = [[0., 0.], [10., 0.], [10., 10.], [5., 5.], [0., 10.]];
        assert!(contains(&points, 5., 2.));
        assert!(!contains(&points, 5., 8.));
        assert!(!contains(&points, -1., 2.));
        assert!(!contains(&[[0., 0.], [1., 1.]], 0., 0.));
    }

    #[test]
    fn visible_props_capture_ink_and_miss_empty_gaps() {
        let visual = sample_visual();
        assert!(visual.contains(24.0, 24.0));
        assert!(!visual.contains(4.0, 4.0));
        assert!(visual.contains(65.0, 62.0));
        assert!(!visual.contains(65.0, 68.0));
        assert!(visual.contains(70.0, 24.0));
        assert!(!visual.contains(90.0, 90.0));
        assert!(!visual.contains(110.0, 24.0));
        assert!(!visual.contains(10.0, 80.0));
        assert!(!visual.contains(88.0, 88.0));
        let mut hidden_body = visual;
        hidden_body.body_color.a = 0.0;
        assert!(!hidden_body.contains(24.0, 24.0));
        assert!(hidden_body.contains(65.0, 62.0));
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

    #[test]
    fn raster_paints_visible_props_and_leaves_gaps_clear() {
        let visual = sample_visual();
        let mut pixels = vec![0; 128 * 128 * 4];
        rasterize(&visual, &mut pixels, 128, 128);
        let px = |x: usize, y: usize| {
            let index = (y * 128 + x) * 4;
            [
                pixels[index],
                pixels[index + 1],
                pixels[index + 2],
                pixels[index + 3],
            ]
        };
        assert_eq!(px(0, 0), [0, 0, 0, 0]);
        assert_eq!(px(24, 24), [204, 102, 51, 255]);
        assert_eq!(px(70, 24), [0, 0, 128, 128]);
        assert_eq!(px(65, 62), [0, 255, 0, 255]);
        assert_eq!(px(65, 68), [0, 0, 0, 0]);
        assert_eq!(px(88, 88), [255, 255, 255, 255]);
        assert_eq!(px(105, 20), [0, 0, 0, 0]);
        assert_eq!(px(10, 85), [0, 0, 0, 0]);
        assert!(pixels
            .chunks_exact(4)
            .all(|p| p[..3].iter().all(|c| *c <= p[3])));
    }
}
