//! Shared pointer intent for nested menus, including cancellable hover grace.
//!
//! Feed trigger enter/move events into `enter`/`moved`, then open immediately
//! on `Open`, or use `defer` on `Defer`. The deferred callback must check that
//! its parent and source submenu are still open and its target is still pending.
//! Call `leave` on trigger exit, `cancel` on clicks/keyboard/dismissal, and
//! `contains_pointer` from a window mouse listener to dismiss outside the safe
//! trigger → child corridor. This works for children on either side.

use gpui_kit::{Bounds, Context, Pixels, Point, Task, px};
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum HoverAction {
    None,
    Open,
    Defer,
}

pub struct HoverIntent<K> {
    origin: Option<Point<Pixels>>,
    pending: Option<K>,
    pointer: Option<Point<Pixels>>,
    task: Option<Task<()>>,
}

impl<K> Default for HoverIntent<K> {
    fn default() -> Self {
        Self {
            origin: None,
            pending: None,
            pointer: None,
            task: None,
        }
    }
}

impl<K: Clone + PartialEq> HoverIntent<K> {
    pub fn cancel(&mut self) {
        self.task = None;
        self.pending = None;
        self.pointer = None;
    }

    pub fn reset(&mut self) {
        self.cancel();
        self.origin = None;
    }

    pub fn record_origin(&mut self, pointer: Point<Pixels>) {
        self.origin = Some(pointer);
    }

    pub fn pending(&self) -> Option<&K> {
        self.pending.as_ref()
    }

    fn toward_child(
        &self,
        pointer: Point<Pixels>,
        bounds: Option<Bounds<Pixels>>,
        left: bool,
    ) -> bool {
        self.origin
            .zip(bounds)
            .is_some_and(|(origin, bounds)| corridor(origin, pointer, bounds, left))
    }

    pub fn enter(
        &mut self,
        current: Option<&K>,
        target: &K,
        pointer: Point<Pixels>,
        bounds: Option<Bounds<Pixels>>,
        left: bool,
    ) -> HoverAction {
        self.cancel();
        if current == Some(target) {
            self.record_origin(pointer);
            return HoverAction::None;
        }
        if current.is_some() && self.toward_child(pointer, bounds, left) {
            self.pending = Some(target.clone());
            self.pointer = Some(pointer);
            HoverAction::Defer
        } else {
            HoverAction::Open
        }
    }

    pub fn moved(
        &mut self,
        current: Option<&K>,
        target: &K,
        pointer: Point<Pixels>,
        bounds: Option<Bounds<Pixels>>,
        left: bool,
    ) -> HoverAction {
        if current == Some(target) {
            self.record_origin(pointer);
        } else if self.pending() == Some(target) {
            let forward = self.pointer.map_or(0.0, |previous| {
                f32::from(pointer.x - previous.x) * if left { -1.0 } else { 1.0 }
            });
            if !self.toward_child(pointer, bounds, left) || forward <= -2.0 {
                self.cancel();
                return HoverAction::Open;
            } else if forward >= 2.0 {
                // Renew grace while progressing toward the child; a pause switches.
                return self.enter(current, target, pointer, bounds, left);
            }
        }
        HoverAction::None
    }

    pub fn leave(&mut self, target: &K) {
        if self.pending() == Some(target) {
            self.cancel();
        }
    }

    pub fn contains_pointer(
        &mut self,
        trigger: Bounds<Pixels>,
        child: Option<Bounds<Pixels>>,
        pointer: Point<Pixels>,
        left: bool,
    ) -> bool {
        if trigger.contains(&pointer) {
            self.record_origin(pointer);
            return true;
        }
        let Some(child) = child else { return true };
        child.contains(&pointer) || self.toward_child(pointer, Some(child), left)
    }

    pub fn defer<T: 'static>(
        &mut self,
        cx: &mut Context<T>,
        apply: impl FnOnce(&mut T, &mut Context<T>) + 'static,
    ) {
        self.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let _ = this.update(cx, apply);
        }));
    }
}

