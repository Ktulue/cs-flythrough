use glam::{Mat4, Vec3};
use std::time::Instant;

/// Approximate BSP map units per waypoint segment. Used to convert camera speed
/// (units/sec) into a normalized spline parameter increment per frame.
/// GoldSrc maps use Quake-style units; 256 units is a reasonable average inter-waypoint
/// distance for de_dust2 spawn/bombsite positions.
const MAP_UNIT_SCALE: f32 = 256.0;

/// Pose returned by Camera::update() each frame.
pub struct CameraPose {
    pub view: Mat4,
    pub eye: Vec3,
    /// Radians. atan2(forward.y, forward.x). JSON output should convert to degrees.
    pub yaw: f32,
    /// Radians. asin(forward.z.clamp(-1.0, 1.0)). JSON output should convert to degrees.
    pub pitch: f32,
}

pub struct Camera {
    waypoints: Vec<Vec3>,  // in the order given (caller is responsible for ordering)
    t: f32,                // spline parameter, 0.0..1.0 over full loop
    speed: f32,            // units/sec
    bob_amplitude: f32,
    bob_frequency: f32,
    start_time: Instant,
    first_update: bool,
}

impl Camera {
    /// Create a camera from pre-ordered waypoints.
    /// Returns Err if fewer than 4 waypoints are provided.
    pub fn new(waypoints: Vec<Vec3>, speed: f32, bob_amplitude: f32, bob_frequency: f32) -> anyhow::Result<Self> {
        anyhow::ensure!(waypoints.len() >= 4, "need at least 4 waypoints, got {}", waypoints.len());
        Ok(Self {
            waypoints,
            t: 0.0,
            speed,
            bob_amplitude,
            bob_frequency,
            start_time: Instant::now(),
            first_update: true,
        })
    }

    /// Advance the spline parameter and return a CameraPose.
    pub fn update(&mut self, delta_secs: f32) -> CameraPose {
        let n = self.waypoints.len() as f32;
        self.t = (self.t + self.speed * delta_secs / (n * MAP_UNIT_SCALE)) % 1.0;

        let pos = catmull_rom_position(&self.waypoints, self.t);
        // Asymmetric pitch clamp: floor at +2°, ceiling at +12°.
        // GoldSrc maps have void below the playable area. A small upward floor
        // prevents floor-level ledges from exposing void at the bottom of the frame,
        // and matches the screensaver aesthetic of looking slightly up into the map.
        let raw = catmull_rom_tangent(&self.waypoints, self.t);
        let len = raw.length();
        let min_z =  3.0_f32.to_radians().sin() * len;  // floor: always look slightly up
        let max_z = 12.0_f32.to_radians().sin() * len;  // ceiling: max upward
        let forward = Vec3::new(raw.x, raw.y, raw.z.clamp(min_z, max_z)).normalize_or_zero();

        if self.first_update {
            self.first_update = false;
            crate::diag!("[cs-flythrough] camera pos: {:?}  forward: {:?}  t={:.4}  waypoints: {}", pos, forward, self.t, self.waypoints.len());
        }
        let elapsed = self.start_time.elapsed().as_secs_f32();
        let bob = self.bob_amplitude * (elapsed * self.bob_frequency * std::f32::consts::TAU).sin();

        let eye = pos + Vec3::new(0.0, 0.0, 64.0 + bob);
        let target = eye + forward;
        let up = Vec3::Z;

        let view = Mat4::look_at_rh(eye, target, up);
        let yaw = forward.y.atan2(forward.x);
        let pitch = forward.z.clamp(-1.0, 1.0).asin();

        CameraPose { view, eye, yaw, pitch }
    }
}

/// Remove waypoints that are closer than `min_dist` to the previous one.
///
/// Dense clusters of small NAV areas (doorways, corridors) produce tight Catmull-Rom
/// hairpins that push the camera against wall geometry. Keeping only waypoints that are
/// at least `min_dist` apart leaves the path in open areas of the map where the spline
/// has room to interpolate cleanly.
///
/// The first waypoint is always kept. Returns all input points unchanged if every
/// consecutive pair exceeds `min_dist` already.
pub fn decimate_waypoints(pts: Vec<Vec3>, min_dist: f32) -> Vec<Vec3> {
    let mut result: Vec<Vec3> = Vec::new();
    for pt in pts {
        if result.last().map_or(true, |last: &Vec3| last.distance(pt) >= min_dist) {
            result.push(pt);
        }
    }
    result
}

