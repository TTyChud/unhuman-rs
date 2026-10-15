//! The mesh optimiser: the passes, and the order they must run in.
//!
//! Three of these tests are corrections of tests that were quietly vacuous,
//! taken from what upstream learned the hard way. They are here because a
//! plausible-looking assertion that cannot fail is worse than no assertion.

use std::mem::offset_of;

use glam::{Vec2, Vec3, Vec4};
use vault::gltf::optimizer::{self, acmr};
use vault::gltf::vertex::{offsets, Vertex};

/// A quad in the XY plane, two triangles, four vertices.
///
/// Vertex 0 and vertex 1 share a position and a normal and differ only in UV --
/// the UV seam that the tangent ordering exists to protect.
fn quad() -> (Vec<Vertex>, Vec<u32>) {
    let vertices = vec![
        Vertex {
            position: Vec3::new(0.0, 0.0, 0.0),
            normal: Vec3::Z,
            uv: Vec2::new(0.0, 0.0),
            ..Vertex::unskinned(Vec3::ZERO)
        },
        Vertex {
            position: Vec3::new(1.0, 0.0, 0.0),
            normal: Vec3::Z,
            uv: Vec2::new(1.0, 0.0),
            ..Vertex::unskinned(Vec3::ZERO)
        },
        Vertex {
            position: Vec3::new(1.0, 1.0, 0.0),
            normal: Vec3::Z,
            uv: Vec2::new(1.0, 1.0),
            ..Vertex::unskinned(Vec3::ZERO)
        },
        Vertex {
            position: Vec3::new(0.0, 1.0, 0.0),
            normal: Vec3::Z,
            uv: Vec2::new(0.0, 1.0),
            ..Vertex::unskinned(Vec3::ZERO)
        },
    ];
    (vertices, vec![0, 1, 2, 0, 2, 3])
}

/// A grid mesh, for a scramble that is actually a scramble.
///
/// Reversing triangle order is **not** a scramble for a grid: consecutive
/// triangles stay adjacent in the buffer either way, so any locality assertion
/// on the reversed mesh is vacuous. Upstream found this and switched to a seeded
/// Fisher-Yates shuffle.
fn grid(n: u32) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    for y in 0..n {
        for x in 0..n {
            let (u, v) = (x as f32 / (n - 1) as f32, y as f32 / (n - 1) as f32);
            vertices.push(Vertex {
                position: Vec3::new(u, v, 0.0),
                normal: Vec3::Z,
                uv: Vec2::new(u, v),
                ..Vertex::unskinned(Vec3::ZERO)
            });
        }
    }

    let mut indices = Vec::new();
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let a = y * n + x;
            let b = a + 1;
            let c = a + n;
            let d = c + 1;
            indices.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }

    // Seeded shuffle, so the fixture is reproducible and genuinely disordered.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    for i in (1..indices.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state % (i as u64 + 1)) as usize;
        indices.swap(i, j);
    }

    (vertices, indices)
}

#[test]
fn the_vertex_has_no_padding() {
    // `identity_bytes` reinterprets a `Vertex` as bytes for hashing, which is
    // only sound if there is no padding -- otherwise the hash reads
    // uninitialised memory and two "different" vertices compare equal by luck.
    //
    // This walks every field and asserts the offsets are contiguous, which is
    // what "no padding" means. A field with 8-byte alignment after a 4-byte one
    // would break it.
    let mut expected = 0u32;
    let mut fields: Vec<(&str, u32, u32)> = vec![
        ("position", offsets::POSITION, 12),
        ("normal", offsets::NORMAL, 12),
        ("uv", offsets::UV, 8),
        ("tangent", offsets::TANGENT, 16),
        ("joints", offsets::JOINTS, 8),
        ("joint_weights", offsets::JOINT_WEIGHTS, 16),
        ("colour", offsets::COLOUR, 16),
    ];
    fields.sort_by_key(|(_, offset, _)| *offset);

    for (name, offset, size) in fields {
        assert_eq!(
            offset, expected,
            "{name} is at {offset}, expected {expected}"
        );
        expected += size;
    }

    assert_eq!(expected, std::mem::size_of::<Vertex>() as u32);
    assert_eq!(
        std::mem::align_of::<Vertex>(),
        4,
        "a 16-byte-aligned field would reintroduce the padding this guards"
    );
    assert_eq!(Vertex::STRIDE, 88, "the documented stride");
}

