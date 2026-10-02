//! A small `f64` vector for the host evaluator.

use std::ops::{Add, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[must_use]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub const fn splat(v: f64) -> Self {
        Self::new(v, v, v)
    }

    pub fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs(), self.z.abs())
    }

    pub fn normalize(self) -> Self {
        self * (1.0 / self.length())
    }

    /// Component-wise clamp to `[-limit, limit]`.
    pub fn clamp_sym(self, limit: f64) -> Self {
        Self::new(
            self.x.clamp(-limit, limit),
            self.y.clamp(-limit, limit),
            self.z.clamp(-limit, limit),
        )
    }

    pub fn min_component(self) -> f64 {
        self.x.min(self.y).min(self.z)
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Self;
    fn mul(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

/// A 3x3 rotation, row-major.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3 {
    pub rows: [Vec3; 3],
}

impl Mat3 {
    pub const IDENTITY: Self = Self {
        rows: [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ],
    };

    /// Rotation by Euler angles in radians, applied X, then Y, then Z. The
    /// shader builds the same matrix from the same three parameters.
    pub fn from_euler(rx: f64, ry: f64, rz: f64) -> Self {
        let (sx, cx) = rx.sin_cos();
        let (sy, cy) = ry.sin_cos();
        let (sz, cz) = rz.sin_cos();
        // Rz * Ry * Rx
        Self {
            rows: [
                Vec3::new(cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx),
                Vec3::new(sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx),
                Vec3::new(-sy, cy * sx, cy * cx),
            ],
        }
    }

    pub fn apply(&self, v: Vec3) -> Vec3 {
        Vec3::new(
            self.rows[0].dot(v),
            self.rows[1].dot(v),
            self.rows[2].dot(v),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-12
    }

    #[test]
    fn euler_rotation_preserves_length_and_composes_in_order() {
        let m = Mat3::from_euler(0.3, -1.1, 2.0);
        let v = Vec3::new(1.0, 2.0, -0.5);
        assert!((m.apply(v).length() - v.length()).abs() < 1e-12);
        let x_only = Mat3::from_euler(std::f64::consts::FRAC_PI_2, 0.0, 0.0);
        assert!(close(
            x_only.apply(Vec3::new(0.0, 1.0, 0.0)),
            Vec3::new(0.0, 0.0, 1.0)
        ));
        let z_only = Mat3::from_euler(0.0, 0.0, std::f64::consts::FRAC_PI_2);
        assert!(close(
            z_only.apply(Vec3::new(1.0, 0.0, 0.0)),
            Vec3::new(0.0, 1.0, 0.0)
        ));
    }
}
