//! The component set a scene file can hold.
//!
//! ## The shape
//!
//! One enum, not a trait object per component and not a `Vec<Box<dyn Any>>`.
//! An enum means a scene file is self-describing enough for a tool to read, its
//! size is known, and adding a component is one line in the macro below --
//! rather than a new type, a new registration, and a second place to remember.
//!
//! ## The trait
//!
//! [`ComponentKind`] exists so `SceneDoc::components_of::<T>()` works. It is
//! five lines and entirely safe: the enum erases to `&dyn Any` by coercion,
//! then `downcast_ref` checks the type at runtime. The runtime check is
//! deliberate -- it costs a `TypeId` comparison per lookup and it means a
//! mismatch between a type and the name it declares is a `None`, never
//! undefined behaviour. An earlier version cast through a name comparison and
//! was sound only if the two lists stayed in step by hand.
//!
//! ## What is deliberately absent
//!
//! `Transform`, `ID`, `Tag` and `Relationship` are **not** variants. They are
//! fields on [`crate::doc::EntityDoc`]. In the C++ they are four components
//! because EnTT has no other way to hold them; a document has, and making them
//! optional variants would mean every consumer handles both "present as a
//! component" and "present as a field".

use serde::{Deserialize, Serialize};
use std::any::Any;

mod light;
mod misc;
mod physics;
mod render;

pub use light::{DirectionalLight, PointLight, SpotLight};
pub use misc::{Animator, NativeScript, Property, SpriteAnimation, Text};
pub use physics::{
    BodyType, BoxCollider2d, BoxCollider3d, CapsuleCollider3d, RigidBody, RigidBody2d,
    SphereCollider3d,
};
pub use render::{Camera2d, Model3d, Sprite};

/// Lets a component be pulled back out of the enum by type.
pub trait ComponentKind: Any + Clone {
    /// The [`Component::kind_name`] of the variant this type lives in.
    const KIND_NAME: &'static str;
}

/// Declares the component set: the binary enum, the typed accessor, and the
/// TOML mirror.
///
/// One list, three outputs. Generating the TOML mirror from the same list is
/// the point: a hand-written second list is a list that will drift, and a
/// component that serialises but cannot be read back out of a text file is
/// worse than one that never existed.
macro_rules! declare_components {
    ($($variant:ident($ty:ty) => $name:literal,)*) => {
        /// One [`Component`], as stored in a scene file.
        ///
        /// `missing_docs` is off for the variants only: each is named by the
        /// `$name` literal in the macro invocation below, and the doc is the
        /// payload struct's own.
        #[allow(missing_docs)]
        #[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
        #[non_exhaustive]
        pub enum Component {
            $($variant($ty),)*
        }

        impl Component {
            /// A stable name for this kind, for diagnostics and text export.
            pub fn kind_name(&self) -> &'static str {
                match self {
                    $(Component::$variant(_) => $name,)*
                }
            }

            /// Erase to `&dyn Any`, by coercion. Sound because every arm yields
            /// a reference to a concrete type.
            fn as_any(&self) -> &dyn Any {
                match self {
                    $(Component::$variant(inner) => inner,)*
                }
            }

            /// The payload as a `&T`, or `None` if this is a different kind.
            pub fn as_kind<T: ComponentKind>(&self) -> Option<&T> {
                // The name check is an optimisation, not the safety argument --
                // `downcast_ref` re-checks the type, so a `KIND_NAME` that
                // disagrees with reality yields `None` rather than a bad cast.
                if self.kind_name() != T::KIND_NAME {
                    return None;
                }
                self.as_any().downcast_ref()
            }
        }

        $(
            impl ComponentKind for $ty {
                const KIND_NAME: &'static str = $name;
            }
        )*

        /// A component as it appears in an editable file.
        ///
        /// An internally-tagged serde enum, so it serialises as a flat table
        /// with the kind as one key among the component's own fields:
        ///
        /// ```toml
        /// kind = "RigidBody3d"          # <- same table, not a nested one
        /// body_type = "dynamic"
        /// mass = 1.0
        /// ```
        ///
        /// The obvious alternative -- `Vec<toml::Value>`, tagging each by
        /// inserting a `kind` key -- needs `toml::to_value`, which toml 1.x
        /// does not export, and would mean a second encode and decode written
        /// by hand for every component. Serde's tag does both.
        // No docs on the variants: each mirrors a documented type, and a second
        // copy of the same documentation would drift.
        #[allow(missing_docs)]
        #[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
        #[serde(tag = "kind")]
        #[non_exhaustive]
        pub enum TextComponent {
            $($variant($ty),)*
        }

        impl From<&Component> for TextComponent {
            fn from(component: &Component) -> Self {
                match component {
                    $(Component::$variant(inner) => TextComponent::$variant(inner.clone()),)*
                }
            }
        }

        impl From<TextComponent> for Component {
            fn from(text: TextComponent) -> Self {
                match text {
                    $(TextComponent::$variant(inner) => Component::$variant(inner),)*
                }
            }
        }
    };
}

declare_components! {
    Sprite(Sprite) => "Sprite",
    Camera2d(Camera2d) => "Camera2d",
    Model3d(Model3d) => "Model3d",
    DirectionalLight(DirectionalLight) => "DirectionalLight",
    PointLight(PointLight) => "PointLight",
    SpotLight(SpotLight) => "SpotLight",
    RigidBody3d(RigidBody) => "RigidBody3d",
    BoxCollider3d(BoxCollider3d) => "BoxCollider3d",
    SphereCollider3d(SphereCollider3d) => "SphereCollider3d",
    CapsuleCollider3d(CapsuleCollider3d) => "CapsuleCollider3d",
    RigidBody2d(RigidBody2d) => "RigidBody2d",
    BoxCollider2d(BoxCollider2d) => "BoxCollider2d",
    SpriteAnimation(SpriteAnimation) => "SpriteAnimation",
    Text(Text) => "Text",
    NativeScript(NativeScript) => "NativeScript",
    Animator(Animator) => "Animator",
}
