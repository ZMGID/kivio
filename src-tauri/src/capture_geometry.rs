#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CaptureRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CaptureDisplay {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MappedSourceRect {
    pub(crate) display_index: usize,
    pub(crate) source_rect: CaptureRect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub(crate) struct CaptureMonitor {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale_factor: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub(crate) struct CaptureRegionPx {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) fn source_rect_for_region(
    region: CaptureRect,
    displays: &[CaptureDisplay],
) -> Option<MappedSourceRect> {
    if !is_positive_rect(region) {
        return None;
    }

    displays
        .iter()
        .enumerate()
        .filter_map(|(display_index, display)| {
            let clipped = intersect(region, (*display).into())?;
            let area = clipped.width * clipped.height;
            if area <= 0.0 {
                return None;
            }
            Some((
                area,
                MappedSourceRect {
                    display_index,
                    source_rect: CaptureRect {
                        x: clipped.x - display.x,
                        y: clipped.y - display.y,
                        width: clipped.width,
                        height: clipped.height,
                    },
                },
            ))
        })
        .max_by(|(a, _), (b, _)| a.total_cmp(b))
        .map(|(_, mapped)| mapped)
}

#[allow(dead_code)]
pub(crate) fn monitor_for_region(
    region: CaptureRect,
    monitors: &[CaptureMonitor],
) -> Option<usize> {
    if !is_positive_rect(region) {
        return None;
    }

    monitors
        .iter()
        .enumerate()
        .filter_map(|(idx, monitor)| {
            let clipped = intersect(region, monitor.logical_rect())?;
            let area = clipped.width * clipped.height;
            if area <= 0.0 {
                return None;
            }
            Some((area, idx))
        })
        .max_by(|(a, _), (b, _)| a.total_cmp(b))
        .map(|(_, idx)| idx)
}

/// Logical rectangles can overlap when each Windows monitor has a different
/// scale. A frozen desktop must bind to the physical display chosen for Lens.
pub(crate) fn monitor_for_physical_frame(
    target: CaptureMonitor,
    monitors: &[CaptureMonitor],
) -> Option<usize> {
    monitors.iter().position(|monitor| {
        monitor.x == target.x
            && monitor.y == target.y
            && monitor.width == target.width
            && monitor.height == target.height
    })
}

#[allow(dead_code)]
pub(crate) fn windows_monitor_region(
    region: CaptureRect,
    monitor: CaptureMonitor,
) -> Option<CaptureRegionPx> {
    if !is_positive_rect(region) {
        return None;
    }
    let scale = valid_scale(monitor.scale_factor);
    let monitor_logical = monitor.logical_rect();
    let clipped = intersect(region, monitor_logical)?;

    let left = ((clipped.x - monitor_logical.x) * scale).round() as i32;
    let top = ((clipped.y - monitor_logical.y) * scale).round() as i32;
    let right = ((clipped.x + clipped.width - monitor_logical.x) * scale).round() as i32;
    let bottom = ((clipped.y + clipped.height - monitor_logical.y) * scale).round() as i32;

    let left = left.clamp(0, monitor.width as i32);
    let top = top.clamp(0, monitor.height as i32);
    let right = right.clamp(left, monitor.width as i32);
    let bottom = bottom.clamp(top, monitor.height as i32);

    if right <= left || bottom <= top {
        return None;
    }

    Some(CaptureRegionPx {
        x: left as u32,
        y: top as u32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}

/// Map WebView client coordinates using the window's physical origin. Do not
/// guess a monitor from globally divided logical coordinates at mixed DPI.
pub(crate) fn windows_window_region(
    local: CaptureRect,
    window_x: i32,
    window_y: i32,
    pixel_ratio: f64,
    monitor: CaptureMonitor,
) -> Option<CaptureRegionPx> {
    let scale = valid_scale(pixel_ratio);
    windows_monitor_region(
        CaptureRect {
            x: window_x as f64 / scale + local.x,
            y: window_y as f64 / scale + local.y,
            width: local.width,
            height: local.height,
        },
        CaptureMonitor {
            scale_factor: scale,
            ..monitor
        },
    )
}

fn is_positive_rect(rect: CaptureRect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width > 0.0
        && rect.height > 0.0
}

#[allow(dead_code)]
fn valid_scale(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

fn intersect(a: CaptureRect, b: CaptureRect) -> Option<CaptureRect> {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    if right <= left || bottom <= top {
        return None;
    }
    Some(CaptureRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

impl CaptureMonitor {
    #[allow(dead_code)]
    fn logical_rect(self) -> CaptureRect {
        let scale = valid_scale(self.scale_factor);
        CaptureRect {
            x: self.x as f64 / scale,
            y: self.y as f64 / scale,
            width: self.width as f64 / scale,
            height: self.height as f64 / scale,
        }
    }
}

impl From<CaptureDisplay> for CaptureRect {
    fn from(display: CaptureDisplay) -> Self {
        Self {
            x: display.x,
            y: display.y,
            width: display.width,
            height: display.height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_local_crops_use_selected_physical_monitor_at_each_scale() {
        for (mx, my) in [(0, 0), (-1920, -1080), (3840, 180)] {
            for scale in [1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 3.0] {
                let monitor = CaptureMonitor {
                    x: mx,
                    y: my,
                    width: 1920,
                    height: 1080,
                    scale_factor: scale,
                };
                let rect = windows_window_region(
                    CaptureRect {
                        x: 10.0,
                        y: 12.0,
                        width: 30.0,
                        height: 20.0,
                    },
                    mx,
                    my,
                    scale,
                    monitor,
                )
                .unwrap();
                assert_eq!(rect.x, (10.0 * scale).round() as u32);
                assert_eq!(rect.y, (12.0 * scale).round() as u32);
                assert_eq!(rect.x + rect.width, (40.0 * scale).round() as u32);
                assert_eq!(rect.y + rect.height, (32.0 * scale).round() as u32);
            }
        }
    }

    #[test]
    fn window_crop_rounding_is_independent_of_monitor_origin() {
        for (mx, my) in [(0, 0), (-3840, -2160), (3840, 180)] {
            for scale in [1.1, 1.25, 1.5, 1.75, 2.25, 2.5, 2.75] {
                let monitor = CaptureMonitor {
                    x: mx,
                    y: my,
                    width: 1920,
                    height: 1080,
                    scale_factor: scale,
                };
                for x in 0..30 {
                    let rect = windows_window_region(
                        CaptureRect {
                            x: x as f64,
                            y: x as f64,
                            width: 7.0,
                            height: 7.0,
                        },
                        mx,
                        my,
                        scale,
                        monitor,
                    )
                    .unwrap();
                    assert_eq!(
                        rect.x,
                        (x as f64 * scale).round() as u32,
                        "origin={mx},{my} scale={scale} x={x}"
                    );
                    assert_eq!(rect.y, (x as f64 * scale).round() as u32);
                    assert_eq!(rect.x + rect.width, ((x + 7) as f64 * scale).round() as u32);
                }
            }
        }
    }

    #[test]
    fn physical_frame_lookup_is_unambiguous_with_overlapping_logical_monitors() {
        let primary = CaptureMonitor {
            x: 0,
            y: 0,
            width: 3840,
            height: 2160,
            scale_factor: 1.0,
        };
        let secondary = CaptureMonitor {
            x: 3840,
            y: 0,
            width: 1920,
            height: 1080,
            scale_factor: 2.0,
        };
        // The secondary's logical bounds lie entirely inside the primary's.
        assert_eq!(
            monitor_for_region(secondary.logical_rect(), &[secondary, primary]),
            Some(1)
        );
        assert_eq!(
            monitor_for_physical_frame(secondary, &[secondary, primary]),
            Some(0)
        );
        assert_eq!(
            monitor_for_physical_frame(secondary, &[primary, secondary]),
            Some(1)
        );
    }

    #[test]
    fn physical_frame_lookup_handles_negative_origins_portrait_and_hotplug() {
        for scale in [1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 3.0] {
            let target = CaptureMonitor {
                x: -1080,
                y: -901,
                width: 1080,
                height: 1920,
                scale_factor: scale,
            };
            assert_eq!(monitor_for_physical_frame(target, &[target]), Some(0));
            assert_eq!(monitor_for_physical_frame(target, &[]), None);
            let changed = CaptureMonitor {
                width: 1920,
                height: 1080,
                ..target
            };
            assert_eq!(monitor_for_physical_frame(target, &[changed]), None);
        }
    }

    #[test]
    fn source_rect_supports_negative_display_origins() {
        let displays = [
            CaptureDisplay {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
            },
            CaptureDisplay {
                x: -1280.0,
                y: -120.0,
                width: 1280.0,
                height: 720.0,
            },
        ];
        let region = CaptureRect {
            x: -1180.0,
            y: -20.0,
            width: 250.0,
            height: 160.0,
        };

        let mapped = source_rect_for_region(region, &displays).expect("region should map");

        assert_eq!(mapped.display_index, 1);
        assert_eq!(
            mapped.source_rect,
            CaptureRect {
                x: 100.0,
                y: 100.0,
                width: 250.0,
                height: 160.0,
            }
        );
    }

    #[test]
    fn source_rect_clips_to_display_with_largest_overlap() {
        let displays = [
            CaptureDisplay {
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 800.0,
            },
            CaptureDisplay {
                x: 1000.0,
                y: 0.0,
                width: 1000.0,
                height: 800.0,
            },
        ];
        let region = CaptureRect {
            x: 990.0,
            y: 40.0,
            width: 80.0,
            height: 120.0,
        };

        let mapped = source_rect_for_region(region, &displays).expect("region should map");

        assert_eq!(mapped.display_index, 1);
        assert_eq!(
            mapped.source_rect,
            CaptureRect {
                x: 0.0,
                y: 40.0,
                width: 70.0,
                height: 120.0,
            }
        );
    }

    #[test]
    fn windows_monitor_selection_uses_logical_monitor_bounds() {
        let monitors = [
            CaptureMonitor {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale_factor: 1.0,
            },
            CaptureMonitor {
                x: 1920,
                y: -180,
                width: 2560,
                height: 1440,
                scale_factor: 1.25,
            },
        ];
        let region = CaptureRect {
            x: 1616.0,
            y: -64.0,
            width: 240.0,
            height: 120.0,
        };

        assert_eq!(monitor_for_region(region, &monitors), Some(1));
    }
}
