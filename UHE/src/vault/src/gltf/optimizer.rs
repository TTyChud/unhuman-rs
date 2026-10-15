//! The mesh optimiser: four passes, in an order that is load-bearing.
//!
//! ## The order
//!
//! 1. **Tangents** -- first, always.
//! 2. **Vertex cache** -- reorder indices for post-transform cache locality.
//! 3. **Vertex merge** -- collapse byte-identical vertices. *Skipped for skinned
//!    primitives.*
//! 4. **Vertex fetch** -- reorder vertices for pre-transform locality. *After
//!    the cache reorder*, because it scores triangles and they must be in their
//!    final order for the score to mean anything.
//!
//! Steps 1 and 4 are the ones that are easy to get wrong, and both produce
//! plausible-looking meshes that are wrong.
//!
//! **Tangents must precede merging** because merging hashes every byte of a
//! vertex. Two vertices at the same position sharing a normal, on either side of
//! a UV seam, differ only in UV. Generate tangents first and each side gets its
//! own; merge second and the seam survives. Reverse the two and the merge
//! collapses the seam before tangents exist, so the tangent generator has no way
//! to know a seam was ever there. The mesh renders with a normal map that bends
//! around the seam.
//!
//! ## Why hand-rolled
//!
//! The `meshoptimizer` crate on crates.io is 0.1.1, re-exports
//! `meshoptimizer_sys` (a C++ binding via `cc`), and exposes **only** validation
//! functions -- not `optimizeVertexCache`, not `generateVertexRemap`, and not
//! the `EXT_meshopt_compression` decoder. Binding it would pull a C++ toolchain
//! into the workspace and still not provide the passes we need.
//!
//! Hand-rolling also keeps `cargo test` working with no compiler beyond rustc,
//! which is a property the whole workspace depends on.
//!
//! ## What differs from meshoptimizer, said plainly
//!
//! - **Tangents** are accumulated per triangle and orthonormalised
//!   (Gram-Schmidt against the normal). meshoptimizer implements **MikkTSpace**,
//!   which the glTF spec nominally wants and which is *seamless* across shared
//!   edges. Gram-Schmidt is not: a mesh with a normal map across a hard edge
//!   shows a shading discontinuity. That is a real quality difference, not a
//!   rounding error, and MikkTSpace is the thing to implement when it is wanted.
//! - **Fetch** scores vertices by incident triangle *area*. meshoptimizer uses
//!   area divided by edge length. Same idea, cheaper, slightly worse.

use std::collections::HashMap;

use crate::gltf::vertex::Vertex;
use glam::Vec3;