#[test]
fn offsets_match_the_struct() {
    // The offsets are derived with `offset_of!`, so they cannot drift. This
    // checks the *published* numbers, which is what a shader is written against.
    assert_eq!(offsets::POSITION, 0);
    assert_eq!(offsets::NORMAL, 12);
    assert_eq!(offsets::UV, 24);
    assert_eq!(offsets::TANGENT, 32);
    assert_eq!(offsets::JOINTS, 48);
    assert_eq!(offsets::JOINT_WEIGHTS, 56);
    assert_eq!(offsets::COLOUR, 72);
    assert_eq!(offset_of!(Vertex, position), 0);
}

#[test]
fn a_default_vertex_is_white_not_black() {
    // The single most common "why is my model black" cause. A zero default
    // tints everything that has no COLOR_0 to black.
    let vertex = Vertex::unskinned(Vec3::ZERO);
    assert_eq!(vertex.colour, [1.0; 4]);
    assert_eq!(vertex.normal, Vec3::Y);
    assert_eq!(
        vertex.tangent[3], 1.0,
        "a zero handedness mirrors normal maps"
    );
    assert!(!vertex.is_skinned());
}

#[test]
fn tangents_are_unit_length_and_perpendicular_to_the_normal() {
    let (mut vertices, indices) = grid(8);
    assert!(optimizer::generate_tangents(&mut vertices, &indices));

    for (index, vertex) in vertices.iter().enumerate() {
        let tangent = Vec4::from_array(vertex.tangent).truncate();
        let length = tangent.length();
        assert!(
            (0.99..=1.01).contains(&length),
            "vertex {index} tangent has length {length}"
        );
        let dot = vertex.normal.normalize().dot(tangent);
        assert!(
            dot.abs() <= 0.01,
            "vertex {index} tangent is not perpendicular to its normal (dot {dot})"
        );
        assert_eq!(
            vertex.tangent[3], 1.0,
            "handedness should be positive for a CCW planar mesh"
        );
    }
}

#[test]
fn degenerate_uvs_do_not_produce_nan() {
    // Every UV identical: the UV-space determinant is zero, so the scale is
    // infinite. The accumulator has to skip rather than multiply.
    let mut vertices = quad().0;
    for vertex in &mut vertices {
        vertex.uv = Vec2::ZERO;
    }
    let indices = vec![0, 1, 2, 0, 2, 3];

    optimizer::generate_tangents(&mut vertices, &indices);

    for (index, vertex) in vertices.iter().enumerate() {
        let tangent = Vec4::from_array(vertex.tangent).truncate();
        assert!(
            tangent.is_finite(),
            "vertex {index} tangent is not finite: {tangent:?}"
        );
        assert!(
            tangent.length() > 0.5,
            "vertex {index} tangent is zero-length"
        );
    }
}

#[test]
fn tangent_generation_preserves_the_triangle_count() {
    let (mut vertices, indices) = grid(6);
    let before = indices.len() / 3;
    optimizer::generate_tangents(&mut vertices, &indices);
    assert_eq!(
        indices.len() / 3,
        before,
        "tangent generation must not touch indices"
    );
    assert_eq!(vertices.len(), 36, "and must not touch the vertex count");
}

