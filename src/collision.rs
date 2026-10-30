//! Continuous axis-separated collision against block shape boxes.
//! Shapes are supplied by the caller; render meshes are not collision geometry.
use glam::DVec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}
impl Aabb {
    pub fn new(min: DVec3, max: DVec3) -> Result<Self, String> {
        if !min.is_finite() || !max.is_finite() || !min.cmplt(max).all() {
            return Err("collision box must have finite, positive extents".into());
        }
        Ok(Self { min, max })
    }
    pub fn translated(self, delta: DVec3) -> Self {
        Self {
            min: self.min + delta,
            max: self.max + delta,
        }
    }
    pub fn intersects(self, other: Self) -> bool {
        self.min.cmplt(other.max).all() && self.max.cmpgt(other.min).all()
    }
    /// Region the broad phase must query, including a possible step.
    pub fn swept_bounds(self, delta: DVec3, step_height: f64) -> Self {
        Self {
            min: self.min.min(self.min + delta),
            max: self.max.max(self.max + delta) + DVec3::Y * step_height,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub delta: DVec3,
    pub grounded: bool,
    pub blocked: [bool; 3],
}

fn clip_axis(body: Aabb, obstacles: &[Aabb], axis: usize, mut amount: f64) -> f64 {
    for obstacle in obstacles {
        if (0..3)
            .filter(|&i| i != axis)
            .any(|i| body.max[i] <= obstacle.min[i] || body.min[i] >= obstacle.max[i])
        {
            continue;
        }
        if amount > 0.0 && body.max[axis] <= obstacle.min[axis] {
            amount = amount.min(obstacle.min[axis] - body.max[axis]);
        } else if amount < 0.0 && body.min[axis] >= obstacle.max[axis] {
            amount = amount.max(obstacle.max[axis] - body.min[axis]);
        }
    }
    amount
}
fn sweep(mut body: Aabb, obstacles: &[Aabb], desired: DVec3) -> DVec3 {
    let mut result = DVec3::ZERO;
    let horizontal = if desired.x.abs() >= desired.z.abs() {
        [0, 2]
    } else {
        [2, 0]
    };
    for axis in [1, horizontal[0], horizontal[1]] {
        result[axis] = clip_axis(body, obstacles, axis, desired[axis]);
        let mut offset = DVec3::ZERO;
        offset[axis] = result[axis];
        body = body.translated(offset);
    }
    result
}

/// Resolve a finite displacement without tunneling through supplied boxes.
/// The caller must supply every collider in `body.swept_bounds(desired, step_height)`.
/// Starting penetrations are rejected; depenetration and moving colliders are separate concerns.
pub fn move_body(
    body: Aabb,
    obstacles: &[Aabb],
    desired: DVec3,
    step_height: f64,
    was_grounded: bool,
) -> Result<Motion, String> {
    Aabb::new(body.min, body.max)?;
    if !desired.is_finite() || !step_height.is_finite() || !(0.0..=1.0).contains(&step_height) {
        return Err("invalid movement or step height".into());
    }
    for obstacle in obstacles {
        Aabb::new(obstacle.min, obstacle.max)?;
        if body.intersects(*obstacle) {
            return Err("body starts inside a collider".into());
        }
    }
    let mut delta = sweep(body, obstacles, desired);
    let landed = desired.y < 0.0 && delta.y != desired.y;
    if step_height > 0.0
        && desired.y <= 0.0
        && (was_grounded || landed)
        && (delta.x != desired.x || delta.z != desired.z)
    {
        let up = sweep(body, obstacles, DVec3::Y * step_height);
        let horizontal = sweep(
            body.translated(up),
            obstacles,
            DVec3::new(desired.x, 0.0, desired.z),
        );
        let down = sweep(
            body.translated(up + horizontal),
            obstacles,
            DVec3::Y * (desired.y - up.y),
        );
        let stepped = up + horizontal + down;
        if stepped.x * stepped.x + stepped.z * stepped.z > delta.x * delta.x + delta.z * delta.z {
            delta = stepped;
        }
    }
    Ok(Motion {
        delta,
        grounded: desired.y < 0.0 && delta.y > desired.y,
        blocked: [
            delta.x != desired.x,
            delta.y != desired.y,
            delta.z != desired.z,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cube(min: [f64; 3], max: [f64; 3]) -> Aabb {
        Aabb::new(min.into(), max.into()).unwrap()
    }
    fn player() -> Aabb {
        cube([0.0, 0.0, 0.0], [0.6, 1.8, 0.6])
    }
    #[test]
    fn fast_motion_cannot_tunnel() {
        let wall = cube([2.0, -5.0, -5.0], [2.1, 5.0, 5.0]);
        let motion = move_body(player(), &[wall], DVec3::X * 100.0, 0.0, false).unwrap();
        assert_eq!(motion.delta.x, 1.4);
        assert!(!player().translated(motion.delta).intersects(wall));
    }
    #[test]
    fn slides_along_wall() {
        let wall = cube([2.0, -5.0, -5.0], [3.0, 5.0, 5.0]);
        let motion = move_body(player(), &[wall], DVec3::new(3.0, 0.0, 2.0), 0.0, false).unwrap();
        assert_eq!(motion.delta, DVec3::new(1.4, 0.0, 2.0));
    }
    #[test]
    fn lands_on_slab_and_steps_up() {
        let slab = cube([1.0, 0.0, -1.0], [3.0, 0.5, 2.0]);
        let floor = cube([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0]);
        let motion = move_body(
            player(),
            &[floor, slab],
            DVec3::new(1.2, -0.1, 0.0),
            0.6,
            true,
        )
        .unwrap();
        assert_eq!(motion.delta.x, 1.2);
        assert!((motion.delta.y - 0.5).abs() < 1e-12);
        assert!(motion.grounded);
        let falling = player().translated(DVec3::new(1.2, 10.0, 0.0));
        let motion = move_body(falling, &[slab], -DVec3::Y * 20.0, 0.0, false).unwrap();
        assert_eq!(motion.delta.y, -9.5);
        assert!(motion.grounded);
    }
    #[test]
    fn low_ceiling_prevents_step() {
        let slab = cube([1.0, 0.0, -1.0], [3.0, 0.5, 2.0]);
        let ceiling = cube([-1.0, 2.0, -1.0], [4.0, 3.0, 2.0]);
        let result = move_body(
            player(),
            &[slab, ceiling],
            DVec3::new(1.2, -0.1, 0.0),
            0.6,
            true,
        )
        .unwrap();
        assert_eq!(result.delta.x, 0.4);
    }
    #[test]
    fn rejects_invalid_and_penetrating_inputs() {
        assert!(move_body(player(), &[player()], DVec3::ZERO, 0.0, false).is_err());
        assert!(move_body(player(), &[], DVec3::splat(f64::NAN), 0.0, false).is_err());
        assert!(Aabb::new(DVec3::ZERO, DVec3::ZERO).is_err());
    }
    #[test]
    fn negative_coordinates_and_reverse_motion() {
        let body = player().translated(DVec3::splat(-10.0));
        let wall = cube([-12.0, -20.0, -20.0], [-11.0, 0.0, 0.0]);
        let motion = move_body(body, &[wall], -DVec3::X * 100.0, 0.0, false).unwrap();
        assert_eq!(motion.delta.x, -1.0);
    }
}
