/// Mouse delta magnitude must exceed this threshold to trigger screensaver exit.
/// Filters sub-pixel hardware jitter. Known trade-off: deliberate micro-movements
/// below threshold won't exit. This is a compile-time constant.
pub const MOUSE_EXIT_THRESHOLD: f64 = 10.0;

/// Returns true if the given mouse delta should trigger screensaver exit.
pub fn should_exit_on_mouse(delta: (f64, f64)) -> bool {
    let (dx, dy) = delta;
    (dx * dx + dy * dy).sqrt() > MOUSE_EXIT_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_large_delta_exits() {
        assert!(should_exit_on_mouse((20.0, 0.0)));
        assert!(should_exit_on_mouse((0.0, 15.0)));
        assert!(should_exit_on_mouse((10.1, 0.0)));
    }

    #[test]
    fn test_small_delta_does_not_exit() {
        assert!(!should_exit_on_mouse((0.0, 0.0)));
        assert!(!should_exit_on_mouse((1.0, 1.0)));
        assert!(!should_exit_on_mouse((7.0, 7.0)));
    }

    #[test]
    fn test_threshold_boundary() {
        // exactly at threshold: should NOT exit (strictly greater than)
        assert!(!should_exit_on_mouse((MOUSE_EXIT_THRESHOLD, 0.0)));
    }
}
