//! The properties panel's model: a `Vec<Property>` per selected entity, built
//! by hand.
//!
//! The C++ walked ImGui's `DrawComponents<T>` template once per component type
//! and read the fields directly. A `dyn Any`/reflection layer would cost a type
//! lookup per property per frame, so each property is a getter and a setter
//! closure over the concrete component instead, and the panel draws whatever
//! the list says.
//!
//! Nothing here constructs a `vault::AssetId`: the registry issues those. An
//! asset-backed component exposes its `*_path` as the editable property and
//! keeps the handle as state the scene owns.

use glam::{Vec2, Vec3, Vec4};
use meats::{
    Animator, AudioSource, Camera, Collider, ColliderShape, Entity, Light, LightKind, MeshRenderer,
    ProjectionType, RigidBody, RigidBodyKind, Scene, SpriteRenderer, Tag, Text, Transform,
};

#[derive(Clone, PartialEq, Debug)]
pub enum PropertyValue {
    Bool(bool),
    Int(i32),
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Color([f32; 4]),
    Text(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PropertyKind {
    Bool,
    Int,
    Float,
    Vec2,
    Vec3,
    Color,
    Text,
    /// An index into the named options, edited as a combo.
    Enum(&'static [&'static str]),
}

pub struct Property {
    pub name: &'static str,
    pub component: &'static str,
    pub kind: PropertyKind,
    pub get: Box<dyn Fn(&Scene, Entity) -> PropertyValue + Send + Sync>,
    pub set: Box<dyn Fn(&mut Scene, Entity, &PropertyValue) + Send + Sync>,
}

impl Property {
    pub fn value(&self, scene: &Scene, entity: Entity) -> PropertyValue {
        (self.get)(scene, entity)
    }

    /// A stable key for a property, for queueing an edit before it is applied.
    pub fn key(&self) -> (&'static str, &'static str) {
        (self.component, self.name)
    }
}

fn boolean(
    name: &'static str,
    component: &'static str,
    get: impl Fn(&Scene, Entity) -> bool + Send + Sync + 'static,
    set: impl Fn(&mut Scene, Entity, bool) + Send + Sync + 'static,
) -> Property {
    Property {
        name,
        component,
        kind: PropertyKind::Bool,
        get: Box::new(move |scene, entity| PropertyValue::Bool(get(scene, entity))),
        set: Box::new(move |scene, entity, value| {
            if let PropertyValue::Bool(value) = value {
                set(scene, entity, *value);
            }
        }),
    }
}

fn scalar(
    name: &'static str,
    component: &'static str,
    get: impl Fn(&Scene, Entity) -> f32 + Send + Sync + 'static,
    set: impl Fn(&mut Scene, Entity, f32) + Send + Sync + 'static,
) -> Property {
    Property {
        name,
        component,
        kind: PropertyKind::Float,
        get: Box::new(move |scene, entity| PropertyValue::Float(get(scene, entity))),
        set: Box::new(move |scene, entity, value| {
            if let PropertyValue::Float(value) = value {
                set(scene, entity, *value);
            }
        }),
    }
}

fn vector2(
    name: &'static str,
    component: &'static str,
    get: impl Fn(&Scene, Entity) -> [f32; 2] + Send + Sync + 'static,
    set: impl Fn(&mut Scene, Entity, [f32; 2]) + Send + Sync + 'static,
) -> Property {
    Property {
        name,
        component,
        kind: PropertyKind::Vec2,
        get: Box::new(move |scene, entity| PropertyValue::Vec2(get(scene, entity))),
        set: Box::new(move |scene, entity, value| {
            if let PropertyValue::Vec2(value) = value {
                set(scene, entity, *value);
            }
        }),
    }
}

fn vector3(
    name: &'static str,
    component: &'static str,
    get: impl Fn(&Scene, Entity) -> [f32; 3] + Send + Sync + 'static,
    set: impl Fn(&mut Scene, Entity, [f32; 3]) + Send + Sync + 'static,
) -> Property {
    Property {
        name,
        component,
        kind: PropertyKind::Vec3,
        get: Box::new(move |scene, entity| PropertyValue::Vec3(get(scene, entity))),
        set: Box::new(move |scene, entity, value| {
            if let PropertyValue::Vec3(value) = value {
                set(scene, entity, *value);
            }
        }),
    }
}

fn color(
    name: &'static str,
    component: &'static str,
    get: impl Fn(&Scene, Entity) -> [f32; 4] + Send + Sync + 'static,
    set: impl Fn(&mut Scene, Entity, [f32; 4]) + Send + Sync + 'static,
) -> Property {
    Property {
        name,
        component,
        kind: PropertyKind::Color,
        get: Box::new(move |scene, entity| PropertyValue::Color(get(scene, entity))),
        set: Box::new(move |scene, entity, value| {
            if let PropertyValue::Color(value) = value {
                set(scene, entity, *value);
            }
        }),
    }
}

fn text(
    name: &'static str,
    component: &'static str,
    get: impl Fn(&Scene, Entity) -> String + Send + Sync + 'static,
    set: impl Fn(&mut Scene, Entity, &str) + Send + Sync + 'static,
) -> Property {
    Property {
        name,
        component,
        kind: PropertyKind::Text,
        get: Box::new(move |scene, entity| PropertyValue::Text(get(scene, entity))),
        set: Box::new(move |scene, entity, value| {
            if let PropertyValue::Text(value) = value {
                set(scene, entity, value);
            }
        }),
    }
}

fn enumeration(
    name: &'static str,
    component: &'static str,
    options: &'static [&'static str],
    get: impl Fn(&Scene, Entity) -> i32 + Send + Sync + 'static,
    set: impl Fn(&mut Scene, Entity, i32) + Send + Sync + 'static,
) -> Property {
    Property {
        name,
        component,
        kind: PropertyKind::Enum(options),
        get: Box::new(move |scene, entity| PropertyValue::Int(get(scene, entity))),
        set: Box::new(move |scene, entity, value| {
            if let PropertyValue::Int(value) = value {
                set(scene, entity, *value);
            }
        }),
    }
}