#[test]
fn cache_optimisation_improves_the_acmr() {
    // On a genuinely scrambled grid. The reversed-order variant would pass
    // vacuously -- see the fixture's note.
    let (vertices, indices) = grid(16);
    let scrambled = acmr(&indices, 32);

    let optimised = optimizer::optimize_vertex_cache(&indices, vertices.len());
    let improved = acmr(&optimised, 32);

    assert!(
        improved < scrambled,
        "cache pass did not help: {scrambled} -> {improved}"
    );
    assert_eq!(
        optimised.len(),
        indices.len(),
        "and must not change the index count"
    );
    assert_eq!(optimised.len() % 3, 0, "whole triangles only");
}

#[test]
fn cache_optimisation_emits_every_vertex_and_no_phantoms() {
    let (vertices, indices) = grid(8);
    let optimised = optimizer::optimize_vertex_cache(&indices, vertices.len());

    let mut sorted_original = indices.clone();
    let mut sorted_optimised = optimised.clone();
    sorted_original.sort_unstable();
    sorted_optimised.sort_unstable();

    assert_eq!(
        sorted_original, sorted_optimised,
        "reordering must permute the index buffer, not lose or invent corners"
    );
}

#[test]
fn cache_optimisation_survives_degenerate_input() {
    assert!(optimizer::optimize_vertex_cache(&[], 0).is_empty());
    assert_eq!(optimizer::optimize_vertex_cache(&[0, 1], 2), vec![0, 1]);
    // An index past the vertex count must not panic.
    let _ = optimizer::optimize_vertex_cache(&[0, 1, 99], 2);
}

#[test]
fn merging_collapses_only_byte_identical_vertices() {
    let mut vertices = quad().0;
    // Two genuinely identical duplicates appended.
    vertices.push(vertices[0]);
    vertices.push(vertices[1]);

    let remap = optimizer::generate_vertex_remap(&vertices);
    assert_eq!(remap.len(), vertices.len());
    assert_eq!(
        optimizer::merged_vertex_count(&vertices),
        4,
        "6 in, 4 distinct"
    );

    // And a UV-only difference is NOT merged, which is the whole point.
    assert_ne!(
        remap[0], remap[1],
        "vertices differing only in UV must stay separate"
    );
}

#[test]
fn a_uv_sphere_is_not_a_valid_merge_test() {
    // Upstream's correction, kept because the reasoning is the useful part.
    //
    // On a UV sphere, seam vertices share position and normal and differ only
    // in UV. "No reduction" is therefore the *correct* answer for a sphere, and
    // a test asserting reduction on one is asserting a bug.
    let (mut vertices, _) = grid(4);
    // Split every vertex into a UV-seam pair: same position and normal, uv and
    // uv + (1, 0).
    let mut doubled = Vec::with_capacity(vertices.len() * 2);
    for vertex in &vertices {
        doubled.push(*vertex);
        let mut seam = *vertex;
        seam.uv += Vec2::X;
        doubled.push(seam);
    }
    vertices = doubled;

    let remap = optimizer::generate_vertex_remap(&vertices);
    assert_eq!(
        optimizer::merged_vertex_count(&vertices),
        vertices.len(),
        "a UV-seam mesh must not merge"
    );
    assert!(remap.iter().enumerate().all(|(i, r)| *r as usize == i));
}

#[test]
fn merging_is_skipped_for_skinned_meshes() {
    // Two vertices at the same position with the same UV but different bone
    // weights: that is a bone crease, and merging collapses the deformation.
    let mut vertices = quad().0;
    vertices[1].joint_weights = [0.5, 0.5, 0.0, 0.0];
    vertices[1].joints = [3, 4, -1, -1];

    // Unskinned: the two halves are not byte-identical, so nothing merges.
    let (_, _, unskinned) = optimizer::optimize(vertices.clone(), vec![0, 1, 2, 0, 2, 3], false);
    assert_eq!(unskinned.vertices_before, 4);

    // Skinned: merging is skipped entirely, so the count cannot drop.
    let (_, _, skinned) = optimizer::optimize(vertices, vec![0, 1, 2, 0, 2, 3], true);
    assert!(skinned.merge_skipped, "a skinned mesh must not merge");
    assert_eq!(skinned.vertices_before, skinned.vertices_after);
}