/// Smooth waypoints by iteratively averaging each point with its neighbors.
///
/// Each iteration replaces every waypoint with the weighted average of its predecessor,
/// itself (weight 2), and its successor (weight 1 each), treating the list as a closed
/// loop. Multiple iterations progressively round sharp corners.
///
/// Use after decimation so the path shape is stable before smoothing.
pub fn smooth_waypoints(pts: Vec<Vec3>, iterations: u32) -> Vec<Vec3> {
    let n = pts.len();
    if n < 3 { return pts; }
    let mut current = pts;
    for _ in 0..iterations {
        let mut smoothed = Vec::with_capacity(n);
        for i in 0..n {
            let prev = current[(i + n - 1) % n];
            let curr = current[i];
            let next = current[(i + 1) % n];
            let avg = (prev + curr * 2.0 + next) / 4.0;
            // Only smooth XY — averaging Z moves waypoints outside the map's navigable
            // volume (e.g. entities at different heights pull neighbors into voids).
            smoothed.push(Vec3::new(avg.x, avg.y, curr.z));
        }
        current = smoothed;
    }
    current
}

/// Sort waypoints using nearest-neighbor starting from index 0.
/// Used by main.rs for entity-origin fallback paths.
pub fn nearest_neighbor_sort(mut pts: Vec<Vec3>) -> Vec<Vec3> {
    if pts.is_empty() { return pts; }
    // Start from the waypoint nearest to the centroid rather than index 0.
    // Index 0 is often a map-edge point; the centroid is deep inside the playable
    // volume surrounded by geometry on all sides.
    let centroid = pts.iter().copied().fold(Vec3::ZERO, |a, p| a + p) / pts.len() as f32;
    let start = pts.iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| centroid.distance_squared(**a)
            .partial_cmp(&centroid.distance_squared(**b)).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(0);
    let mut sorted = Vec::with_capacity(pts.len());
    sorted.push(pts.remove(start));
    while !pts.is_empty() {
        let last = *sorted.last().unwrap();
        let nearest = pts.iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                last.distance_squared(**a)
                    .partial_cmp(&last.distance_squared(**b))
                    .unwrap()
            })
            .map(|(i, _)| i)
            .unwrap();
        sorted.push(pts.remove(nearest));
    }
    sorted
}

/// Closed Catmull-Rom spline position at parameter t ∈ [0,1].
fn catmull_rom_position(pts: &[Vec3], t: f32) -> Vec3 {
    let n = pts.len();
    let scaled = t * n as f32;
    let i = scaled.floor() as usize;
    let local_t = scaled - i as f32;

    let p0 = pts[(i + n - 1) % n];
    let p1 = pts[i % n];
    let p2 = pts[(i + 1) % n];
    let p3 = pts[(i + 2) % n];

    catmull_rom(p0, p1, p2, p3, local_t)
}

/// Closed Catmull-Rom tangent at parameter t ∈ [0,1] (for forward direction).
fn catmull_rom_tangent(pts: &[Vec3], t: f32) -> Vec3 {
    let epsilon = 0.001_f32;
    let t1 = (t + epsilon) % 1.0;
    let t0 = (t - epsilon + 1.0) % 1.0;
    (catmull_rom_position(pts, t1) - catmull_rom_position(pts, t0)) / (2.0 * epsilon)
}