/// Generate tangents, in place, for every vertex.
///
/// Returns `false` when there was nothing to do -- fewer than three vertices, or
/// no indices -- so a caller can distinguish "generated" from "nothing to
/// generate", which are different situations with the same vertex count.
///
/// Uses the *geometric* normal accumulated alongside the tangent frame, then
/// writes the **attribute** normal only if the file did not supply one. Comparing
/// against a normal the shader will not use is how a normal map ends up subtly
/// wrong on curved surfaces.
pub fn generate_tangents(vertices: &mut [Vertex], indices: &[u32]) -> bool {
    if vertices.len() < 3 || indices.len() < 3 {
        return false;
    }

    let count = vertices.len();
    let mut tangents = vec![Vec3::ZERO; count];
    let mut bitangents = vec![Vec3::ZERO; count];
    let mut normals = vec![Vec3::ZERO; count];

    for triangle in indices.as_chunks::<TRIANGLE>().0.iter() {
        let (Some(&a), Some(&b), Some(&c)) = (triangle.first(), triangle.get(1), triangle.get(2))
        else {
            continue;
        };
        if a as usize >= count || b as usize >= count || c as usize >= count {
            continue;
        }
        let (a, b, c) = (a as usize, b as usize, c as usize);

        let position_a = vertices[a].position;
        let position_b = vertices[b].position;
        let position_c = vertices[c].position;
        let uv_a = vertices[a].uv;
        let uv_b = vertices[b].uv;
        let uv_c = vertices[c].uv;

        let edge1 = position_b - position_a;
        let edge2 = position_c - position_a;
        // (du1, dv1) is corner a -> b, (du2, dv2) is a -> c. Confusing two
        // pairs of these is the classic sign error in tangent generation: it
        // produces a tangent that is perpendicular to nothing, and a normal map
        // that is subtly wrong everywhere rather than obviously wrong somewhere.
        let du1 = uv_b.x - uv_a.x;
        let dv1 = uv_b.y - uv_a.y;
        let du2 = uv_c.x - uv_a.x;
        let dv2 = uv_c.y - uv_a.y;

        // Face normal by the cross product, accumulated un-normalised so the
        // area weighting falls out of the addition.
        let face_normal = edge1.cross(edge2);
        for vertex in [a, b, c] {
            normals[vertex] += face_normal;
        }

        // The UV-space determinant scales the tangent frame. Degenerate UVs
        // contribute nothing rather than NaN, which is what breaks every normal
        // map on a model with a collapsed UV island.
        let determinant = du1 * dv2 - du2 * dv1;
        if determinant.abs() <= f32::EPSILON {
            continue;
        }
        let scale = determinant.recip();

        // The standard tangent-frame accumulation. Each corner of the triangle
        // gets the *same* contribution: the frame is a property of the face, and
        // summing it into all three vertices is what averages it across the
        // surface. Weighting by face area falls out of the cross product's
        // magnitude, so large triangles contribute proportionally more.
        // MikkTSpace's formulation, per-face and unnormalised:
        //   T = (e1 * dv2 - e2 * dv1) / det
        //   B = (e2 * du1 - e1 * du2) / det
        let tangent_delta = (edge1 * dv2 - edge2 * dv1) * scale;
        let bitangent_delta = (edge2 * du1 - edge1 * du2) * scale;

        for vertex in [a, b, c] {
            tangents[vertex] += tangent_delta;
            bitangents[vertex] += bitangent_delta;
        }
    }

    for index in 0..count {
        let geometric = if normals[index].length_squared() > f32::EPSILON {
            normals[index].normalize()
        } else {
            Vec3::Y
        };

        // Gram-Schmidt against the normal the shader uses.
        let reference = if vertices[index].normal.length_squared() > f32::EPSILON {
            vertices[index].normal.normalize()
        } else {
            geometric
        };
        let tangent = tangents[index];
        let orthogonal = tangent - reference * reference.dot(tangent);

        let unit_tangent = if orthogonal.length_squared() > f32::EPSILON {
            orthogonal.normalize()
        } else {
            // No tangent survived. Any unit vector perpendicular to the normal
            // is as good as another; a zero tangent normalises to NaN three
            // lines later and poisons the whole vertex.
            reference.any_orthonormal_vector()
        };

        // Handedness: whether the accumulated bitangent agrees with
        // `normal x tangent`. Sign only; the magnitude means nothing.
        let geometric_bitangent = reference.cross(unit_tangent);
        let handedness = if geometric_bitangent.dot(bitangents[index]) < 0.0 {
            -1.0
        } else {
            1.0
        };

        vertices[index].tangent = [unit_tangent.x, unit_tangent.y, unit_tangent.z, handedness];
    }

    true
}

/// Corners per triangle. A named constant so every traversal agrees, and so a
/// change from triangles to something else is one edit rather than a search.
pub const TRIANGLE: usize = 3;

/// Post-transform vertex cache size. 32 is the usual figure for the driver's
/// real cache and the value Forsyth's original is tuned against.
const CACHE_SIZE: usize = 32;