/// Applies an edit to one component, doing nothing when the entity has gone.
fn edit<T: Send + Sync + 'static>(scene: &mut Scene, entity: Entity, change: impl FnOnce(&mut T)) {
    if let Some(mut entry) = scene.get_mut(entity) {
        if let Some(value) = entry.get_mut::<T>() {
            change(value);
        }
    }
}

#[derive(Default)]
pub struct Inspector {
    dirty: Vec<Entity>,
}

impl Inspector {
    pub fn new() -> Self {
        Self::default()
    }

    /// The components the entity has, in the panel's order.
    pub fn properties(&self, scene: &Scene, entity: Entity) -> Vec<Property> {
        let Some(entry) = scene.get(entity) else {
            return Vec::new();
        };
        let mut properties = Vec::new();
        if entry.has::<Tag>() {
            properties.extend(tag_properties());
        }
        if entry.has::<Transform>() {
            properties.extend(transform_properties());
        }
        if entry.has::<Camera>() {
            properties.extend(camera_properties());
        }
        if entry.has::<MeshRenderer>() {
            properties.extend(mesh_renderer_properties());
        }
        if entry.has::<SpriteRenderer>() {
            properties.extend(sprite_renderer_properties());
        }
        if entry.has::<Text>() {
            properties.extend(text_properties());
        }
        if entry.has::<Light>() {
            properties.extend(light_properties());
        }
        if entry.has::<RigidBody>() {
            properties.extend(rigid_body_properties());
        }
        if entry.has::<Collider>() {
            properties.extend(collider_properties());
        }
        if entry.has::<AudioSource>() {
            properties.extend(audio_source_properties());
        }
        if entry.has::<Animator>() {
            properties.extend(animator_properties());
        }
        properties
    }

