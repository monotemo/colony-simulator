//! Shared math primitives for the simulation.
//!
//! [`Vec3`] is the one vector type used throughout the domain — prefer it over
//! bare `(f64, f64, f64)` tuples so operations stay consistent. The simulation
//! is dimensionally 3D; the `z` axis is currently kept at `0.0` everywhere
//! (bees, resources, spawns) until flight behavior and depth rendering land,
//! but the geometry, bounds, and integration loop all carry it for real so
//! enabling the third dimension later is behavior work, not a wire-format break.

use serde::{Deserialize, Serialize};

/// A 3D vector / point in world space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn add(self, other: Vec3) -> Vec3 {
        Vec3::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    pub fn sub(self, other: Vec3) -> Vec3 {
        Vec3::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    pub fn scale(self, factor: f64) -> Vec3 {
        Vec3::new(self.x * factor, self.y * factor, self.z * factor)
    }

    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    /// Returns a unit-length vector in the same direction, or [`Vec3::ZERO`]
    /// if this vector has (near) zero length.
    pub fn normalized(self) -> Vec3 {
        let len = self.length();
        if len <= f64::EPSILON {
            Vec3::ZERO
        } else {
            self.scale(1.0 / len)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sub_scale() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(3.0, 4.0, 5.0);
        assert_eq!(a.add(b), Vec3::new(4.0, 6.0, 8.0));
        assert_eq!(b.sub(a), Vec3::new(2.0, 2.0, 2.0));
        assert_eq!(a.scale(2.0), Vec3::new(2.0, 4.0, 6.0));
    }

    #[test]
    fn length_and_normalize() {
        // 2D triple still resolves with z = 0.
        let v = Vec3::new(3.0, 4.0, 0.0);
        assert_eq!(v.length(), 5.0);
        // Full 3D Pythagorean triple (1, 2, 2) -> 3.
        let w = Vec3::new(1.0, 2.0, 2.0);
        assert_eq!(w.length(), 3.0);
        let n = w.normalized();
        assert!((n.length() - 1.0).abs() < 1e-9);
        assert_eq!(Vec3::ZERO.normalized(), Vec3::ZERO);
    }
}
