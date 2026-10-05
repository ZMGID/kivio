//! Real AppKit regression: cargo test --manifest-path src-tauri/Cargo.toml --test traffic-lights
#![allow(deprecated)]
#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))]

#[cfg(target_os = "macos")]
#[path = "../src/windows/traffic_lights.rs"]
mod traffic_lights;

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSPoint, NSRect, NSSize};
    use objc::{class, msg_send, sel, sel_impl};
    extern "C" {
        fn objc_initWeak(location: *mut id, object: id) -> id;
        fn objc_loadWeakRetained(location: *mut id) -> id;
        fn objc_destroyWeak(location: *mut id);
    }

    unsafe fn assert_aligned(window: id, center: f64, spacing: f64) {
        for index in 0..3_u64 {
            let button: id = msg_send![window, standardWindowButton: index];
            let bounds: NSRect = msg_send![button, bounds];
            let content: id = msg_send![window, contentView];
            let rect: NSRect = msg_send![button, convertRect: bounds toView: content];
            let content_bounds: NSRect = msg_send![content, bounds];
            let y = content_bounds.size.height - rect.origin.y - rect.size.height / 2.0;
            let x = rect.origin.x - content_bounds.origin.x;
            assert!(
                (y - center).abs() < 0.25,
                "button {index} not aligned: {y}, expected {center}"
            );
            assert!(
                (x - (22.0 + index as f64 * spacing)).abs() < 0.25,
                "button {index} horizontal drift: {x}, spacing {spacing}"
            );
        }
    }

    unsafe {
        let pool: id = msg_send![class!(NSAutoreleasePool), new];
        let _: id = msg_send![class!(NSApplication), sharedApplication];
        let mut recoveries = 0;
        for _ in 0..20 {
            let lifetime_pool: id = msg_send![class!(NSAutoreleasePool), new];
            let window: id = msg_send![class!(NSWindow), alloc];
            let window: id = msg_send![window,
            initWithContentRect: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0))
            styleMask: (1_u64 | 2 | 4 | 8 | (1 << 15)) backing: 2_u64 defer: false];
            let _: () = msg_send![window, setReleasedWhenClosed: false];
            let mut weak_window = nil;
            objc_initWeak(&mut weak_window, window);
            let _: () = msg_send![window, setTitlebarAppearsTransparent: true];
            let _: () = msg_send![window, setTitleVisibility: 1_u64];
            let _: () = msg_send![window, orderFront: nil];
            let close: id = msg_send![window, standardWindowButton: 0_u64];
            let mini: id = msg_send![window, standardWindowButton: 1_u64];
            let close_rect: NSRect = msg_send![close, frame];
            let mini_rect: NSRect = msg_send![mini, frame];
            let native_spacing = mini_rect.origin.x - close_rect.origin.x;
            traffic_lights::apply_saved_position(window);
            assert_eq!(
                traffic_lights::measure_traffic_light_center_y(window),
                Some(26.0)
            );

            for center in [26.0, 30.0, 36.0] {
                assert!(traffic_lights::restore_macos_traffic_lights(window, center));
                let close: id = msg_send![window, standardWindowButton: 0_u64];
                let titlebar: id = msg_send![close, superview];
                // Exercise each button independently as well as whole-group drift,
                // including x changes that must not redefine the native spacing.
                for changed in [&[1_u64][..], &[2][..], &[0][..], &[0, 1, 2][..]] {
                    for (dx, dy) in [(0.0, 14.0), (11.0, 0.0), (-7.0, -9.0)] {
                        for explicit_layout in [true, false] {
                            for &index in changed {
                                let button: id = msg_send![window, standardWindowButton: index];
                                let mut rect: NSRect = msg_send![button, frame];
                                rect.origin.x += dx;
                                rect.origin.y += dy;
                                let _: () = msg_send![button, setFrameOrigin: rect.origin];
                            }
                            if explicit_layout {
                                let _: () = msg_send![titlebar, setNeedsLayout: true];
                                let _: () = msg_send![titlebar, layoutSubtreeIfNeeded];
                            } else {
                                // No forced layout callback: exercise AppKit's actual drawing path.
                                let _: () = msg_send![window, displayIfNeeded];
                            }
                            assert_aligned(window, center, native_spacing);
                            assert!(traffic_lights::restore_macos_traffic_lights(window, center));
                            assert_aligned(window, center, native_spacing);
                            recoveries += 1;
                        }
                    }
                }
                for size in [NSSize::new(640.0, 400.0), NSSize::new(1100.0, 800.0)] {
                    let _: () = msg_send![window, setContentSize: size];
                    let content: id = msg_send![window, contentView];
                    let root: id = msg_send![content, superview];
                    let _: () = msg_send![root, layoutSubtreeIfNeeded];
                    assert_aligned(window, center, native_spacing);
                }
                let _: () = msg_send![window, orderOut: nil];
                traffic_lights::apply_saved_position(window);
                let _: () = msg_send![window, orderFront: nil];
                assert_aligned(window, center, native_spacing);
            }
            // Keep an old observed button alive beyond its window. Cleanup must
            // release the window and stop callbacks into its now-dead pointer.
            let _: () = msg_send![mini, retain];
            let _: () = msg_send![window, close];
            let _: () = msg_send![window, release];
            let _: () = msg_send![lifetime_pool, drain];
            let alive = objc_loadWeakRetained(&mut weak_window);
            assert!(
                alive == nil,
                "traffic-light observations kept the closed window alive"
            );
            objc_destroyWeak(&mut weak_window);
            let mut frame: NSRect = msg_send![mini, frame];
            frame.origin.y += 14.0;
            let _: () = msg_send![mini, setFrameOrigin: frame.origin];
            let _: () = msg_send![mini, release];
        }
        println!("{recoveries} geometry recoveries passed across 20 window lifecycles");
        let _: () = msg_send![pool, drain];
    }
}