    /// Writes one property and marks the entity dirty, so the renderer and the
    /// physics backend can see that their copy of the scene is stale.
    pub fn set(&mut self, scene: &mut Scene, entity: Entity, property: &Property, value: PropertyValue) {
        (property.set)(scene, entity, &value);
        self.mark_dirty(entity);
    }

    /// Writes a property named by component and field, which is how an undone
    /// edit is replayed without holding on to a closure.
    pub fn set_named(
        &mut self,
        scene: &mut Scene,
        entity: Entity,
        component: &str,
        property: &str,
        value: &PropertyValue,
    ) -> bool {
        let found = self
            .properties(scene, entity)
            .into_iter()
            .find(|candidate| candidate.component == component && candidate.name == property);
        match found {
            Some(property) => {
                (property.set)(scene, entity, value);
                self.mark_dirty(entity);
                true
            }
            None => false,
        }
    }

    pub fn mark_dirty(&mut self, entity: Entity) {
        if !self.dirty.contains(&entity) {
            self.dirty.push(entity);
        }
    }

    pub fn dirty(&self) -> &[Entity] {
        &self.dirty
    }

    pub fn is_dirty(&self, entity: Entity) -> bool {
        self.dirty.contains(&entity)
    }

    /// Hands the edited entities to whoever reacts to them; the caller is
    /// responsible for what the change needs (transforms, a mesh rebuild).
    pub fn take_dirty(&mut self) -> Vec<Entity> {
        std::mem::take(&mut self.dirty)
    }

    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }
}

fn tag_properties() -> Vec<Property> {
    vec![text(
        "Tag",
        "Tag",
        |scene, entity| {
            scene
                .get(entity)
                .and_then(|entry| entry.get::<Tag>().map(|tag| tag.tag.clone()))
                .unwrap_or_default()
        },
        |scene, entity, value| edit::<Tag>(scene, entity, |tag| tag.tag = value.to_string()),
    )]
}

fn transform_properties() -> Vec<Property> {
    vec![
        vector3(
            "Translation",
            "Transform",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Transform>().map(|t| t.translation))
                    .unwrap_or(Vec3::ZERO)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<Transform>(scene, entity, |transform| {
                    transform.translation = Vec3::from_array(value)
                })
            },
        ),
        vector3(
            "Rotation",
            "Transform",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Transform>().map(|t| t.rotation))
                    .unwrap_or(Vec3::ZERO)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<Transform>(scene, entity, |transform| {
                    transform.rotation = Vec3::from_array(value)
                })
            },
        ),
        vector3(
            "Scale",
            "Transform",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Transform>().map(|t| t.scale))
                    .unwrap_or(Vec3::ONE)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<Transform>(scene, entity, |transform| {
                    transform.scale = Vec3::from_array(value)
                })
            },
        ),
    ]
}

