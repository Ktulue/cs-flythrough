use glam::Vec3;

/// Minimal collision data extracted from qbsp's BspData.
pub struct CollisionData {
    pub planes: Vec<CollisionPlane>,
    pub nodes: Vec<CollisionNode>,
    pub leaves: Vec<LeafContents>,
}

#[derive(Clone, Copy)]
pub struct CollisionPlane {
    pub normal: Vec3,
    pub dist: f32,
}

#[derive(Clone, Copy)]
pub struct CollisionNode {
    pub plane_idx: u32,
    /// Positive = node index, negative = -(leaf_index + 1)
    pub front: i32,
    pub back: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LeafContents {
    Empty,
    Solid,
}

impl CollisionPlane {
    pub fn point_side(&self, point: Vec3) -> f32 {
        self.normal.dot(point) - self.dist
    }
}

pub fn point_in_solid(data: &CollisionData, point: Vec3) -> bool {
    let mut idx: i32 = 0;
    loop {
        if idx < 0 {
            let leaf_idx = (-idx - 1) as usize;
            return data.leaves.get(leaf_idx).copied() == Some(LeafContents::Solid);
        }
        let node = &data.nodes[idx as usize];
        let plane = &data.planes[node.plane_idx as usize];
        let dist = plane.point_side(point);
        idx = if dist >= 0.0 { node.front } else { node.back };
    }
}

/// Resolve camera movement from `from` to `to`, sliding along walls if `to` is solid.
pub fn resolve_position(data: &CollisionData, from: Vec3, to: Vec3) -> Vec3 {
    if !point_in_solid(data, to) {
        return to;
    }

    // If starting position is already solid, attempt push-out along axes.
    if point_in_solid(data, from) {
        eprintln!("[cs-flythrough] warning: camera started inside solid geometry, pushing out");
        for dist in [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0_f32] {
            for dir in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
                let candidate = from + dir * dist;
                if !point_in_solid(data, candidate) {
                    return candidate;
                }
            }
        }
        return from;
    }

    // Binary search to find contact point along from→to.
    let mut safe = from;
    let mut blocked = to;
    let mut contact_plane: Option<CollisionPlane> = None;

    for _ in 0..16 {
        let mid = (safe + blocked) * 0.5;
        if (mid - safe).length() < 0.5 {
            break;
        }
        if point_in_solid(data, mid) {
            blocked = mid;
            contact_plane = find_contact_plane(data, safe, mid);
        } else {
            safe = mid;
        }
    }

    // Slide along the contact plane.
    if let Some(plane) = contact_plane {
        let remaining = to - safe;
        let slide = remaining - plane.normal * remaining.dot(plane.normal);
        let slid_pos = safe + slide;
        if !point_in_solid(data, slid_pos) {
            return slid_pos;
        }
    }

    safe
}

fn find_contact_plane(data: &CollisionData, open: Vec3, solid: Vec3) -> Option<CollisionPlane> {
    let mut idx: i32 = 0;
    let mut last_plane: Option<CollisionPlane> = None;

    loop {
        if idx < 0 {
            return last_plane;
        }
        let node = &data.nodes[idx as usize];
        let plane = &data.planes[node.plane_idx as usize];
        let d_open = plane.point_side(open);
        let d_solid = plane.point_side(solid);

        if d_open >= 0.0 && d_solid < 0.0 {
            last_plane = Some(*plane);
        }

        idx = if d_solid >= 0.0 { node.front } else { node.back };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_bsp() -> CollisionData {
        CollisionData {
            planes: vec![CollisionPlane { normal: Vec3::X, dist: 0.0 }],
            nodes: vec![CollisionNode { plane_idx: 0, front: -1, back: -2 }],
            leaves: vec![LeafContents::Empty, LeafContents::Solid],
        }
    }

    #[test]
    fn test_point_in_solid_empty() {
        let bsp = simple_bsp();
        assert!(!point_in_solid(&bsp, Vec3::new(5.0, 0.0, 0.0)));
    }

    #[test]
    fn test_point_in_solid_solid() {
        let bsp = simple_bsp();
        assert!(point_in_solid(&bsp, Vec3::new(-5.0, 0.0, 0.0)));
    }

    #[test]
    fn test_resolve_open_movement() {
        let bsp = simple_bsp();
        let from = Vec3::new(5.0, 0.0, 0.0);
        let to = Vec3::new(10.0, 0.0, 0.0);
        let result = resolve_position(&bsp, from, to);
        assert_eq!(result, to);
    }

    #[test]
    fn test_resolve_into_wall() {
        let bsp = simple_bsp();
        let from = Vec3::new(5.0, 0.0, 0.0);
        let to = Vec3::new(-5.0, 0.0, 0.0);
        let result = resolve_position(&bsp, from, to);
        assert!(!point_in_solid(&bsp, result));
        assert!(result.x >= -0.5, "result.x={} should be >= -0.5", result.x);
        assert!(result.x <= 1.0, "result.x={} should be near the wall", result.x);
    }

    #[test]
    fn test_resolve_from_inside_solid() {
        let bsp = simple_bsp();
        let from = Vec3::new(-5.0, 0.0, 0.0);
        let to = Vec3::new(-10.0, 0.0, 0.0);
        let result = resolve_position(&bsp, from, to);
        // Should attempt push-out along axes
        assert!(!point_in_solid(&bsp, result) || result == from);
    }
}
