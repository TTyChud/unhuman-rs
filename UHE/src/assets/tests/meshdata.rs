//! Model geometry from a hand-written glTF, and the animation compression
//! round trip against its stated error.

use std::fs;
use std::path::PathBuf;

use vault::{
    compress, AnimationData, AssetKind, Assets, CompressSettings, Interpolation, VectorTrack,
};

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("vault-meshdata-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch directory");
        Self { root }
    }

    fn write(&self, name: &str, contents: &[u8]) {
        fs::write(self.root.join(name), contents).expect("scratch file");
    }

    fn assets(&self) -> Assets {
        Assets::new(&self.root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A triangle with no indices: the loader has to synthesise them, and the
/// vertex count is what the test checks the stream against.
const TRIANGLE_GLTF: &str = r#"{
  "asset": { "version": "2.0" },
  "scene": 0,
  "scenes": [ { "nodes": [ 0 ] } ],
  "nodes": [ { "name": "TriangleNode", "mesh": 0 } ],
  "meshes": [ {
    "name": "Triangle",
    "primitives": [ { "attributes": { "POSITION": 0 }, "material": 0 } ]
  } ],
  "materials": [ {
    "name": "Paint",
    "pbrMetallicRoughness": {
      "baseColorFactor": [ 1.0, 0.5, 0.25, 1.0 ],
      "metallicFactor": 0.25,
      "roughnessFactor": 0.75
    },
    "doubleSided": true
  } ],
  "buffers": [ { "uri": "triangle.bin", "byteLength": 36 } ],
  "bufferViews": [ { "buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962 } ],
  "accessors": [ {
    "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
    "min": [ 0.0, 0.0, 0.0 ], "max": [ 1.0, 1.0, 0.0 ]
  } ]
}"#;

/// Three positions, little-endian `f32`: (0,0,0), (1,0,0), (0,1,0).
const TRIANGLE_BIN: [f32; 9] = [
    0.0, 0.0, 0.0, //
    1.0, 0.0, 0.0, //
    0.0, 1.0, 0.0,
];

#[test]
fn a_non_indexed_triangle_decodes_with_synthesised_indices_and_normals() {
    let scratch = Scratch::new("triangle");
    scratch.write("triangle.gltf", TRIANGLE_GLTF.as_bytes());
    let mut bin = Vec::with_capacity(36);
    for value in TRIANGLE_BIN {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    scratch.write("triangle.bin", &bin);

    let mut assets = scratch.assets();
    let id = assets.load("triangle.gltf").expect("the model loads");
    assert_eq!(assets.kind(id), Some(AssetKind::Model));

    let mesh = assets.mesh(id).expect("the model has a mesh");
    assert_eq!(mesh.name, "Triangle");
    assert_eq!(mesh.positions.len(), 3);
    assert_eq!(mesh.positions[1], [1.0, 0.0, 0.0]);
    // glTF without indices is legal; the loader synthesises them, because a
    // zero index count is a silently skipped draw.
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    // And with no NORMAL attribute the normals come from the triangle itself.
    assert_eq!(mesh.normals.len(), 3);
    for normal in &mesh.normals {
        assert!((normal[2] - 1.0).abs() < 1e-6, "{normal:?}");
    }
    // No TEXCOORD_0 means zeroed UVs, one per vertex.
    assert_eq!(mesh.uvs, vec![[0.0, 0.0]; 3]);
    assert!(!mesh.has_vertex_color);
    assert!(mesh.colors.is_empty());
    assert!(mesh.tangents.is_empty());
    assert_eq!(mesh.bounds_min, [0.0, 0.0, 0.0]);
    assert_eq!(mesh.bounds_max, [1.0, 1.0, 0.0]);

    let material = mesh
        .material
        .as_ref()
        .expect("the primitive names material 0");
    assert_eq!(material.name, "Paint");
    assert_eq!(material.base_color, [1.0, 0.5, 0.25, 1.0]);
    assert_eq!(material.metallic, 0.25);
    assert_eq!(material.roughness, 0.75);
    assert!(material.double_sided);

    // One node, one primitive, identity placement.
    let model = assets.model(id).expect("the model has a node graph");
    assert_eq!(model.roots, vec![0]);
    assert_eq!(model.nodes.len(), 1);
    let node = &model.nodes[0];
    assert_eq!(node.name, "TriangleNode");
    assert_eq!(node.parent, -1);
    assert_eq!(node.primitives, vec![0]);
    assert_eq!(
        node.world_transform,
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    );
    assert!(model.skins.is_empty());
    assert_eq!(assets.meshes(id).map(|meshes| meshes.len()), Some(1));
}

#[test]
fn a_model_that_is_not_gltf_is_an_error() {
    let scratch = Scratch::new("bad-gltf");
    scratch.write("broken.gltf", b"{ not json at all");

    let mut assets = scratch.assets();
    let error = assets.load("broken.gltf").expect_err("refused");
    assert!(matches!(error, vault::Error::Decode(_)), "{error}");
}

fn curve(samples: usize) -> VectorTrack {
    let mut track = VectorTrack {
        interpolation: Interpolation::Linear,
        ..Default::default()
    };
    for index in 0..samples {
        let time = index as f32 / (samples - 1) as f32;
        track.times.push(time);
        // Curvature in two components, so a segment that "happens" to pass
        // through the endpoints still fails the tolerance check.
        track.values.push([time * time, time.sin(), 0.0]);
    }
    track
}

#[test]
fn compressed_keys_stay_within_the_stated_error_at_their_own_sample_times() {
    let settings = CompressSettings::default();
    let mut clip = AnimationData {
        name: "Wave".to_owned(),
        duration: 1.0,
        position_tracks: vec![curve(65)],
        ..Default::default()
    };
    let times = clip.position_tracks[0].times.clone();
    let values = clip.position_tracks[0].values.clone();

    let stats = compress(&mut clip, settings);
    assert_eq!(stats.tracks_processed, 1);
    assert_eq!(stats.keys_before, 65);
    assert!(
        stats.keys_after < stats.keys_before,
        "a curved track should lose keys, kept {}",
        stats.keys_after
    );
    assert_eq!(stats.keys_after, clip.position_tracks[0].times.len());
    assert!(stats.byte_reduction() > 0.0);

    for (time, value) in times.iter().zip(&values) {
        let sampled = clip.position_tracks[0].sample(*time);
        let error = ((sampled[0] - value[0]).powi(2)
            + (sampled[1] - value[1]).powi(2)
            + (sampled[2] - value[2]).powi(2))
        .sqrt();
        assert!(
            error <= settings.position_tolerance + 1e-6,
            "error {error} at {time}"
        );
    }
}

#[test]
fn measuring_reports_the_same_reduction_without_applying_it() {
    let clip = AnimationData {
        duration: 1.0,
        position_tracks: vec![curve(65)],
        ..Default::default()
    };
    let settings = CompressSettings::default();

    let measured = vault::measure(&clip, settings);
    let mut applied_clip = clip.clone();
    let applied = compress(&mut applied_clip, settings);

    assert_eq!(measured.keys_before, applied.keys_before);
    assert_eq!(measured.keys_after, applied.keys_after);
    assert_eq!(clip.position_tracks[0].times.len(), 65);
    assert_eq!(
        applied_clip.position_tracks[0].times.len(),
        applied.keys_after
    );
}

#[test]
fn a_step_track_keeps_every_key() {
    let mut track = curve(33);
    track.interpolation = Interpolation::Step;
    let mut clip = AnimationData {
        duration: 1.0,
        position_tracks: vec![track],
        ..Default::default()
    };
    let stats = compress(&mut clip, CompressSettings::default());
    assert_eq!(stats.keys_after, 33);
    assert_eq!(clip.position_tracks[0].times.len(), 33);
}
