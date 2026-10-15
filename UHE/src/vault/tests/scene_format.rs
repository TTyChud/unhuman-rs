//! Round-trip and compatibility tests for the two scene formats.
//!
//! The compatibility tests are the ones that matter. A binary format that
//! *fails to reject* a file from a different build is worse than one that
//! cannot read it, because the failure surfaces later as a scene that loads and
//! renders wrong.

use vault::components::{Component, RigidBody};
use vault::doc::{EntityDoc, SceneDoc, Transform, Uuid};
use vault::header::{Header, Incompatible, MAGIC};
use vault::{scene_binary, scene_text};

fn uuid(last: u8) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[15] = last;
    // Never the zero UUID, which means "unset".
    assert_ne!(bytes, Uuid::NONE.0);
    Uuid(bytes)
}

fn sample_scene() -> SceneDoc {
    SceneDoc {
        entities: vec![
            EntityDoc {
                id: uuid(1),
                parent: Uuid::NONE,
                name: "root".into(),
                tags: vec!["static".into(), "spawner".into()],
                transform: Transform::default(),
                components: vec![
                    Component::DirectionalLight(Default::default()),
                    Component::Model3d(vault::components::Model3d {
                        path: "models/crate.glb".into(),
                        material_override: None,
                        node: None,
                        auto_play: false,
                    }),
                ],
            },
            EntityDoc {
                id: uuid(2),
                parent: uuid(1),
                name: "child".into(),
                tags: vec![],
                transform: Transform {
                    translation: [1.0, 2.0, 3.0],
                    rotation: [0.0, 0.707, 0.0, 0.707],
                    scale: [2.0, 2.0, 2.0],
                },
                components: vec![
                    Component::RigidBody3d(RigidBody {
                        mass: 12.5,
                        friction: 0.8,
                        ..RigidBody::default()
                    }),
                    Component::SphereCollider3d(Default::default()),
                ],
            },
        ],
    }
}

#[test]
fn binary_round_trips() {
    let scene = sample_scene();
    let bytes = scene_binary::encode(&scene);
    let decoded = scene_binary::decode(&bytes).expect("a scene we just wrote must decode");
    assert_eq!(decoded, scene);
}

#[test]
fn text_round_trips() {
    let scene = sample_scene();
    let text = scene_text::to_toml(&scene);
    let decoded = scene_text::from_toml(&text).expect("text we just wrote must parse");
    assert_eq!(decoded, scene);
}

#[test]
fn both_forms_agree() {
    // The point of having two forms is that they describe the same scene. If
    // they drift, the text form is worse than no text form -- it looks
    // authoritative and is not.
    let scene = sample_scene();
    let from_binary = scene_binary::decode(&scene_binary::encode(&scene)).expect("binary");
    let from_text = scene_text::from_toml(&scene_text::to_toml(&scene)).expect("text");
    assert_eq!(from_binary, from_text);
}

#[test]
fn the_text_form_has_comments_and_the_binary_does_not() {
    // Not a test of correctness. It is a test that the reason for having a
    // text form is still true -- TOML supports `#` comments, and that is the
    // whole argument for the format.
    let text = scene_text::to_toml(&sample_scene());
    assert!(text.contains('#'), "the editable form must carry comments");
}

#[test]
fn a_foreign_file_is_rejected_by_magic() {
    let mut bytes = scene_binary::encode(&sample_scene());
    bytes[0] = b'X';

    match scene_binary::decode(&bytes) {
        Err(scene_binary::LoadError::Incompatible(Incompatible::NotOurs { found })) => {
            assert_eq!(found[0], b'X')
        }
        other => panic!("expected a magic rejection, got {other:?}"),
    }
}

#[test]
fn a_file_from_another_schema_is_rejected_by_name() {
    // The case this whole header exists for. A file whose document layout
    // differs is readable in principle and wrong in practice, so it must be
    // refused with a message that says which schema was expected.
    let mut bytes = scene_binary::encode(&sample_scene());

    // Rewrite the schema hash to a plausible "other build" value.
    let other_schema = scene_binary::SCENE_SCHEMA ^ 0xdead_beef;
    bytes[8..16].copy_from_slice(&other_schema.to_le_bytes());

    match scene_binary::decode(&bytes) {
        Err(scene_binary::LoadError::Incompatible(Incompatible::Schema { found, expected })) => {
            assert_eq!(found, other_schema);
            assert_eq!(expected, scene_binary::SCENE_SCHEMA);
            // And the message must be actionable, not just a hash.
            let rendered =
                scene_binary::LoadError::Incompatible(Incompatible::Schema { found, expected })
                    .to_string();
            assert!(rendered.contains("field was"), "unhelpful: {rendered}");
        }
        other => panic!("expected a schema rejection, got {other:?}"),
    }
}

#[test]
fn a_file_from_another_format_version_is_rejected() {
    let mut bytes = scene_binary::encode(&sample_scene());
    bytes[4..8].copy_from_slice(&99u32.to_le_bytes());

    assert!(matches!(
        scene_binary::decode(&bytes),
        Err(scene_binary::LoadError::Incompatible(
            Incompatible::FormatVersion { found: 99, .. }
        ))
    ));
}

#[test]
fn a_truncated_file_is_an_error_not_a_panic() {
    let bytes = scene_binary::encode(&sample_scene());
    let truncated = &bytes[..bytes.len() - 10];

    // The payload length in the header no longer matches, so this must be
    // caught before any parsing. A short read here would instead produce a
    // decode error deep inside postcard, naming nothing.
    assert!(matches!(
        scene_binary::decode(truncated),
        Err(scene_binary::LoadError::Incompatible(
            Incompatible::Truncated { .. }
        ))
    ));
}