fn camera_properties() -> Vec<Property> {
    vec![
        boolean(
            "Primary",
            "Camera",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Camera>().map(|camera| camera.primary))
                    .unwrap_or(false)
            },
            |scene, entity, value| edit::<Camera>(scene, entity, |camera| camera.primary = value),
        ),
        boolean(
            "Fixed Aspect Ratio",
            "Camera",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<Camera>()
                            .map(|camera| camera.fixed_aspect_ratio)
                    })
                    .unwrap_or(false)
            },
            |scene, entity, value| {
                edit::<Camera>(scene, entity, |camera| camera.fixed_aspect_ratio = value)
            },
        ),
        enumeration(
            "Projection",
            "Camera",
            PROJECTIONS,
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry.get::<Camera>().map(|camera| {
                            match camera.camera.projection_type() {
                                ProjectionType::Perspective => 0,
                                ProjectionType::Orthographic => 1,
                            }
                        })
                    })
                    .unwrap_or(0)
            },
            |scene, entity, value| {
                edit::<Camera>(scene, entity, |camera| {
                    camera.camera.set_projection_type(if value == 0 {
                        ProjectionType::Perspective
                    } else {
                        ProjectionType::Orthographic
                    });
                })
            },
        ),
        scalar(
            "FOV",
            "Camera",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Camera>().map(|camera| camera.camera.fov()))
                    .unwrap_or(std::f32::consts::FRAC_PI_4)
                    .to_degrees()
            },
            |scene, entity, value| {
                edit::<Camera>(scene, entity, |camera| {
                    let near = camera.camera.perspective_near();
                    let far = camera.camera.perspective_far();
                    camera.camera.set_perspective(value.to_radians(), near, far);
                })
            },
        ),
        scalar(
            "Perspective Near",
            "Camera",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<Camera>()
                            .map(|camera| camera.camera.perspective_near())
                    })
                    .unwrap_or(0.01)
            },
            |scene, entity, value| {
                edit::<Camera>(scene, entity, |camera| {
                    let fov = camera.camera.fov();
                    let far = camera.camera.perspective_far();
                    camera.camera.set_perspective(fov, value, far);
                })
            },
        ),
        scalar(
            "Perspective Far",
            "Camera",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<Camera>()
                            .map(|camera| camera.camera.perspective_far())
                    })
                    .unwrap_or(1000.0)
            },
            |scene, entity, value| {
                edit::<Camera>(scene, entity, |camera| {
                    let fov = camera.camera.fov();
                    let near = camera.camera.perspective_near();
                    camera.camera.set_perspective(fov, near, value);
                })
            },
        ),
        scalar(
            "Orthographic Size",
            "Camera",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<Camera>()
                            .map(|camera| camera.camera.orthographic_size())
                    })
                    .unwrap_or(10.0)
            },
            |scene, entity, value| {
                edit::<Camera>(scene, entity, |camera| {
                    let near = camera.camera.orthographic_near();
                    let far = camera.camera.orthographic_far();
                    camera.camera.set_orthographic(value, near, far);
                })
            },
        ),
        scalar(
            "Orthographic Near",
            "Camera",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<Camera>()
                            .map(|camera| camera.camera.orthographic_near())
                    })
                    .unwrap_or(-1.0)
            },
            |scene, entity, value| {
                edit::<Camera>(scene, entity, |camera| {
                    let size = camera.camera.orthographic_size();
                    let far = camera.camera.orthographic_far();
                    camera.camera.set_orthographic(size, value, far);
                })
            },
        ),
        scalar(
            "Orthographic Far",
            "Camera",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<Camera>()
                            .map(|camera| camera.camera.orthographic_far())
                    })
                    .unwrap_or(1.0)
            },
            |scene, entity, value| {
                edit::<Camera>(scene, entity, |camera| {
                    let size = camera.camera.orthographic_size();
                    let near = camera.camera.orthographic_near();
                    camera.camera.set_orthographic(size, near, value);
                })
            },
        ),
    ]
}

const PROJECTIONS: &[&str] = &["Perspective", "Orthographic"];

fn mesh_renderer_properties() -> Vec<Property> {
    vec![
        text(
            "Mesh Path",
            "MeshRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<MeshRenderer>().map(|mesh| mesh.mesh_path.clone()))
                    .unwrap_or_default()
            },
            |scene, entity, value| {
                edit::<MeshRenderer>(scene, entity, |mesh| {
                    mesh.mesh_path = value.to_string()
                })
            },
        ),
        text(
            "Material Path",
            "MeshRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<MeshRenderer>()
                            .map(|mesh| mesh.material_path.clone())
                    })
                    .unwrap_or_default()
            },
            |scene, entity, value| {
                edit::<MeshRenderer>(scene, entity, |mesh| {
                    mesh.material_path = value.to_string()
                })
            },
        ),
        color(
            "Tint",
            "MeshRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<MeshRenderer>().map(|mesh| mesh.tint))
                    .unwrap_or(Vec4::ONE)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<MeshRenderer>(scene, entity, |mesh| {
                    mesh.tint = Vec4::from_array(value)
                })
            },
        ),
        boolean(
            "Visible",
            "MeshRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<MeshRenderer>().map(|mesh| mesh.visible))
                    .unwrap_or(false)
            },
            |scene, entity, value| edit::<MeshRenderer>(scene, entity, |mesh| mesh.visible = value),
        ),
        boolean(
            "Cast Shadows",
            "MeshRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry.get::<MeshRenderer>().map(|mesh| mesh.cast_shadows)
                    })
                    .unwrap_or(false)
            },
            |scene, entity, value| {
                edit::<MeshRenderer>(scene, entity, |mesh| mesh.cast_shadows = value)
            },
        ),
    ]
}

