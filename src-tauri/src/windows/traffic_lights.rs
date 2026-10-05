// macOS traffic-light geometry and its window-owned AppKit lifecycle.
#![allow(deprecated)]

/// 保留现有窗口左边距；纵向跟随页面顶栏控件的实际中心。
#[cfg(target_os = "macos")]
const CHAT_TRAFFIC_LIGHT_X: f64 = 22.0;

/// 把 close 按钮的 bounds 转到 contentView 坐标系，换算成「距内容顶缘」。
/// 按 contentView 的实际坐标方向换算，避免依赖 NSView 子类的翻转约定。
/// Overlay 标题栏下 contentView 铺满整个窗口 frame，所以这就是 CSS 的 y。
#[cfg(target_os = "macos")]
pub(super) unsafe fn measure_traffic_light_center_y(window: cocoa::base::id) -> Option<f64> {
    use cocoa::base::id;
    use objc::{msg_send, sel, sel_impl};

    let close: id = msg_send![window, standardWindowButton: 0_u64];
    measure_button_position(window, close).map(|point| point.y)
}

/// (left edge, vertical center), relative to the content's top-left corner.
unsafe fn measure_button_position(
    window: cocoa::base::id,
    button: cocoa::base::id,
) -> Option<cocoa::foundation::NSPoint> {
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSPoint, NSRect};
    use objc::{msg_send, sel, sel_impl};

    let content: id = msg_send![window, contentView];
    if button == nil || content == nil {
        return None;
    }
    let bounds: NSRect = msg_send![button, bounds];
    let in_content: NSRect = msg_send![button, convertRect: bounds toView: content];
    let content_bounds: NSRect = msg_send![content, bounds];
    if content_bounds.size.height <= 0.0 || in_content.size.height <= 0.0 {
        return None;
    }
    let flipped: bool = msg_send![content, isFlipped];
    let center = in_content.origin.y + in_content.size.height / 2.0;
    let y = if flipped {
        center - content_bounds.origin.y
    } else {
        content_bounds.origin.y + content_bounds.size.height - center
    };
    Some(NSPoint::new(
        in_content.origin.x - content_bounds.origin.x,
        y,
    ))
}

/// Reapply the saved target when showing/reusing the window. Main thread only.
pub(super) unsafe fn apply_saved_position(window: cocoa::base::id) {
    hide_macos_window_title(window);
    let observer = observe_macos_traffic_light_layout(window, None);
    let center_y = *(*observer).get_ivar::<f64>("targetCenter");
    restore_macos_traffic_lights(window, center_y);
}