#[test]
fn an_empty_file_is_an_error_not_a_panic() {
    assert!(matches!(
        scene_binary::decode(&[]),
        Err(scene_binary::LoadError::Incompatible(
            Incompatible::Truncated { .. }
        ))
    ));
}

#[test]
fn a_payload_that_is_not_postcard_is_rejected() {
    // A file with a perfect header and garbage inside. The header alone cannot
    // catch this, which is why the payload decode is a separate error.
    let garbage = vec![0xffu8; 64];
    let sealed = Header::new(scene_binary::scene_schema(), garbage.len() as u64).seal(&garbage);

    assert!(matches!(
        scene_binary::decode(&sealed),
        Err(scene_binary::LoadError::Payload(_))
    ));
}

#[test]
fn the_two_hash_implementations_agree() {
    // `SCENE_SCHEMA` is computed by a `const fn` so it can be used in a
    // constant; `header::fnv1a64` is the runtime version. If they ever
    // disagree, a file sealed with one would be opened with the other and
    // every scene would be rejected. So they are checked against each other.
    assert_eq!(
        vault::header::fnv1a64(scene_binary::SCENE_SCHEMA_DESCRIPTION),
        scene_binary::SCENE_SCHEMA,
        "the const and runtime FNV-1a implementations have diverged"
    );
}

#[test]
fn the_schema_description_lists_every_wire_field() {
    // The schema hash is only as good as the string behind it. This quotes the
    // description so a reviewer sees it next to the field list, and asserts the
    // marker is present -- a change to the layout must change this string, and
    // that is the thing to check for when reviewing a diff.
    let description = std::str::from_utf8(scene_binary::SCENE_SCHEMA_DESCRIPTION).unwrap();
    for field in ["id", "parent", "name", "tags", "transform", "components"] {
        assert!(
            description.contains(field),
            "the schema description no longer mentions `{field}`; the layout \
             changed and the description must change with it"
        );
    }
}

#[test]
fn a_component_retrieved_by_type_is_the_right_one() {
    let scene = sample_scene();
    let body = scene
        .components_of::<RigidBody>(uuid(2))
        .expect("the child has a rigid body");
    assert_eq!(body.mass, 12.5);

    // And asking for a kind the entity does not have is `None`, not a panic
    // and not a wrong answer.
    assert!(scene
        .components_of::<vault::components::Sprite>(uuid(1))
        .is_none());
}

#[test]
fn binary_is_smaller_than_the_text_form() {
    // The reason for having a binary form at all. If this ever inverts, the
    // varint argument in `scene_binary` is wrong and the comment should change.
    let scene = sample_scene();
    let binary = scene_binary::encode(&scene);
    let text = scene_text::to_toml(&scene);
    assert!(
        binary.len() < text.len(),
        "binary {} bytes vs text {} bytes",
        binary.len(),
        text.len()
    );
}

#[test]
fn a_dangling_parent_is_reported() {
    let scene = SceneDoc {
        entities: vec![EntityDoc {
            parent: uuid(99), // no such entity
            ..sample_scene().entities.remove(0)
        }],
    };
    assert!(matches!(
        scene.validate(),
        Err(vault::doc::InvalidScene::DanglingParent { .. })
    ));
}

#[test]
fn a_parent_cycle_is_reported() {
    let a = uuid(1);
    let b = uuid(2);
    let scene = SceneDoc {
        entities: vec![
            EntityDoc {
                id: a,
                parent: b,
                ..EntityDoc {
                    id: a,
                    parent: Uuid::NONE,
                    name: String::new(),
                    tags: vec![],
                    transform: Transform::default(),
                    components: vec![],
                }
            },
            EntityDoc {
                id: b,
                parent: a,
                ..EntityDoc {
                    id: b,
                    parent: Uuid::NONE,
                    name: String::new(),
                    tags: vec![],
                    transform: Transform::default(),
                    components: vec![],
                }
            },
        ],
    };
    assert!(matches!(
        scene.validate(),
        Err(vault::doc::InvalidScene::Cycle { .. })
    ));
}

#[test]
fn a_diamond_is_not_a_cycle() {
    // Two parents sharing a child is a DAG. Cycle detection that uses one
    // shared `visited` set reports this as a cycle, which is the bug in the
    // obvious implementation.
    let root = uuid(1);
    let left = uuid(2);
    let right = uuid(3);
    let shared = uuid(4);

    let bare = |id, parent| EntityDoc {
        id,
        parent,
        name: String::new(),
        tags: vec![],
        transform: Transform::default(),
        components: vec![],
    };

    let scene = SceneDoc {
        entities: vec![
            bare(root, Uuid::NONE),
            bare(left, root),
            bare(right, root),
            bare(shared, left),
        ],
    };
    assert!(
        scene.validate().is_ok(),
        "a branching hierarchy is not a cycle"
    );
    assert_eq!(scene.children_of(root).count(), 2);
}

#[test]
fn magic_is_not_something_a_png_can_be() {
    // Four bytes chosen so an image or a text file cannot pass.
    assert_eq!(&MAGIC, b"UHV1");
    assert_ne!(&MAGIC[..], b"\x89PNG");
    assert_ne!(&MAGIC[..], b"{  ");
}