fn catmull_rom(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * (
        (2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn four_square_pts() -> Vec<Vec3> {
        vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1000.0, 0.0, 0.0),
            Vec3::new(1000.0, 1000.0, 0.0),
            Vec3::new(0.0, 1000.0, 0.0),
        ]
    }

    #[test]
    fn test_new_requires_four_points() {
        assert!(Camera::new(vec![], 133.0, 2.0, 2.0).is_err());
        assert!(Camera::new(vec![Vec3::ZERO; 3], 133.0, 2.0, 2.0).is_err());
        assert!(Camera::new(four_square_pts(), 133.0, 2.0, 2.0).is_ok());
    }

    #[test]
    fn test_smooth_waypoints_rounds_sharp_corner() {
        // Sharp V-shape: A, B (far away), A's neighbor
        let pts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1000.0, 0.0),  // far north
            Vec3::new(1000.0, 0.0, 0.0),  // east, making a hairpin
            Vec3::new(500.0, 0.0, 0.0),
        ];
        let smoothed = smooth_waypoints(pts.clone(), 5);
        assert_eq!(smoothed.len(), pts.len());
        // After smoothing, the far-north point should have moved closer to the centroid
        assert!(smoothed[1].y < pts[1].y, "north point should have moved south after smoothing");
    }

    #[test]
    fn test_smooth_waypoints_zero_iterations_unchanged() {
        let pts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(500.0, 0.0, 0.0),
            Vec3::new(500.0, 500.0, 0.0),
            Vec3::new(0.0, 500.0, 0.0),
        ];
        let result = smooth_waypoints(pts.clone(), 0);
        assert_eq!(result, pts);
    }

    #[test]
    fn test_decimate_waypoints_removes_close_points() {
        let pts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(50.0, 0.0, 0.0),   // 50 units — too close, removed
            Vec3::new(200.0, 0.0, 0.0),  // 200 from first kept point — kept
            Vec3::new(250.0, 0.0, 0.0),  // 50 from previous — removed
            Vec3::new(500.0, 0.0, 0.0),  // 300 from last kept — kept
        ];
        let result = decimate_waypoints(pts, 150.0);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(result[1], Vec3::new(200.0, 0.0, 0.0));
        assert_eq!(result[2], Vec3::new(500.0, 0.0, 0.0));
    }

    #[test]
    fn test_decimate_waypoints_keeps_all_when_spacing_large() {
        let pts = four_square_pts(); // 1000 units apart
        let result = decimate_waypoints(pts.clone(), 150.0);
        assert_eq!(result.len(), pts.len());
    }

    #[test]
    fn test_nearest_neighbor_sort_visits_all() {
        let pts = four_square_pts();
        let sorted = nearest_neighbor_sort(pts.clone());
        assert_eq!(sorted.len(), pts.len());
        // All original points should appear
        for p in &pts {
            assert!(sorted.contains(p));
        }
    }

    #[test]
    fn test_catmull_rom_at_t0_equals_p1() {
        let pts = four_square_pts();
        let pos = catmull_rom_position(&pts, 0.0);
        assert!((pos - pts[0]).length() < 0.01);
    }

    #[test]
    fn test_catmull_rom_loops_smoothly() {
        let pts = four_square_pts();
        let pos_start = catmull_rom_position(&pts, 0.0);
        let pos_near_end = catmull_rom_position(&pts, 0.999);
        // Near-end should be close to start for a closed spline
        assert!((pos_start - pos_near_end).length() < 50.0);
    }

    #[test]
    fn test_update_returns_pose_with_view() {
        let mut cam = Camera::new(four_square_pts(), 133.0, 2.0, 2.0).unwrap();
        let pose = cam.update(0.016);
        assert_ne!(pose.view, Mat4::IDENTITY);
    }

    #[test]
    fn test_update_pose_eye_is_above_waypoint() {
        // bob_amplitude=0 so no bob; eye should be exactly 64 units above waypoint Z
        let mut cam = Camera::new(four_square_pts(), 133.0, 0.0, 0.0).unwrap();
        let pose = cam.update(0.0);
        assert!(pose.eye.z > 60.0, "eye z={} expected >60", pose.eye.z);
    }

    #[test]
    fn test_update_pose_yaw_pitch_finite() {
        let mut cam = Camera::new(four_square_pts(), 133.0, 2.0, 2.0).unwrap();
        let pose = cam.update(0.016);
        assert!(pose.yaw.is_finite());
        assert!(pose.pitch.is_finite());
        assert!(pose.pitch.abs() <= std::f32::consts::FRAC_PI_2 + 1e-5);
    }
}
