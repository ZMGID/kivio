//! Original Kivio silhouette and upper-right resting gaze (kivioBlobSim.ts).
//! A bounded native response: no idle timer and no transcript layout animation.
use gpui_kit::{
    component::button::{Button, ButtonVariants},
    *,
};
use std::time::Duration;

#[derive(Default)]
pub struct Momo {
    poke: usize,
}
impl Render for Momo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let image = img("kivio/momo.svg")
            .absolute()
            .top_0()
            .left_0()
            .w(px(56.))
            .h(px(56.));
        let image = if self.poke > 0 {
            image
                .with_animation(
                    ("momo-poke", self.poke),
                    Animation::new(Duration::from_millis(480)).with_max_fps(60.),
                    |image, phase| image.top(px(-5. * (phase * std::f32::consts::PI).sin())),
                )
                .into_any_element()
        } else {
            image.into_any_element()
        };
        Button::new("momo")
            .ghost()
            .p_0()
            .w(px(56.))
            .h(px(56.))
            .accessibility_label("轻碰 Momo")
            .tooltip("Momo")
            .child(div().relative().w(px(56.)).h(px(56.)).child(image))
            .on_click(cx.listener(|this, _, _, cx| {
                this.poke = this.poke.wrapping_add(1);
                cx.notify();
            }))
    }
}