fn sprite_renderer_properties() -> Vec<Property> {
    vec![
        color(
            "Color",
            "SpriteRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<SpriteRenderer>().map(|sprite| sprite.color))
                    .unwrap_or(Vec4::ONE)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<SpriteRenderer>(scene, entity, |sprite| {
                    sprite.color = Vec4::from_array(value)
                })
            },
        ),
        text(
            "Texture Path",
            "SpriteRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<SpriteRenderer>()
                            .map(|sprite| sprite.texture_path.clone())
                    })
                    .unwrap_or_default()
            },
            |scene, entity, value| {
                edit::<SpriteRenderer>(scene, entity, |sprite| {
                    sprite.texture_path = value.to_string()
                })
            },
        ),
        scalar(
            "Tiling Factor",
            "SpriteRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<SpriteRenderer>()
                            .map(|sprite| sprite.tiling_factor)
                    })
                    .unwrap_or(1.0)
            },
            |scene, entity, value| {
                edit::<SpriteRenderer>(scene, entity, |sprite| {
                    sprite.tiling_factor = value
                })
            },
        ),
        boolean(
            "Use SubTexture",
            "SpriteRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<SpriteRenderer>()
                            .map(|sprite| sprite.use_sub_texture)
                    })
                    .unwrap_or(false)
            },
            |scene, entity, value| {
                edit::<SpriteRenderer>(scene, entity, |sprite| {
                    sprite.use_sub_texture = value
                })
            },
        ),
        vector2(
            "SubTexture Coords",
            "SpriteRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<SpriteRenderer>()
                            .map(|sprite| sprite.sub_texture_coords)
                    })
                    .unwrap_or(Vec2::ZERO)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<SpriteRenderer>(scene, entity, |sprite| {
                    sprite.sub_texture_coords = Vec2::from_array(value)
                })
            },
        ),
        vector2(
            "SubTexture Cell Size",
            "SpriteRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<SpriteRenderer>()
                            .map(|sprite| sprite.sub_texture_cell_size)
                    })
                    .unwrap_or(Vec2::ONE)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<SpriteRenderer>(scene, entity, |sprite| {
                    sprite.sub_texture_cell_size = Vec2::from_array(value)
                })
            },
        ),
        vector2(
            "SubTexture Sprite Size",
            "SpriteRenderer",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<SpriteRenderer>()
                            .map(|sprite| sprite.sub_texture_sprite_size)
                    })
                    .unwrap_or(Vec2::ONE)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<SpriteRenderer>(scene, entity, |sprite| {
                    sprite.sub_texture_sprite_size = Vec2::from_array(value)
                })
            },
        ),
    ]
}

fn text_properties() -> Vec<Property> {
    vec![
        text(
            "Text",
            "Text",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Text>().map(|text| text.text.clone()))
                    .unwrap_or_default()
            },
            |scene, entity, value| edit::<Text>(scene, entity, |text| text.text = value.to_string()),
        ),
        text(
            "Font Path",
            "Text",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Text>().map(|text| text.font_path.clone()))
                    .unwrap_or_default()
            },
            |scene, entity, value| {
                edit::<Text>(scene, entity, |text| text.font_path = value.to_string())
            },
        ),
        color(
            "Color",
            "Text",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Text>().map(|text| text.color))
                    .unwrap_or(Vec4::ONE)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<Text>(scene, entity, |text| text.color = Vec4::from_array(value))
            },
        ),
        scalar(
            "Kerning",
            "Text",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Text>().map(|text| text.kerning))
                    .unwrap_or(0.0)
            },
            |scene, entity, value| edit::<Text>(scene, entity, |text| text.kerning = value),
        ),
        scalar(
            "Line Spacing",
            "Text",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Text>().map(|text| text.line_spacing))
                    .unwrap_or(0.0)
            },
            |scene, entity, value| edit::<Text>(scene, entity, |text| text.line_spacing = value),
        ),
    ]
}