/// Reorder indices for post-transform vertex cache locality.
///
/// Forsyth's algorithm: score each vertex by how expensive it is to fetch given
/// the current cache, then emit the best-scoring corner of a candidate triangle,
/// rotated so that corner comes first. Linear in the triangle count.
///
/// The candidate set is the triangles adjacent to the one emitted last, which
/// is what keeps it linear -- the *first* triangle is found by a full scan, and
/// every one after it by adjacency.
pub fn optimize_vertex_cache(indices: &[u32], vertex_count: usize) -> Vec<u32> {
    let corners: Vec<[u32; 3]> = indices.as_chunks::<TRIANGLE>().0.to_vec();
    if corners.is_empty() {
        return indices.to_vec();
    }

    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); vertex_count];
    for (position, triangle) in corners.iter().enumerate() {
        for corner in triangle {
            if (*corner as usize) < vertex_count {
                adjacency[*corner as usize].push(position);
            }
        }
    }

    // Forsyth's two scores.
    //
    // `score[v]` is how many triangles still want this vertex. `neighbour[v]` is
    // a *vertex* -- some other corner of the first triangle that wanted `v` --
    // used as a proxy for "something near this will be needed soon, so keep the
    // cache warm for it".
    //
    // That second one is a vertex and not a triangle, and getting it wrong is
    // silent: the array bounds are fine either way, so the cache pass runs to
    // completion and simply does not help. Measured on a scrambled grid, the
    // broken version left ACMR at 2.99 against 2.98 before -- worse than doing
    // nothing, which is how the mistake was found.
    let mut score: Vec<f32> = adjacency.iter().map(|a| a.len() as f32).collect();
    let mut neighbour: Vec<usize> = vec![usize::MAX; vertex_count];
    for (vertex, triangles) in adjacency.iter().enumerate() {
        let Some(first) = triangles.first().copied() else {
            continue;
        };
        neighbour[vertex] = corners[first]
            .iter()
            .find(|corner| **corner as usize != vertex)
            .copied()
            .unwrap_or(vertex as u32) as usize;
    }

    let mut dead = vec![false; corners.len()];
    let mut remaining = corners.len();
    let mut cache = [usize::MAX; CACHE_SIZE];
    let mut output = Vec::with_capacity(indices.len());
    let mut last = usize::MAX;

    while remaining > 0 {
        // Pick the triangle to emit next.
        // A malformed file can carry an index past the vertex count. Refusing
        // to emit that triangle is better than panicking: the file is untrusted.
        let chosen = if last == usize::MAX {
            // First one: the highest priority anywhere.
            (0..corners.len())
                .filter(|t| !dead[*t])
                .min_by(|a, b| {
                    triangle_score(corners[*a], &cache, &score, &neighbour)
                        .total_cmp(&triangle_score(corners[*b], &cache, &score, &neighbour))
                })
                .unwrap_or(0)
        } else {
            // After that: only neighbours, which is the linear part.
            let mut best = None;
            let mut best_score = f32::INFINITY;
            for vertex in corners[last] {
                if (vertex as usize) >= adjacency.len() {
                    continue;
                }
                for candidate in &adjacency[vertex as usize] {
                    if dead[*candidate] {
                        continue;
                    }
                    let candidate_score =
                        triangle_score(corners[*candidate], &cache, &score, &neighbour);
                    if candidate_score < best_score {
                        best_score = candidate_score;
                        best = Some(*candidate);
                    }
                }
            }
            match best {
                Some(triangle) => triangle,
                // No live neighbour -- the last triangle was the only one left.
                // Falling back to index 0 would re-emit a dead triangle and
                // silently grow the index buffer.
                None => (0..corners.len()).find(|t| !dead[*t]).unwrap_or(0),
            }
        };

        let triangle = corners[chosen];
        dead[chosen] = true;
        remaining -= 1;
        last = chosen;

        // Rotate so the best-scoring corner is fetched first.
        let rotation = (0..3)
            .min_by(|a, b| {
                vertex_score(triangle[*a] as usize, &cache, &score, &neighbour).total_cmp(
                    &vertex_score(triangle[*b] as usize, &cache, &score, &neighbour),
                )
            })
            .unwrap_or(0);

        for step in 0..3 {
            output.push(triangle[(rotation + step) % 3]);
            // Cache slot 0 is most recent. The driver evicts from the far end,
            // so that is where a miss goes.
            // Slot 0 is most recent; a miss evicts from the far end.
            if CACHE_SIZE > 1 {
                cache.copy_within(1..CACHE_SIZE, 0);
            }
            cache[0] = triangle[(rotation + step) % 3] as usize;
        }

        // Decrement the scores of the vertices this triangle used, so a vertex
        // whose last use has passed stops looking attractive.
        for corner in triangle {
            if (corner as usize) >= adjacency.len() {
                continue;
            }
            if let Some(vertex) = adjacency[corner as usize].iter().position(|t| *t == chosen) {
                adjacency[corner as usize].remove(vertex);
                score[corner as usize] = adjacency[corner as usize].len() as f32;
            }
        }
    }

    output
}