// NSWindow 关联持有观察者，观察者只保存弱窗口指针；窗口销毁时解除订阅。
#[cfg(target_os = "macos")]
unsafe fn observe_macos_traffic_light_layout(
    window: cocoa::base::id,
    center_y: Option<f64>,
) -> cocoa::base::id {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::runtime::{Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};
    use std::sync::OnceLock;
    extern "C" {
        fn objc_getAssociatedObject(object: id, key: *const u8) -> id;
        fn objc_setAssociatedObject(object: id, key: *const u8, value: id, policy: usize);
        fn object_setClass(
            object: id,
            class: *const objc::runtime::Class,
        ) -> *const objc::runtime::Class;
    }
    static KEY: u8 = 0;
    static CLASS: OnceLock<usize> = OnceLock::new();
    extern "C" fn update(this: &mut Object, _: Sel, _: id) {
        unsafe {
            restore_macos_traffic_lights(
                *this.get_ivar::<id>("targetWindow"),
                *this.get_ivar::<f64>("targetCenter"),
            );
        }
    }
    extern "C" fn dealloc(this: &mut Object, _: Sel) {
        unsafe {
            let center: id = msg_send![class!(NSNotificationCenter), defaultCenter];
            let _: () = msg_send![center, removeObserver: this as *mut Object];
            this.set_ivar("targetWindow", nil);
            let views = *this.get_ivar::<id>("observedViews");
            if views != nil {
                let _: () = msg_send![views, release];
            }
            let _: () = msg_send![super(this, class!(NSObject)), dealloc];
        }
    }
    let class = *CLASS.get_or_init(|| {
        let mut decl =
            objc::declare::ClassDecl::new("KivioTrafficLightLayoutObserver", class!(NSObject))
                .unwrap();
        decl.add_ivar::<id>("targetWindow");
        decl.add_ivar::<f64>("targetCenter");
        decl.add_ivar::<f64>("buttonSpacing");
        decl.add_ivar::<id>("observedViews");
        decl.add_ivar::<bool>("busy");

        decl.add_method(
            sel!(windowUpdated:),
            update as extern "C" fn(&mut Object, Sel, id),
        );
        decl.add_method(sel!(dealloc), dealloc as extern "C" fn(&mut Object, Sel));
        decl.register() as *const _ as usize
    }) as *const objc::runtime::Class;
    let mut observer = objc_getAssociatedObject(window, &KEY);
    if observer == nil {
        observer = msg_send![class, new];
        (*observer).set_ivar("targetWindow", window);
        // Match the 52px header before the first renderer measurement arrives.
        (*observer).set_ivar("targetCenter", center_y.unwrap_or(26.0));
        (*observer).set_ivar("buttonSpacing", 0.0);
        (*observer).set_ivar("observedViews", nil);
        (*observer).set_ivar("busy", false);

        objc_setAssociatedObject(window, &KEY, observer, 1);
        let center: id = msg_send![class!(NSNotificationCenter), defaultCenter];
        for event in [
            // AppKit may reposition the buttons without changing the container frame.
            // Check after the native update pass as well, including while inactive.
            "NSWindowDidUpdateNotification",
            "NSWindowDidResizeNotification",
            "NSWindowDidBecomeKeyNotification",
            "NSWindowDidResignKeyNotification",
            "NSWindowDidExitFullScreenNotification",
        ] {
            let name = NSString::alloc(nil).init_str(event);
            let _: () = msg_send![center, addObserver: observer selector: sel!(windowUpdated:) name: name object: window];
            let _: () = msg_send![name, release];
        }
        let _: () = msg_send![observer, release];
    }
    if let Some(center_y) = center_y {
        (*observer).set_ivar("targetCenter", center_y);
    }
    // 在标题栏自身的布局入口保持高度，避免拖动追踪循环绕过窗口更新通知。
    // 只给此窗口的容器安装无额外 ivar 的子类，不修改系统类或其它窗口。
    extern "C" fn set_frame(this: &mut Object, _: Sel, mut frame: cocoa::foundation::NSRect) {
        unsafe {
            let window: id = msg_send![this, window];
            // During fullscreen teardown the container briefly has no owning window.
            let observer = if window == nil {
                nil
            } else {
                objc_getAssociatedObject(window, &KEY)
            };
            let mask: u64 = if window == nil {
                0
            } else {
                msg_send![window, styleMask]
            };
            if observer != nil && mask & (1 << 14) == 0 {
                let height = *(*observer).get_ivar::<f64>("targetCenter") * 2.0;
                frame.origin.y += frame.size.height - height;
                frame.size.height = height;
            }
            // AppKit may add another dynamic subclass during fullscreen transitions.
            // Dispatch above our own implementation, not above the object's current class.
            let mut owner = this.class();
            while !owner.name().starts_with("KivioAlignedTitlebar_") {
                owner = owner.superclass().unwrap();
            }
            let superclass = owner.superclass().unwrap();
            let _: () = msg_send![super(this, superclass), setFrame: frame];
            if observer != nil {
                restore_macos_traffic_lights(window, *(*observer).get_ivar::<f64>("targetCenter"));
            }
        }
    }
    // setFrame: is not the end of AppKit's layout pass. The titlebar can lay out
    // its buttons afterwards, even when the container frame did not change.
    // Restore synchronously after super.layout, before that geometry is drawn;
    // window notifications and renderer timers alone leave a visible gap.
    extern "C" fn layout(this: &mut Object, _: Sel) {
        unsafe {
            let mut owner = this.class();
            while !owner.name().starts_with("KivioAlignedTitlebar_") {
                owner = owner.superclass().unwrap();
            }
            let _: () = msg_send![super(this, owner.superclass().unwrap()), layout];
            let window: id = msg_send![this, window];
            if window != nil {
                let observer = objc_getAssociatedObject(window, &KEY);
                if observer != nil {
                    restore_macos_traffic_lights(
                        window,
                        *(*observer).get_ivar::<f64>("targetCenter"),
                    );
                }
            }
        }
    }
    let button: id = msg_send![window, standardWindowButton: 0_u64];
    if button == nil {
        return observer;
    }
    // Capture AppKit's native spacing before the first alignment, once per
    // window. Later displaced buttons must never become the new spacing source.
    if *(*observer).get_ivar::<f64>("buttonSpacing") <= 0.0 {
        let mini: id = msg_send![window, standardWindowButton: 1_u64];
        if mini != nil {
            let close_rect: cocoa::foundation::NSRect = msg_send![button, frame];
            let mini_rect: cocoa::foundation::NSRect = msg_send![mini, frame];
            let spacing = mini_rect.origin.x - close_rect.origin.x;
            if spacing.is_finite() && spacing > 0.0 {
                (*observer).set_ivar("buttonSpacing", spacing);
            }
        }
    }
    let parent: id = msg_send![button, superview];
    if parent == nil {
        return observer;
    }
    let container: id = msg_send![parent, superview];
    // Direct frame edits do not necessarily schedule layout. Observe them
    // synchronously as well, before a display-only pass can paint displaced
    // controls. Own the observed views until unsubscribing; AppKit can replace
    // or temporarily detach them while entering/leaving fullscreen.
    let mini: id = msg_send![window, standardWindowButton: 1_u64];
    let zoom: id = msg_send![window, standardWindowButton: 2_u64];
    if mini != nil && zoom != nil {
        let views = [button, mini, zoom, parent];
        let previous = *(*observer).get_ivar::<id>("observedViews");
        let count: usize = if previous == nil {
            0
        } else {
            msg_send![previous, count]
        };
        let unchanged = count == views.len()
            && views.iter().enumerate().all(|(index, view)| {
                let old: id = msg_send![previous, objectAtIndex: index];
                old == *view
            });
        if !unchanged {
            let center: id = msg_send![class!(NSNotificationCenter), defaultCenter];
            let name = NSString::alloc(nil).init_str("NSViewFrameDidChangeNotification");
            for index in 0..count {
                let old: id = msg_send![previous, objectAtIndex: index];
                let _: () = msg_send![center, removeObserver: observer name: name object: old];
            }
            let current: id = msg_send![class!(NSArray), alloc];
            let current: id =
                msg_send![current, initWithObjects: views.as_ptr() count: views.len()];
            (*observer).set_ivar("observedViews", current);
            if previous != nil {
                let _: () = msg_send![previous, release];
            }
            for view in views {
                let _: () = msg_send![view, setPostsFrameChangedNotifications: true];
                let _: () = msg_send![center, addObserver: observer selector: sel!(windowUpdated:) name: name object: view];
            }
            let _: () = msg_send![name, release];
        }
    }
    // AppKit can replace these views during fullscreen transitions. Look them up
    // on each entry; only subclass this window's instances, never a system class.
    for (view, resize_container) in [(container, true), (parent, false)] {
        if view == nil {
            continue;
        }
        let mut installed = false;
        let mut ancestor = Some((*view).class());
        while let Some(class) = ancestor {
            if class.name().starts_with("KivioAlignedTitlebar_") {
                installed = true;
                break;
            }
            ancestor = class.superclass();
        }
        if installed {
            continue;
        }
        let base = (*view).class();
        let name = format!("KivioAlignedTitlebar_{}", base.name());
        let subclass = if let Some(class) = objc::runtime::Class::get(&name) {
            class
        } else {
            let mut decl = objc::declare::ClassDecl::new(&name, base).unwrap();
            if resize_container {
                decl.add_method(
                    sel!(setFrame:),
                    set_frame as extern "C" fn(&mut Object, Sel, cocoa::foundation::NSRect),
                );
            }
            decl.add_method(sel!(layout), layout as extern "C" fn(&mut Object, Sel));
            decl.register()
        };
        object_setClass(view, subclass);
    }
    observer
}