const LIGHT_KINDS: &[&str] = &["Directional", "Point", "Spot"];

fn light_properties() -> Vec<Property> {
    vec![
        boolean(
            "Enabled",
            "Light",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Light>().map(|light| light.enabled))
                    .unwrap_or(false)
            },
            |scene, entity, value| edit::<Light>(scene, entity, |light| light.enabled = value),
        ),
        enumeration(
            "Kind",
            "Light",
            LIGHT_KINDS,
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry.get::<Light>().map(|light| match light.kind {
                            LightKind::Directional => 0,
                            LightKind::Point => 1,
                            LightKind::Spot => 2,
                        })
                    })
                    .unwrap_or(0)
            },
            |scene, entity, value| {
                edit::<Light>(scene, entity, |light| {
                    light.kind = match value {
                        0 => LightKind::Directional,
                        1 => LightKind::Point,
                        _ => LightKind::Spot,
                    }
                })
            },
        ),
        vector3(
            "Color",
            "Light",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Light>().map(|light| light.color))
                    .unwrap_or(Vec3::ONE)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<Light>(scene, entity, |light| {
                    light.color = Vec3::from_array(value)
                })
            },
        ),
        scalar(
            "Intensity",
            "Light",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Light>().map(|light| light.intensity))
                    .unwrap_or(1.0)
            },
            |scene, entity, value| edit::<Light>(scene, entity, |light| light.intensity = value),
        ),
        scalar(
            "Radius",
            "Light",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Light>().map(|light| light.radius))
                    .unwrap_or(0.0)
            },
            |scene, entity, value| edit::<Light>(scene, entity, |light| light.radius = value),
        ),
        scalar(
            "Inner Cone",
            "Light",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Light>().map(|light| light.inner_cone_angle))
                    .unwrap_or(0.0)
            },
            |scene, entity, value| {
                edit::<Light>(scene, entity, |light| light.inner_cone_angle = value)
            },
        ),
        scalar(
            "Outer Cone",
            "Light",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Light>().map(|light| light.outer_cone_angle))
                    .unwrap_or(0.0)
            },
            |scene, entity, value| {
                edit::<Light>(scene, entity, |light| light.outer_cone_angle = value)
            },
        ),
    ]
}

const RIGID_BODY_KINDS: &[&str] = &["Static", "Kinematic", "Dynamic"];

fn rigid_body_properties() -> Vec<Property> {
    vec![
        enumeration(
            "Body Type",
            "RigidBody",
            RIGID_BODY_KINDS,
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry.get::<RigidBody>().map(|body| match body.kind {
                            RigidBodyKind::Static => 0,
                            RigidBodyKind::Kinematic => 1,
                            RigidBodyKind::Dynamic => 2,
                        })
                    })
                    .unwrap_or(0)
            },
            |scene, entity, value| {
                edit::<RigidBody>(scene, entity, |body| {
                    body.kind = match value {
                        0 => RigidBodyKind::Static,
                        1 => RigidBodyKind::Kinematic,
                        _ => RigidBodyKind::Dynamic,
                    }
                })
            },
        ),
        scalar(
            "Mass",
            "RigidBody",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<RigidBody>().map(|body| body.mass))
                    .unwrap_or(1.0)
            },
            |scene, entity, value| edit::<RigidBody>(scene, entity, |body| body.mass = value),
        ),
        vector3(
            "Velocity",
            "RigidBody",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<RigidBody>().map(|body| body.velocity))
                    .unwrap_or(Vec3::ZERO)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<RigidBody>(scene, entity, |body| {
                    body.velocity = Vec3::from_array(value)
                })
            },
        ),
        scalar(
            "Damping",
            "RigidBody",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<RigidBody>().map(|body| body.damping))
                    .unwrap_or(0.05)
            },
            |scene, entity, value| edit::<RigidBody>(scene, entity, |body| body.damping = value),
        ),
        scalar(
            "Gravity Scale",
            "RigidBody",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<RigidBody>().map(|body| body.gravity_scale))
                    .unwrap_or(1.0)
            },
            |scene, entity, value| {
                edit::<RigidBody>(scene, entity, |body| body.gravity_scale = value)
            },
        ),
    ]
}