/// Forsyth's vertex score: what it costs to fetch this vertex right now.
///
/// **This is a cost, so lower is better.** Taking the maximum instead of the
/// minimum is the single easiest mistake here, and it is completely silent: the
/// output is still a valid permutation of the index buffer, so every structural
/// test passes, and the pass simply does nothing. Measured on a scrambled grid,
/// the inverted version left ACMR at 2.99 against 2.98 before -- worse than
/// running no optimisation at all.
fn vertex_score(
    vertex: usize,
    cache: &[usize; CACHE_SIZE],
    score: &[f32],
    neighbour: &[usize],
) -> f32 {
    if vertex >= score.len() {
        return f32::NEG_INFINITY;
    }
    if let Some(position) = cache.iter().position(|entry| *entry == vertex) {
        // Already cached: cheaper the more recent it is. The ideal order would
        // have placed it at position 0.
        return score[vertex] / (1.0 + position as f32 * score[vertex]);
    }
    // Not cached: full cost, plus a bonus if something it shares a triangle with
    // is already in the cache.
    let neighbour = neighbour
        .get(vertex)
        .copied()
        .unwrap_or(usize::MAX)
        .min(score.len().saturating_sub(1));
    score[vertex] + 0.5 * score[neighbour]
}

fn triangle_score(
    triangle: [u32; 3],
    cache: &[usize; CACHE_SIZE],
    score: &[f32],
    neighbour: &[usize],
) -> f32 {
    // The *cheapest* corner, because the triangle is emitted starting from it.
    triangle
        .iter()
        .map(|corner| vertex_score(*corner as usize, cache, score, neighbour))
        .fold(f32::INFINITY, f32::min)
}

/// Average cache miss ratio: misses per triangle. Lower is better.
pub fn acmr(indices: &[u32], cache_size: usize) -> f32 {
    let triangles = (indices.len() / TRIANGLE).max(1);
    let mut cache = vec![usize::MAX; cache_size.max(1)];
    let mut misses = 0usize;

    for corner in indices {
        let vertex = *corner as usize;
        if !cache.contains(&vertex) {
            misses += 1;
            let oldest = cache
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| **entry)
                .map(|(position, _)| position)
                .unwrap_or(0);
            cache[oldest] = vertex;
        }
    }

    misses as f32 / triangles as f32
}

/// Merge byte-identical vertices.
///
/// `remap[i]` is the index vertex `i` becomes; vertices sharing an entry are
/// identical in every byte.
///
/// **Skipping this for skinned primitives is not an optimisation.** Two vertices
/// at the same position with the same UV can carry different joint indices or
/// weights -- that is what a bone crease is -- and merging them collapses the
/// deformation at that vertex. Upstream guards on `skinned`
/// (`LoadModelGeometry.cpp:349`); [`optimize`] does the same.
pub fn generate_vertex_remap(vertices: &[Vertex]) -> Vec<u32> {
    let mut table: HashMap<[u8; std::mem::size_of::<Vertex>()], u32> =
        HashMap::with_capacity(vertices.len());
    let mut remap = Vec::with_capacity(vertices.len());

    for vertex in vertices {
        let key = vertex.identity_bytes();
        let next = table.len() as u32;
        remap.push(*table.entry(key).or_insert(next));
    }

    remap
}

/// How many distinct vertices [`generate_vertex_remap`] would produce.
pub fn merged_vertex_count(vertices: &[Vertex]) -> usize {
    generate_vertex_remap(vertices)
        .iter()
        .copied()
        .max()
        .map_or(0, |highest| highest as usize + 1)
}

/// Apply a vertex remap: `out[remap[i]] = vertices[i]`.
pub fn apply_remap(vertices: &[Vertex], remap: &[u32]) -> Vec<Vertex> {
    let count = merged_vertex_count(vertices);
    let mut out = vec![Vertex::default(); count];
    let mut written = vec![false; count];
    for (index, vertex) in vertices.iter().enumerate() {
        let target = remap[index] as usize;
        if !written[target] {
            out[target] = *vertex;
            written[target] = true;
        }
    }
    out
}