/// Triangle from the active trigger's last pointer position to the near child edge.
fn corridor(
    origin: Point<Pixels>,
    pointer: Point<Pixels>,
    submenu: Bounds<Pixels>,
    on_left: bool,
) -> bool {
    let edge = if on_left {
        submenu.right()
    } else {
        submenu.left()
    };
    let direction = if on_left { -1.0 } else { 1.0 };
    let distance = f32::from(edge - origin.x) * direction;
    let advance = f32::from(pointer.x - origin.x) * direction;
    if distance <= 0.0 || advance <= 0.0 || advance > distance + 8.0 {
        return false;
    }
    let fraction = (advance / distance).min(1.0);
    let top = origin.y + (submenu.top() - px(8.0) - origin.y) * fraction;
    let bottom = origin.y + (submenu.bottom() + px(8.0) - origin.y) * fraction;
    pointer.y >= top && pointer.y <= bottom
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, size};

    #[test]
    fn diagonal_travel_renews_grace_but_reversing_switches_on_either_side() {
        for left in [false, true] {
            let p = |x: f32, y: f32| point(px(if left { 400.0 - x } else { x }), px(y));
            let child = Bounds::new(
                p(if left { 280.0 } else { 200.0 }, 80.0),
                size(px(80.0), px(160.0)),
            );
            let mut intent = HoverIntent::<usize>::default();
            intent.record_origin(p(100.0, 100.0));
            assert_eq!(
                intent.enter(Some(&0), &1, p(150.0, 140.0), Some(child), left),
                HoverAction::Defer
            );
            assert_eq!(
                intent.moved(Some(&0), &1, p(153.0, 141.0), Some(child), left),
                HoverAction::Defer
            );
            assert_eq!(
                intent.moved(Some(&0), &1, p(153.5, 141.0), Some(child), left),
                HoverAction::None
            );
            assert_eq!(
                intent.moved(Some(&0), &1, p(151.0, 141.0), Some(child), left),
                HoverAction::Open
            );
            assert!(intent.pending().is_none());
        }
    }

    #[test]
    fn leaving_or_dismissing_cancels_pending_switches() {
        let mut intent = HoverIntent::<usize>::default();
        let child = Bounds::new(point(px(200.0), px(80.0)), size(px(80.0), px(160.0)));
        let origin = point(px(100.0), px(100.0));
        let pointer = point(px(150.0), px(140.0));
        intent.record_origin(origin);
        assert_eq!(
            intent.enter(Some(&0), &1, pointer, Some(child), false),
            HoverAction::Defer
        );
        intent.leave(&2);
        assert_eq!(intent.pending(), Some(&1));
        intent.leave(&1);
        assert!(intent.pending().is_none());
        assert_eq!(
            intent.moved(Some(&0), &1, pointer, Some(child), false),
            HoverAction::None
        );
        intent.enter(Some(&0), &1, pointer, Some(child), false);
        intent.reset();
        assert!(intent.pending().is_none());
        assert_eq!(
            intent.enter(Some(&0), &1, pointer, Some(child), false),
            HoverAction::Open
        );
    }

    #[test]
    fn trigger_child_and_corridor_are_safe_but_unrelated_space_is_not() {
        let mut intent = HoverIntent::<usize>::default();
        let trigger = Bounds::new(point(px(50.0), px(85.0)), size(px(100.0), px(30.0)));
        let child = Bounds::new(point(px(200.0), px(80.0)), size(px(80.0), px(160.0)));
        assert!(intent.contains_pointer(trigger, Some(child), trigger.center(), false));
        assert!(intent.contains_pointer(trigger, Some(child), point(px(175.0), px(140.0)), false));
        assert!(intent.contains_pointer(trigger, Some(child), child.center(), false));
        assert!(!intent.contains_pointer(trigger, Some(child), point(px(175.0), px(300.0)), false));
        assert_eq!(
            intent.enter(
                Some(&0),
                &1,
                point(px(100.0), px(140.0)),
                Some(child),
                false
            ),
            HoverAction::Open
        );
    }
}
