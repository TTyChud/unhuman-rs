//! Lights, in the layout the shader reads.
//!
//! Port of `UHE/Renderer3D/LightSystem.{h,cpp}`. The C++ walked three entity
//! views and a glTF node list itself; `meats::Scene::lights` now produces the
//! same values already placed in world space, so this module keeps the *GPU*
//! half: the four-`vec4` record, and the per-frame buffer it lives in.

use bytemuck::{Pod, Zeroable};
use chassis::{Buffer, BufferDesc, BufferUsage, Resources};
use glam::Vec3;

/// One light in the GPU list. Byte-identical to `RD3d::LightData` and to the
/// `LightData` struct in `Basic3D.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LightData {
    /// `x` = type (0 directional, 1 point, 2 spot), `y` = radius, `z`/`w` =
    /// cos(inner)/cos(outer) cone angles. Cosines so the fragment shader compares
    /// dots instead of taking two inverse cosines per light per pixel.
    pub type_radius_cones: [f32; 4],
    /// Position for point and spot (`w` = 1), direction for a directional light
    /// (`w` = 0).
    pub position_or_direction: [f32; 4],
    /// A spot's forward axis; unused otherwise.
    pub direction_pad: [f32; 4],
    /// `rgb` colour and intensity.
    pub color_intensity: [f32; 4],
}

pub const TYPE_DIRECTIONAL: f32 = 0.0;
pub const TYPE_POINT: f32 = 1.0;
pub const TYPE_SPOT: f32 = 2.0;

/// The C++ caps the list at 1024, both in the buffer size and in the draw's
/// `numLights`.
pub const MAX_LIGHTS: usize = 1024;

impl LightData {
    pub fn directional(direction: Vec3, color: Vec3, intensity: f32) -> Self {
        Self {
            type_radius_cones: [TYPE_DIRECTIONAL, 0.0, 0.0, 0.0],
            position_or_direction: [direction.x, direction.y, direction.z, 0.0],
            direction_pad: [0.0; 4],
            color_intensity: [color.x, color.y, color.z, intensity],
        }
    }

    pub fn point(position: Vec3, color: Vec3, intensity: f32, radius: f32) -> Self {
        Self {
            type_radius_cones: [TYPE_POINT, radius, 0.0, 0.0],
            position_or_direction: [position.x, position.y, position.z, 1.0],
            direction_pad: [0.0; 4],
            color_intensity: [color.x, color.y, color.z, intensity],
        }
    }

    /// Cone angles in radius, stored as cosines. Normalized direction, as the
    /// C++ does: the shader's cone test is a dot product against a unit axis.
    pub fn spot(
        position: Vec3,
        direction: Vec3,
        color: Vec3,
        intensity: f32,
        radius: f32,
        inner_angle: f32,
        outer_angle: f32,
    ) -> Self {
        let axis = direction.normalize_or_zero();
        Self {
            type_radius_cones: [TYPE_SPOT, radius, inner_angle.cos(), outer_angle.cos()],
            position_or_direction: [position.x, position.y, position.z, 1.0],
            direction_pad: [axis.x, axis.y, axis.z, 0.0],
            color_intensity: [color.x, color.y, color.z, intensity],
        }
    }

    /// `meats::LightSample` carries degrees for the cone angles, as the editor
    /// edits them; the C++ component carried radians. Converted here, once, so
    /// no shader has to.
    pub fn from_sample(sample: &meats::LightSample) -> Self {
        match sample.kind {
            meats::LightKind::Directional => {
                Self::directional(sample.direction, sample.color, sample.intensity)
            }
            meats::LightKind::Point => Self::point(
                sample.position,
                sample.color,
                sample.intensity,
                sample.radius,
            ),
            meats::LightKind::Spot => Self::spot(
                sample.position,
                sample.direction,
                sample.color,
                sample.intensity,
                sample.radius,
                sample.inner_cone_angle.to_radians(),
                sample.outer_cone_angle.to_radians(),
            ),
        }
    }

    pub fn light_type(&self) -> f32 {
        self.type_radius_cones[0]
    }
}

/// The frame's light list, and the buffer it is uploaded into.
///
/// `Resources` has no update-a-buffer call — `create_buffer` takes initial
/// contents and that is all — so the buffer is re-created when the list
/// *changes*, and left alone when it does not. The C++ re-uploaded into a
/// persistent host-visible buffer every `BeginScene`; a static scene pays
/// nothing here, and a scene whose lights move pays one small allocation.
pub struct LightBuffer {
    buffer: Option<Buffer>,
    uploaded: Vec<LightData>,
}

impl LightBuffer {
    pub fn new() -> Self {
        Self {
            buffer: None,
            uploaded: Vec::new(),
        }
    }

    /// The buffer holding `lights`, re-created only if it does not hold them
    /// already. Truncated to `MAX_LIGHTS`, as the C++ truncates `numLights`.
    pub fn ensure(
        &mut self,
        resources: &mut dyn Resources,
        lights: &[LightData],
    ) -> Result<(Buffer, usize), chassis::Error> {
        let lights = &lights[..lights.len().min(MAX_LIGHTS)];

        if self.buffer.is_none() || self.uploaded != lights {
            if let Some(buffer) = self.buffer.take() {
                resources.destroy_buffer(buffer);
            }
            // Never zero-sized: a buffer of no lights is still a bound buffer,
            // and a backend given a zero-length one has nothing to bind.
            let bytes = if lights.is_empty() {
                bytemuck::bytes_of(&[0u8; 16])
            } else {
                bytemuck::cast_slice(lights)
            };
            let desc = BufferDesc::new(bytes.len() as u64, BufferUsage::STORAGE);
            self.buffer = Some(resources.create_buffer(desc, Some(bytes))?);
            self.uploaded = lights.to_vec();
        }

        Ok((self.buffer.expect("just created"), self.uploaded.len()))
    }

    pub fn destroy(&mut self, resources: &mut dyn Resources) {
        if let Some(buffer) = self.buffer.take() {
            resources.destroy_buffer(buffer);
        }
        self.uploaded.clear();
    }

    pub fn len(&self) -> usize {
        self.uploaded.len()
    }

    pub fn is_empty(&self) -> bool {
        self.uploaded.is_empty()
    }
}

impl Default for LightBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Every enabled light in the scene, in the GPU layout.
///
/// The C++ `LightSystem::ExtractLights` also ends with a default directional
/// light when nothing was found, so that an unlit scene is lit rather than
/// black; that is kept, and is why this never returns an empty list.
pub fn extract(scene: &meats::Scene) -> Vec<LightData> {
    let mut lights: Vec<LightData> = scene
        .lights()
        .iter()
        .map(LightData::from_sample)
        .collect();

    if lights.is_empty() {
        lights.push(LightData {
            type_radius_cones: [TYPE_DIRECTIONAL, 0.0, 0.0, 0.0],
            position_or_direction: [0.5, -1.0, 0.3, 0.0],
            direction_pad: [0.0; 4],
            color_intensity: [1.0, 1.0, 1.0, 1.0],
        });
    }

    lights.truncate(MAX_LIGHTS);
    lights
}
