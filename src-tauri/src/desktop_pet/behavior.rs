//! Desktop-only gaze and placement over the round state animation.
//! Native adapters own placement; this state never pulls a placed pet downward.
use super::{visual::Visual, Mood};

#[derive(Clone, Copy, Debug, Default)]
pub struct Environment {
    pub cursor: Option<[f64; 2]>,
    pub pressed: bool,
    pub dragging: bool,
    pub floor_distance: f64,
    pub horizontal_room: [f64; 2],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Motion {
    pub delta: [f64; 2],
    pub persist: bool,
    pub active: bool,
}

pub struct DesktopBehavior {
    last: Option<f64>,
    gaze: [f64; 2],
    next_stroll: f64,
    stroll_left: f64,
    stroll_direction: f64,
    stroll_moved: bool,
}

impl DesktopBehavior {
    pub fn new() -> Self {
        Self {
            last: None,
            gaze: [0.0; 2],
            next_stroll: 15.0,
            stroll_left: 0.0,
            stroll_direction: -1.0,
            stroll_moved: false,
        }
    }

    pub fn apply(
        &mut self,
        visual: &mut Visual,
        mood: Mood,
        env: Environment,
        now: f64,
        reduced_motion: bool,
    ) -> Motion {
        let dt = self.last.map_or(0.0, |last| (now - last).clamp(0.0, 0.08));
        self.last = Some(now);
        let mut motion = Motion::default();
        let center = center(&visual.body);
        let mut look = [0.0; 2];
        let mut near = false;
        if let Some(cursor) = env.cursor {
            let dx = cursor[0] - center[0];
            let dy = cursor[1] - center[1];
            let distance = dx.hypot(dy);
            near = distance < 220.0;
            if near && distance > 0.5 && !reduced_motion && matches!(mood, Mood::Idle) {
                let weight = (1.0 - distance / 220.0) / distance;
                look = [dx * weight * 5.0, dy * weight * 3.5];
            }
        }

        let captured = env.pressed && !env.dragging;
        let calm = matches!(mood, Mood::Idle)
            && !near
            && !captured
            && !env.dragging
            && env.floor_distance <= 0.01
            && !reduced_motion;
        // A press pauses a step already in progress. Release resumes it, so the
        // panel does not save or reverse while the button is still down.
        if !captured && !calm {
            motion.persist |= self.stroll_moved;
            self.stroll_left = 0.0;
            self.stroll_moved = false;
            self.next_stroll = now + 15.0;
        } else if !captured {
            if self.stroll_left == 0.0 && now >= self.next_stroll {
                let room = if self.stroll_direction < 0.0 {
                    env.horizontal_room[0]
                } else {
                    env.horizontal_room[1]
                };
                self.stroll_left = room.max(0.0).min(24.0);
                self.next_stroll = now + 15.0;
                if self.stroll_left < 0.01 {
                    self.stroll_direction = -self.stroll_direction;
                }
            }
            if self.stroll_left > 0.0 {
                let room = if self.stroll_direction < 0.0 {
                    env.horizontal_room[0]
                } else {
                    env.horizontal_room[1]
                };
                let step = (dt * 14.0).min(self.stroll_left).min(room.max(0.0));
                motion.delta[0] = self.stroll_direction * step;
                self.stroll_left -= step;
                self.stroll_moved |= step > 0.0;
                if self.stroll_left < 0.01 || room <= step + 0.01 {
                    motion.persist |= self.stroll_moved;
                    self.stroll_left = 0.0;
                    self.stroll_moved = false;
                    self.stroll_direction = -self.stroll_direction;
                    self.next_stroll = now + 15.0;
                }
            }
        }

        if reduced_motion || !matches!(mood, Mood::Idle) {
            self.gaze = [0.0; 2];
        } else {
            for (value, target) in self.gaze.iter_mut().zip(look) {
                approach(value, target, dt);
            }
        }
        let settling = self
            .gaze
            .iter()
            .zip(look)
            .any(|(value, target)| (value - target).abs() > 0.04);
        // Dragging moves the native panel, never stretches the circular body.
        // Only the idle eyes follow the pointer; task poses keep facing their prop.
        for eye in &mut visual.eyes {
            for point in eye {
                point[0] += self.gaze[0];
                point[1] += self.gaze[1];
            }
        }
        motion.active = !reduced_motion && (env.dragging || self.stroll_left > 0.0 || settling);
        motion
    }
}

fn approach(value: &mut f64, target: f64, dt: f64) {
    *value += (target - *value) * (1.0 - (-10.0 * dt).exp());
    if (*value - target).abs() < 0.001 {
        *value = target;
    }
}
fn center(points: &[[f64; 2]]) -> [f64; 2] {
    let mut result = [0.0; 2];
    for p in points {
        result[0] += p[0];
        result[1] += p[1];
    }
    [
        result[0] / points.len() as f64,
        result[1] / points.len() as f64,
    ]
}

/// AppKit rounds window origins to whole points; Win32 uses physical pixels.
/// Keep fractional travel between frames so slow strolls retain their distance.
pub(super) fn pixel_motion(delta: [f64; 2], scale: f64, remainder: &mut [f64; 2]) -> [i32; 2] {
    let mut pixels = [0; 2];
    for axis in 0..2 {
        let amount = delta[axis] * scale + remainder[axis];
        pixels[axis] = amount.round() as i32;
        remainder[axis] = amount - pixels[axis] as f64;
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::super::visual::SIZE;
    use super::*;
    #[test]
    fn subpixel_stroll_preserves_distance_at_native_coordinate_boundaries() {
        for scale in [1.0, 1.25, 2.0] {
            let mut remainder = [0.0; 2];
            let mut actual = [0; 2];
            for frame in 1..=120 {
                let step = pixel_motion([-0.2, 0.2], scale, &mut remainder);
                actual[0] += step[0];
                actual[1] += step[1];
                assert!((actual[0] as f64 + frame as f64 * 0.2 * scale).abs() <= 0.500001);
                assert!((actual[1] as f64 - frame as f64 * 0.2 * scale).abs() <= 0.500001);
            }
            assert_eq!(actual, [(-24.0 * scale) as i32, (24.0 * scale) as i32]);
        }
    }
    fn frame() -> Visual {
        super::super::sim::BlobSim::new(true).sample(0.0)
    }
    #[test]
    fn task_pose_keeps_its_gaze_on_the_prop_after_idle_pointer_tracking() {
        let mut behavior = DesktopBehavior::new();
        let env = Environment {
            cursor: Some([140.0, 70.0]),
            ..Default::default()
        };
        for step in 0..120 {
            behavior.apply(&mut frame(), Mood::Idle, env, step as f64 / 60.0, false);
        }
        let mut animation = super::super::sim::BlobSim::new(true);
        animation.set_mood(Mood::Working, 0.0);
        let mut visual = animation.sample(0.0);
        let expected = visual;
        behavior.apply(&mut visual, Mood::Working, env, 2.0, false);
        assert_eq!(visual, expected);
    }
    #[test]
    fn restored_or_released_pet_keeps_its_elevated_position() {
        for reduced in [false, true] {
            for mood in [Mood::Idle, Mood::Thinking, Mood::Waiting] {
                let mut behavior = DesktopBehavior::new();
                let mut env = Environment {
                    floor_distance: 240.0,
                    horizontal_room: [200.0; 2],
                    ..Default::default()
                };
                for step in 0..1800 {
                    // Restored location, pickup, then release at the chosen height.
                    env.pressed = (60..90).contains(&step);
                    env.dragging = (65..90).contains(&step);
                    let motion =
                        behavior.apply(&mut frame(), mood, env, step as f64 / 60.0, reduced);
                    assert_eq!(motion.delta, [0.0; 2]);
                    assert!(
                        !motion.persist,
                        "native drag release owns placement persistence"
                    );
                }
            }
        }
    }
    #[test]
    fn press_owns_position_and_release_keeps_it_without_clipping() {
        let mut behavior = DesktopBehavior::new();
        let mut env = Environment {
            pressed: true,
            floor_distance: 120.0,
            horizontal_room: [200.0; 2],
            ..Default::default()
        };
        for step in 0..60 {
            env.dragging = step > 20;
            let mut visual = frame();
            let motion = behavior.apply(&mut visual, Mood::Idle, env, step as f64 / 60.0, false);
            assert_eq!(motion.delta, [0.0; 2]);
            assert!(!motion.persist);
            assert!(visual
                .body
                .iter()
                .chain(visual.eyes.iter().flatten())
                .all(|p| p.iter().all(|v| *v >= 0.0 && *v <= SIZE)));
        }
        let mut visual = frame();
        let before = visual.body;
        behavior.apply(&mut visual, Mood::Idle, env, 0.99, false);
        assert_eq!(visual.body, before, "dragging must not deform the body");
        env.pressed = false;
        env.dragging = false;
        let motion = behavior.apply(&mut frame(), Mood::Idle, env, 1.0, true);
        assert_eq!(motion.delta, [0.0; 2]);
        assert!(!motion.persist && !motion.active);
    }
    #[test]
    fn idle_stroll_is_bounded_and_stops_for_work_or_pointer() {
        for stop in [Mood::Waiting, Mood::Thinking, Mood::Idle] {
            let mut behavior = DesktopBehavior::new();
            let mut env = Environment {
                horizontal_room: [200.0; 2],
                ..Default::default()
            };
            let mut distance = 0.0;
            for step in 0..930 {
                let motion =
                    behavior.apply(&mut frame(), Mood::Idle, env, step as f64 / 60.0, false);
                distance += motion.delta[0].abs();
            }
            assert!(distance > 1.0 && distance <= 24.0);
            if matches!(stop, Mood::Idle) {
                env.cursor = Some([80.0, 80.0]);
            }
            let motion = behavior.apply(&mut frame(), stop, env, 15.5, false);
            assert_eq!(motion.delta, [0.0; 2]);
            assert!(motion.persist);
            let next = behavior.apply(&mut frame(), stop, env, 15.52, false);
            assert!(!next.persist);
        }
    }
    #[test]
    fn reduced_motion_stays_still_and_settled_gaze_does_not_demand_high_fps() {
        let mut behavior = DesktopBehavior::new();
        let env = Environment {
            cursor: Some([140.0, 70.0]),
            horizontal_room: [200.0; 2],
            ..Default::default()
        };
        let mut motion = Motion::default();
        for step in 0..180 {
            motion = behavior.apply(&mut frame(), Mood::Idle, env, step as f64 / 60.0, false);
        }
        assert!(!motion.active);
        assert!(behavior.gaze[0] > 1.0);
        for step in 180..1200 {
            motion = behavior.apply(&mut frame(), Mood::Idle, env, step as f64 / 60.0, true);
            assert_eq!(motion.delta, [0.0; 2]);
            assert!(!motion.active);
        }
        assert_eq!(behavior.gaze, [0.0; 2]);
    }
    #[test]
    fn press_before_drag_pauses_an_idle_stroll() {
        let mut behavior = DesktopBehavior::new();
        let mut env = Environment {
            horizontal_room: [80.0, 80.0],
            ..Default::default()
        };
        let mut walked = 0.0;
        for step in 0..960 {
            let motion = behavior.apply(&mut frame(), Mood::Idle, env, step as f64 / 60.0, false);
            walked += motion.delta[0];
            env.horizontal_room[0] += motion.delta[0];
            env.horizontal_room[1] -= motion.delta[0];
        }
        assert!(walked < -1.0 && walked >= -24.0);
        let held = env.horizontal_room;
        env.pressed = true;
        for step in 960..1020 {
            let motion = behavior.apply(&mut frame(), Mood::Idle, env, step as f64 / 60.0, false);
            assert_eq!(motion.delta, [0.0; 2]);
            assert!(!motion.persist);
        }
        env.pressed = false;
        let motion = behavior.apply(&mut frame(), Mood::Idle, env, 1020.0 / 60.0, false);
        assert!(motion.delta[0] < 0.0);
        assert!(motion.delta[0] >= -24.0);
        assert_eq!(env.horizontal_room, held);
    }
}