#[test]
fn fetch_optimisation_does_not_lose_triangles() {
    let (vertices, indices) = grid(10);
    let remap = optimizer::optimize_vertex_fetch_remap(&vertices, &indices);

    // The remap must be a bijection before anything else can be true of it.
    let mut seen = vec![false; vertices.len()];
    for target in &remap {
        assert!(
            (*target as usize) < vertices.len(),
            "remap target {target} out of range"
        );
        assert!(
            !seen[*target as usize],
            "remap is not injective: {target} twice"
        );
        seen[*target as usize] = true;
    }
    assert!(seen.iter().all(|hit| *hit), "remap misses a vertex");

    let remapped = optimizer::remap_indices(&indices, &remap);

    // In range before and after, and the same length.
    assert_eq!(remapped.len(), indices.len());
    for (before, after) in indices.iter().zip(&remapped) {
        assert!(
            (*before as usize) < vertices.len(),
            "index out of range before remap"
        );
        assert!(
            (*after as usize) < vertices.len(),
            "remap produced an out-of-range index"
        );
    }

    // The bijection check above is the whole of "this is a permutation". A
    // bijection on 0..n, applied to a list whose entries are all in 0..n, cannot
    // change the multiset -- so an explicit multiset comparison here would be
    // implied rather than additional. Kept as the bijection because that is the
    // property a caller of `remap_indices` actually depends on, and because it
    // fails with a message naming the offending vertex.
}

#[test]
fn fetch_optimisation_groups_large_triangles() {
    // A grid plus one big triangle far away. The big triangle's vertices should
    // end up near index 0, because that is where the locality is.
    let (mut vertices, mut indices) = grid(4);
    let base = vertices.len() as u32;

    vertices.push(Vertex::unskinned(Vec3::new(100.0, 0.0, 0.0)));
    vertices.push(Vertex::unskinned(Vec3::new(101.0, 0.0, 0.0)));
    vertices.push(Vertex::unskinned(Vec3::new(100.0, 1.0, 0.0)));
    indices.extend_from_slice(&[base, base + 1, base + 2]);

    let remap = optimizer::optimize_vertex_fetch_remap(&vertices, &indices);
    let sorted: Vec<u32> = {
        let mut by_old: Vec<(u32, u32)> = remap
            .iter()
            .copied()
            .enumerate()
            .map(|(old, new)| (old as u32, new))
            .collect();
        by_old.sort_by_key(|(_, new)| *new);
        by_old.into_iter().map(|(old, _)| old).collect()
    };

    let big_triangle_positions: Vec<u32> = [base, base + 1, base + 2]
        .iter()
        .map(|index| remap[*index as usize])
        .collect();

    for position in &big_triangle_positions {
        assert!(
            *position < sorted.len() as u32 / 2,
            "a vertex of the largest triangle landed at {position}"
        );
    }
}

#[test]
fn the_whole_pipeline_keeps_the_index_count() {
    let (vertices, indices) = grid(12);
    let (out_vertices, out_indices, stats) = optimizer::optimize(vertices, indices, false);

    assert_eq!(out_indices.len(), stats.indices);
    assert_eq!(out_indices.len() % 3, 0);
    assert!(out_vertices.len() >= 4);
    for index in &out_indices {
        assert!(*index as usize <= out_vertices.len().saturating_sub(1));
    }
    assert!(
        stats.acmr_after <= stats.acmr_before + f32::EPSILON,
        "the cache pass made locality worse: {} -> {}",
        stats.acmr_before,
        stats.acmr_after
    );
}

#[test]
fn the_pipeline_tolerates_an_empty_mesh() {
    let (vertices, indices, stats) = optimizer::optimize(Vec::new(), Vec::new(), false);
    assert!(vertices.is_empty());
    assert!(indices.is_empty());
    assert_eq!(stats.vertices_before, 0);
    assert_eq!(stats.indices, 0);
}