const COLLIDER_SHAPES: &[&str] = &["Box", "Sphere", "Capsule"];

fn collider_properties() -> Vec<Property> {
    vec![
        enumeration(
            "Shape",
            "Collider",
            COLLIDER_SHAPES,
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry.get::<Collider>().map(|collider| {
                            if matches!(&collider.shape, ColliderShape::Sphere { .. }) {
                                1
                            } else if matches!(&collider.shape, ColliderShape::Capsule { .. }) {
                                2
                            } else {
                                0
                            }
                        })
                    })
                    .unwrap_or(0)
            },
            |scene, entity, value| {
                edit::<Collider>(scene, entity, |collider| {
                    // The C++ defaults for a freshly added collider.
                    collider.shape = match value {
                        1 => ColliderShape::Sphere { radius: 0.5 },
                        2 => ColliderShape::Capsule {
                            radius: 0.5,
                            half_height: 1.0,
                        },
                        _ => ColliderShape::Box {
                            half_extent: Vec3::splat(0.5),
                        },
                    }
                })
            },
        ),
        vector3(
            "Offset",
            "Collider",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Collider>().map(|collider| collider.offset))
                    .unwrap_or(Vec3::ZERO)
                    .to_array()
            },
            |scene, entity, value| {
                edit::<Collider>(scene, entity, |collider| {
                    collider.offset = Vec3::from_array(value)
                })
            },
        ),
        vector3(
            "Half Extent",
            "Collider",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry.get::<Collider>().map(|collider| match &collider.shape {
                            ColliderShape::Box { half_extent } => *half_extent,
                            ColliderShape::Sphere { .. } => Vec3::splat(0.5),
                            ColliderShape::Capsule { .. } => Vec3::splat(0.5),
                        })
                    })
                    .unwrap_or(Vec3::splat(0.5))
                    .to_array()
            },
            |scene, entity, value| {
                edit::<Collider>(scene, entity, |collider| {
                    if let ColliderShape::Box { half_extent } = &mut collider.shape {
                        *half_extent = Vec3::from_array(value);
                    }
                })
            },
        ),
        scalar(
            "Radius",
            "Collider",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry.get::<Collider>().map(|collider| match &collider.shape {
                            ColliderShape::Sphere { radius } => *radius,
                            ColliderShape::Capsule { radius, .. } => *radius,
                            ColliderShape::Box { .. } => 0.5,
                        })
                    })
                    .unwrap_or(0.5)
            },
            |scene, entity, value| {
                edit::<Collider>(scene, entity, |collider| match &mut collider.shape {
                    ColliderShape::Sphere { radius } => *radius = value,
                    ColliderShape::Capsule { radius, .. } => *radius = value,
                    ColliderShape::Box { .. } => {}
                })
            },
        ),
        scalar(
            "Half Height",
            "Collider",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry.get::<Collider>().map(|collider| match &collider.shape {
                            ColliderShape::Capsule { half_height, .. } => *half_height,
                            ColliderShape::Box { .. } | ColliderShape::Sphere { .. } => 1.0,
                        })
                    })
                    .unwrap_or(1.0)
            },
            |scene, entity, value| {
                edit::<Collider>(scene, entity, |collider| match &mut collider.shape {
                    ColliderShape::Capsule { half_height, .. } => *half_height = value,
                    ColliderShape::Box { .. } | ColliderShape::Sphere { .. } => {}
                })
            },
        ),
        scalar(
            "Friction",
            "Collider",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Collider>().map(|collider| collider.friction))
                    .unwrap_or(0.2)
            },
            |scene, entity, value| edit::<Collider>(scene, entity, |collider| collider.friction = value),
        ),
        scalar(
            "Restitution",
            "Collider",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry.get::<Collider>().map(|collider| collider.restitution)
                    })
                    .unwrap_or(0.0)
            },
            |scene, entity, value| {
                edit::<Collider>(scene, entity, |collider| collider.restitution = value)
            },
        ),
    ]
}

