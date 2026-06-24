//! Shared math primitives for the simulation.
//!
//! [`Vec2`] is the one vector type used throughout the domain — prefer it over
//! bare `(f64, f64)` tuples so operations stay consistent.

use serde::{Deserialize, Serialize};

/// A 2D vector / point in world space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn add(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x + other.x, self.y + other.y)
    }

    pub fn sub(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x - other.x, self.y - other.y)
    }

    pub fn scale(self, factor: f64) -> Vec2 {
        Vec2::new(self.x * factor, self.y * factor)
    }

    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Returns a unit-length vector in the same direction, or [`Vec2::ZERO`]
    /// if this vector has (near) zero length.
    pub fn normalized(self) -> Vec2 {
        let len = self.length();
        if len <= f64::EPSILON {
            Vec2::ZERO
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
        let a = Vec2::new(1.0, 2.0);
        let b = Vec2::new(3.0, 4.0);
        assert_eq!(a.add(b), Vec2::new(4.0, 6.0));
        assert_eq!(b.sub(a), Vec2::new(2.0, 2.0));
        assert_eq!(a.scale(2.0), Vec2::new(2.0, 4.0));
    }

    #[test]
    fn length_and_normalize() {
        let v = Vec2::new(3.0, 4.0);
        assert_eq!(v.length(), 5.0);
        let n = v.normalized();
        assert!((n.length() - 1.0).abs() < 1e-9);
        assert_eq!(Vec2::ZERO.normalized(), Vec2::ZERO);
    }
}
