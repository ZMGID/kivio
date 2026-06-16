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
    fn source_rect_is_display_relative_without_y_flip() {
        let displays = [
            CaptureDisplay {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
            },
            CaptureDisplay {
                x: 1440.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
        ];
        let region = CaptureRect {
            x: 1520.0,
            y: 120.0,
            width: 300.0,
            height: 200.0,
        };

        let mapped = source_rect_for_region(region, &displays).expect("region should map");

        assert_eq!(mapped.display_index, 1);
        assert_eq!(
            mapped.source_rect,
            CaptureRect {
                x: 80.0,
                y: 120.0,
                width: 300.0,
                height: 200.0,
            }
        );
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

    #[test]
    fn windows_physical_region_uses_monitor_local_logical_origin() {
        let monitor = CaptureMonitor {
            x: 1920,
            y: -180,
            width: 2560,
            height: 1440,
            scale_factor: 1.25,
        };
        let region = CaptureRect {
            x: 1616.0,
            y: -64.0,
            width: 240.0,
            height: 120.0,
        };

        let mapped = windows_monitor_region(region, monitor).expect("region should map to monitor");

        assert_eq!(
            mapped,
            CaptureRegionPx {
                x: 100,
                y: 100,
                width: 300,
                height: 150,
            }
        );
    }
}