fn audio_source_properties() -> Vec<Property> {
    vec![
        text(
            "Clip Path",
            "AudioSource",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| {
                        entry
                            .get::<AudioSource>()
                            .map(|source| source.clip_path.clone())
                    })
                    .unwrap_or_default()
            },
            |scene, entity, value| {
                edit::<AudioSource>(scene, entity, |source| {
                    source.clip_path = value.to_string()
                })
            },
        ),
        boolean(
            "Playing",
            "AudioSource",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<AudioSource>().map(|source| source.playing))
                    .unwrap_or(false)
            },
            |scene, entity, value| edit::<AudioSource>(scene, entity, |source| source.playing = value),
        ),
        boolean(
            "Looping",
            "AudioSource",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<AudioSource>().map(|source| source.looping))
                    .unwrap_or(false)
            },
            |scene, entity, value| edit::<AudioSource>(scene, entity, |source| source.looping = value),
        ),
        boolean(
            "Spatial",
            "AudioSource",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<AudioSource>().map(|source| source.spatial))
                    .unwrap_or(false)
            },
            |scene, entity, value| edit::<AudioSource>(scene, entity, |source| source.spatial = value),
        ),
        scalar(
            "Gain",
            "AudioSource",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<AudioSource>().map(|source| source.gain))
                    .unwrap_or(1.0)
            },
            |scene, entity, value| edit::<AudioSource>(scene, entity, |source| source.gain = value),
        ),
        scalar(
            "Pitch",
            "AudioSource",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<AudioSource>().map(|source| source.pitch))
                    .unwrap_or(1.0)
            },
            |scene, entity, value| edit::<AudioSource>(scene, entity, |source| source.pitch = value),
        ),
        scalar(
            "Range",
            "AudioSource",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<AudioSource>().map(|source| source.range))
                    .unwrap_or(0.0)
            },
            |scene, entity, value| edit::<AudioSource>(scene, entity, |source| source.range = value),
        ),
    ]
}

fn animator_properties() -> Vec<Property> {
    vec![
        text(
            "Clip Path",
            "Animator",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Animator>().map(|clip| clip.clip_path.clone()))
                    .unwrap_or_default()
            },
            |scene, entity, value| {
                edit::<Animator>(scene, entity, |animator| {
                    animator.clip_path = value.to_string()
                })
            },
        ),
        boolean(
            "Playing",
            "Animator",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Animator>().map(|clip| clip.playing))
                    .unwrap_or(false)
            },
            |scene, entity, value| edit::<Animator>(scene, entity, |animator| animator.playing = value),
        ),
        boolean(
            "Looping",
            "Animator",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Animator>().map(|clip| clip.looping))
                    .unwrap_or(false)
            },
            |scene, entity, value| edit::<Animator>(scene, entity, |animator| animator.looping = value),
        ),
        scalar(
            "Speed",
            "Animator",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Animator>().map(|clip| clip.speed))
                    .unwrap_or(1.0)
            },
            |scene, entity, value| edit::<Animator>(scene, entity, |animator| animator.speed = value),
        ),
        scalar(
            "Time",
            "Animator",
            |scene, entity| {
                scene
                    .get(entity)
                    .and_then(|entry| entry.get::<Animator>().map(|clip| clip.time))
                    .unwrap_or(0.0)
            },
            |scene, entity, value| edit::<Animator>(scene, entity, |animator| animator.time = value),
        ),
    ]
}
