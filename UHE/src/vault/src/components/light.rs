//! Light sources.
//!
//! Three types rather than one with a `kind` field, because a directional light
//! has no position and no range while the other two have both, and the unused
//! fields of a tagged struct are always someone's bug.

use serde::{Deserialize, Serialize};

/// A directional light: infinitely far away, so position does not matter.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DirectionalLight {
    /// Direction the light travels, normalized on load.
    /// Direction the light travels. Normalized on load.
    pub direction: [f32; 3],
    /// Linear RGB.
    pub color: [f32; 3],
    /// Brightness multiplier.
    pub intensity: f32,
    /// 0 = no shadow, 1 = fully black shadow.
    pub shadow_strength: f32,
}

impl Default for DirectionalLight {
    /// A white sun pointing straight down, so a freshly added light is visible
    /// rather than invisible. `direction` is negated in the shader, so this
    /// points down the world -Y axis.
    fn default() -> Self {
        Self {
            direction: [0.0, -1.0, 0.0],
            color: [1.0, 0.95, 0.9],
            intensity: 1.0,
            shadow_strength: 1.0,
        }
    }
}

/// A point light.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct PointLight {
    /// Linear RGB.
    pub color: [f32; 3],
    /// Brightness multiplier.
    pub intensity: f32,
    /// 0 for a light of infinite range.
    pub radius: f32,
}

/// A spot light: a point light with a cone.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct SpotLight {
    /// Linear RGB.
    pub color: [f32; 3],
    /// Brightness multiplier.
    pub intensity: f32,
    /// Half-angle of the fully-lit cone, in radians.
    /// Half-angle of the fully-lit cone, in radians.
    pub inner_angle: f32,
    /// Half-angle of the falloff edge, in radians.
    /// Half-angle of the falloff edge, in radians.
    pub outer_angle: f32,
    /// Range; 0 for infinite.
    pub radius: f32,
}

impl Default for SpotLight {
    fn default() -> Self {
        Self {
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            inner_angle: 0.3,
            outer_angle: 0.6,
            radius: 0.0,
        }
    }
}