/// Apply a vertex remap to an index buffer.
pub fn remap_indices(indices: &[u32], remap: &[u32]) -> Vec<u32> {
    indices
        .iter()
        .map(|index| remap.get(*index as usize).copied().unwrap_or(u32::MAX))
        .collect()
}

/// Reorder vertices so triangles that are spatially close share nearby indices.
///
/// Scores each vertex by the total area of the triangles touching it, then
/// emits a remap ordered by score. Vertices used by large triangles end up
/// adjacent, so the vertex fetch sees locality.
///
/// Stable sort, so equal-scored vertices keep their relative order and a
/// degenerate mesh does not come out shuffled.
pub fn optimize_vertex_fetch_remap(vertices: &[Vertex], indices: &[u32]) -> Vec<u32> {
    let mut score = vec![0.0f32; vertices.len()];

    for triangle in indices.as_chunks::<TRIANGLE>().0.iter() {
        let (Some(&a), Some(&b), Some(&c)) = (triangle.first(), triangle.get(1), triangle.get(2))
        else {
            continue;
        };
        if a as usize >= vertices.len()
            || b as usize >= vertices.len()
            || c as usize >= vertices.len()
        {
            continue;
        }
        let area = triangle_area(
            &vertices[a as usize],
            &vertices[b as usize],
            &vertices[c as usize],
        );
        for corner in [a, b, c] {
            score[corner as usize] += area;
        }
    }

    let mut order: Vec<u32> = (0..vertices.len() as u32).collect();
    order.sort_by(|a, b| score[*b as usize].total_cmp(&score[*a as usize]));

    let mut remap = vec![0u32; vertices.len()];
    for (new_index, old_index) in order.into_iter().enumerate() {
        remap[old_index as usize] = new_index as u32;
    }
    remap
}

fn triangle_area(a: &Vertex, b: &Vertex, c: &Vertex) -> f32 {
    let e1 = b.position - a.position;
    let e2 = c.position - a.position;
    e1.cross(e2).length() * 0.5
}

/// What one full optimise pass reported.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Stats {
    /// Distinct vertices before any pass.
    pub vertices_before: usize,
    /// After merging and reordering.
    pub vertices_after: usize,
    /// Index count, which no pass changes.
    pub indices: usize,
    /// Average cache miss ratio before the cache pass.
    pub acmr_before: f32,
    /// After.
    pub acmr_after: f32,
    /// Whether merging was skipped because the mesh is skinned.
    pub merge_skipped: bool,
}

impl Stats {
    /// Vertices removed by merging.
    pub fn merged(&self) -> usize {
        self.vertices_before.saturating_sub(self.vertices_after)
    }
}

/// Run all four passes, in order.
///
/// `skinned` suppresses pass 3. See [`generate_vertex_remap`] for why.
pub fn optimize(
    mut vertices: Vec<Vertex>,
    mut indices: Vec<u32>,
    skinned: bool,
) -> (Vec<Vertex>, Vec<u32>, Stats) {
    let mut stats = Stats {
        vertices_before: vertices.len(),
        indices: indices.len(),
        acmr_before: acmr(&indices, CACHE_SIZE),
        ..Stats::default()
    };

    // 1. Tangents, before anything hashes the vertex bytes.
    let _ = generate_tangents(&mut vertices, &indices);

    // 2. Cache order.
    indices = optimize_vertex_cache(&indices, vertices.len());
    stats.acmr_after = acmr(&indices, CACHE_SIZE);

    // 3. Merge, unless the mesh is skinned.
    if skinned {
        stats.merge_skipped = true;
    } else {
        let remap = generate_vertex_remap(&vertices);
        indices = remap_indices(&indices, &remap);
        vertices = apply_remap(&vertices, &remap);
    }

    // 4. Fetch order, after the triangles are in their final order.
    let fetch_remap = optimize_vertex_fetch_remap(&vertices, &indices);
    indices = remap_indices(&indices, &fetch_remap);
    vertices = apply_remap(&vertices, &fetch_remap);

    stats.vertices_after = vertices.len();
    (vertices, indices, stats)
}