/// 原生位置由这里唯一维护；builder 不再设置会在每次重绘覆盖校准的 inset。
/// 全屏期间由系统管理标题栏，不修改也不采样隐藏按钮。
#[cfg(target_os = "macos")]
pub(super) unsafe fn restore_macos_traffic_lights(window: cocoa::base::id, center_y: f64) -> bool {
    let observer = observe_macos_traffic_light_layout(window, Some(center_y));
    if *(*observer).get_ivar::<bool>("busy") {
        return false;
    }
    (*observer).set_ivar("busy", true);
    let spacing = *(*observer).get_ivar::<f64>("buttonSpacing");
    let aligned = layout_macos_traffic_lights(window, center_y, spacing);
    (*observer).set_ivar("busy", false);
    aligned
}

#[cfg(target_os = "macos")]
unsafe fn layout_macos_traffic_lights(
    window: cocoa::base::id,
    center_y: f64,
    spacing: f64,
) -> bool {
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSPoint, NSRect};
    use objc::{msg_send, sel, sel_impl};

    let mask: u64 = msg_send![window, styleMask];
    if mask & (1 << 14) != 0 || spacing <= 0.0 {
        return false;
    }
    let close: id = msg_send![window, standardWindowButton: 0_u64];
    let mini: id = msg_send![window, standardWindowButton: 1_u64];
    let zoom: id = msg_send![window, standardWindowButton: 2_u64];
    if close == nil || mini == nil || zoom == nil {
        return false;
    }
    let parent: id = msg_send![close, superview];
    if parent == nil {
        return false;
    }
    let container: id = msg_send![parent, superview];
    if container == nil {
        return false;
    }
    let content: id = msg_send![window, contentView];
    if content == nil {
        return false;
    }
    let buttons = [close, mini, zoom];
    let is_aligned = |(index, button): (usize, &id)| {
        measure_button_position(window, *button).is_some_and(|point| {
            (point.y - center_y).abs() < 0.25
                && (point.x - (CHAT_TRAFFIC_LIGHT_X + index as f64 * spacing)).abs() < 0.25
        })
    };
    let window_rect: NSRect = msg_send![window, frame];
    let mut frame: NSRect = msg_send![container, frame];
    let height = center_y * 2.0;
    let top = window_rect.size.height - height;
    let container_aligned =
        (frame.size.height - height).abs() < 0.25 && (frame.origin.y - top).abs() < 0.25;
    if container_aligned && buttons.iter().enumerate().all(is_aligned) {
        return true;
    }
    if !container_aligned {
        frame.size.height = height;
        frame.origin.y = top;
        let _: () = msg_send![container, setFrame: frame];
    }
    // Convert each target from content coordinates, so an intermediate titlebar
    // offset or flipped coordinate system cannot shift the whole group.
    let bounds: NSRect = msg_send![content, bounds];
    let flipped: bool = msg_send![content, isFlipped];
    let y = bounds.origin.y
        + if flipped {
            center_y
        } else {
            bounds.size.height - center_y
        };
    for (index, button) in buttons.into_iter().enumerate() {
        if is_aligned((index, &button)) {
            continue;
        }
        let parent: id = msg_send![button, superview];
        if parent == nil {
            return false;
        }
        let rect: NSRect = msg_send![button, frame];
        let target = NSPoint::new(
            bounds.origin.x + CHAT_TRAFFIC_LIGHT_X + index as f64 * spacing + rect.size.width / 2.0,
            y,
        );
        let point: NSPoint = msg_send![parent, convertPoint: target fromView: content];
        let origin = NSPoint::new(
            point.x - rect.size.width / 2.0,
            point.y - rect.size.height / 2.0,
        );
        let _: () = msg_send![button, setFrameOrigin: origin];
    }
    buttons.iter().enumerate().all(is_aligned)
}

/// NSWindowTitleHidden — 隐藏 Overlay 标题栏中的窗口标题文字。
#[cfg(target_os = "macos")]
unsafe fn hide_macos_window_title(window: cocoa::base::id) {
    use objc::{msg_send, sel, sel_impl};

    const NS_WINDOW_TITLE_HIDDEN: u64 = 1;
    let _: () = msg_send![window, setTitleVisibility: NS_WINDOW_TITLE_HIDDEN];
}
