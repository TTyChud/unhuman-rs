use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::barrier::{transition, ImageBarrier};
use crate::resources::Resources;
use crate::types::{
    ColorAttachment, DepthAttachment, ErrorKind, ImageState, PassDesc, PassType, Queue,
    ResourceAccess, TextureId, ValidationError,
};

/// A pass, ordered and with its barriers resolved. Pure data: no closures, no
/// backend types. The backend looks the recording function up by `index`.
#[derive(Clone, PartialEq, Debug)]
pub struct CompiledPass {
    /// Declaration index, which is how the backend finds the recording closure.
    pub index: u32,
    pub name: String,
    pub kind: PassType,
    pub queue: Queue,
    pub colors: Vec<ColorAttachment>,
    pub depth: Option<DepthAttachment>,
    pub pre_barriers: Vec<ImageBarrier>,
    pub post_barriers: Vec<ImageBarrier>,
}

/// First and last compiled pass that touches a resource version. Drives
/// transient resource reuse and, later, aliasing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ResourceLifetime {
    pub texture: TextureId,
    pub version: u32,
    pub first_use: u32,
    pub last_use: u32,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct CompiledGraph {
    pub passes: Vec<CompiledPass>,
    pub lifetimes: Vec<ResourceLifetime>,
}

impl CompiledGraph {
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }

    pub fn find(&self, name: &str) -> Option<&CompiledPass> {
        self.passes.iter().find(|p| p.name == name)
    }
}

/// Collects every error before failing, so one run reports all of them.
pub fn validate(passes: &[PassDesc], resources: &Resources) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for pass in passes {
        if pass.name.is_empty() {
            errors.push(ValidationError::new(
                ErrorKind::UnsubmittedPass,
                "pass was opened but never named",
            ));
            continue;
        }

        // Every attachment must also be declared as a write, otherwise the
        // compiler has no idea what state it lands in.
        let written: Vec<ResourceAccess> = pass.writes.clone();
        for color in &pass.colors {
            if !passes_color(pass, color.texture) {
                errors.push(
                    ValidationError::new(
                        ErrorKind::UndeclaredAttachment,
                        "colour attachment was not declared with a write",
                    )
                    .in_pass(&pass.name)
                    .on_resource(resources.texture_name(color.texture)),
                );
            }
        }
        if let Some(depth) = &pass.depth {
            let declared = pass.writes.iter().any(|w| {
                matches!(w, ResourceAccess::Texture(id, _) if *id == depth.texture)
            });
            if !declared {
                errors.push(
                    ValidationError::new(
                        ErrorKind::UndeclaredAttachment,
                        "depth attachment was not declared with a write",
                    )
                    .in_pass(&pass.name)
                    .on_resource(resources.texture_name(depth.texture)),
                );
            }
        }

        // Double write. Compared on the resource slot, not the
        // (slot, version) pair: each write allocates a fresh version, so
        // comparing versions would never match and the error would be
        // unreachable.
        for (i, a) in written.iter().enumerate() {
            if written[i + 1..].iter().any(|b| b.slot() == a.slot()) {
                errors.push(
                    ValidationError::new(ErrorKind::DoubleWrite, "resource written twice by one pass")
                        .in_pass(&pass.name)
                        .on_resource(resource_name(a, resources)),
                );
            }
        }

        // Read at a version no pass ever produces.
        for read in &pass.reads {
            let known = match read {
                ResourceAccess::Texture(id, version) => resources.texture(*id).is_some_and(|slot| {
                    (*version as usize) < slot.version_log.len()
                }),
                ResourceAccess::Buffer(id, version) => resources.buffer(*id).is_some_and(|slot| {
                    (*version as usize) < slot.version_log.len()
                }),
            };
            if !known {
                errors.push(
                    ValidationError::new(
                        ErrorKind::ReadBeforeWrite,
                        "read at a version that does not exist",
                    )
                    .in_pass(&pass.name)
                    .on_resource(read.label(resource_name(read, resources))),
                );
            }
        }

        // Read and write the same texture in one pass is ambiguous here: the
        // pass cannot see which version the read binds to once its own write
        // lands. Matches the C++ `PassStateConflict`.
        for read in &pass.reads {
            if let ResourceAccess::Texture(id, _) = read {
                if written
                    .iter()
                    .any(|w| matches!(w, ResourceAccess::Texture(wid, _) if wid == id))
                {
                    errors.push(
                        ValidationError::new(
                            ErrorKind::PassStateConflict,
                            "one pass reads and writes the same texture",
                        )
                        .in_pass(&pass.name)
                        .on_resource(resources.texture_name(*id)),
                    );
                }
            }
        }
    }

    errors
}

fn passes_color(pass: &PassDesc, texture: TextureId) -> bool {
    pass.writes
        .iter()
        .any(|w| matches!(w, ResourceAccess::Texture(id, _) if *id == texture))
}

fn resource_name<'a>(access: &ResourceAccess, resources: &'a Resources) -> &'a str {
    match access {
        ResourceAccess::Texture(id, _) => resources.texture_name(*id),
        ResourceAccess::Buffer(id, _) => resources.buffer_name(*id),
    }
}

pub fn compile(
    passes: &[PassDesc],
    resources: &Resources,
) -> Result<CompiledGraph, Vec<ValidationError>> {
    let errors = validate(passes, resources);
    if !errors.is_empty() {
        return Err(errors);
    }

    let order = topological_order(passes, resources)?;
    let order = cull(passes, resources, order);

    let (compiled, lifetimes) = synthesize_barriers(passes, resources, &order);

    Ok(CompiledGraph {
        passes: compiled,
        lifetimes,
    })
}

/// Kahn's algorithm over version edges.
///
/// Because writes are bound to the version current at declaration time, the
/// declaration order is *already* topological. The sort verifies that and gives
/// the rest of the compiler a real edge set to work from. Ties break toward
/// declaration order, so a single-queue frame compiles to exactly the declared
/// sequence.
fn topological_order(
    passes: &[PassDesc],
    resources: &Resources,
) -> Result<Vec<usize>, Vec<ValidationError>> {
    let mut indegree = vec![0usize; passes.len()];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); passes.len()];

    for (p, pass) in passes.iter().enumerate() {
        for read in &pass.reads {
            let Some(producer) = resources.producer(*read) else {
                continue; // imported at v0, or a self read
            };
            if producer as usize == p {
                continue;
            }
            dependents[producer as usize].push(p);
            indegree[p] += 1;
        }
    }

    let mut ready: BinaryHeap<Reverse<usize>> = passes
        .iter()
        .enumerate()
        .filter(|(i, _)| indegree[*i] == 0)
        .map(|(i, _)| Reverse(i))
        .collect();

    let mut order = Vec::with_capacity(passes.len());
    while let Some(Reverse(p)) = ready.pop() {
        order.push(p);
        for &dependent in &dependents[p] {
            indegree[dependent] -= 1;
            if indegree[dependent] == 0 {
                ready.push(Reverse(dependent));
            }
        }
    }

    if order.len() != passes.len() {
        return Err(vec![ValidationError::new(
            ErrorKind::Cycle,
            "pass dependency cycle (Kahn's queue emptied early)",
        )]);
    }

    Ok(order)
}

/// Drops passes that cannot reach an imported resource.
///
/// Roots are passes touching an import, read or written. A pass survives if a
/// root transitively depends on it — producer edges only, because dropping the
/// consumers of a culled pass is the entire point. A frame with no imports at
/// all is therefore empty, which is correct: nothing observable is produced.
fn cull(passes: &[PassDesc], resources: &Resources, order: Vec<usize>) -> Vec<usize> {
    let touches_import = |p: usize| -> bool {
        let pass = &passes[p];
        pass.reads.iter().chain(pass.writes.iter()).any(|access| match access {
            ResourceAccess::Texture(id, _) => {
                resources.texture(*id).is_some_and(|slot| slot.imported)
            }
            ResourceAccess::Buffer(id, _) => {
                resources.buffer(*id).is_some_and(|slot| slot.imported)
            }
        })
    };

    let mut live = vec![false; passes.len()];
    for (p, is_live) in live.iter_mut().enumerate() {
        if touches_import(p) {
            *is_live = true;
        }
    }

    // Walk backwards from the roots over producer edges.
    for p in order.iter().rev() {
        if live[*p] {
            for read in &passes[*p].reads {
                if let Some(producer) = resources.producer(*read) {
                    live[producer as usize] = true;
                }
            }
        }
    }

    order.into_iter().filter(|p| live[*p]).collect()
}

/// Walks the frame in execution order and emits one barrier per state change.
///
/// **Pre-barriers only.** Each pass owns the transitions for everything it
/// touches, recorded into its own command list, so a list is self-contained and
/// passes can record in parallel with no ordering constraint between them. That
/// is what the C++ executor's pre/post barrier pair around a *range* of passes
/// cannot do — with one shared command buffer the range boundary is the only
/// place a barrier can go. The single exception is the final transition of an
/// imported resource, which has to follow the last pass that touches it; that
/// becomes that pass's post-barrier.
fn synthesize_barriers(
    passes: &[PassDesc],
    resources: &Resources,
    order: &[usize],
) -> (Vec<CompiledPass>, Vec<ResourceLifetime>) {
    // Current known state per texture, starting from every resource's entry
    // state. Transients start Undefined, so their first transition is free.
    let mut state: Vec<ImageState> = (0..resources.texture_count())
        .map(|i| resources.texture(i as TextureId).map_or(ImageState::Undefined, |t| t.initial_state))
        .collect();

    let mut first_use: Vec<Option<u32>> = vec![None; resources.texture_count()];
    let mut last_use: Vec<Option<u32>> = vec![None; resources.texture_count()];
    let mut lifetime_index: Vec<Vec<u32>> = vec![Vec::new(); resources.texture_count()];

    let mut compiled = Vec::with_capacity(order.len());
    let mut last_writer: Vec<Option<usize>> = vec![None; resources.texture_count()];

    for (position, &p) in order.iter().enumerate() {
        let pass = &passes[p];
        let mut pre_barriers = Vec::new();

        for access in pass.reads.iter().chain(pass.writes.iter()) {
            let ResourceAccess::Texture(id, version) = access else {
                continue;
            };
            let id = *id as usize;

            if first_use[id].is_none() {
                first_use[id] = Some(position as u32);
            }
            last_use[id] = Some(position as u32);
            lifetime_index[id].push(*version);
        }

        // Reads settle into ShaderRead.
        for access in &pass.reads {
            let ResourceAccess::Texture(id, _) = access else {
                continue;
            };
            let index = *id as usize;
            if let Some(barrier) = transition(*id, state[index], ImageState::ShaderRead) {
                pre_barriers.push(barrier);
            }
            state[index] = ImageState::ShaderRead;
        }

        // Writes settle into the state they were declared with, which is how a
        // colour attachment and a storage write stay distinguishable.
        for access in &pass.writes {
            let ResourceAccess::Texture(id, _) = access else {
                continue;
            };
            let index = *id as usize;
            let target = resources
                .version_state(*access)
                .unwrap_or(ImageState::Storage);
            if let Some(barrier) = transition(*id, state[index], target) {
                pre_barriers.push(barrier);
            }
            state[index] = target;
            last_writer[index] = Some(compiled.len());
        }

        compiled.push(CompiledPass {
            index: p as u32,
            name: pass.name.clone(),
            kind: pass.kind,
            queue: pass.queue,
            colors: pass.colors.clone(),
            depth: pass.depth,
            pre_barriers,
            post_barriers: Vec::new(),
        });
    }

    // Final transition of each imported resource. A swapchain image, for
    // instance, has to reach `Present` after the last pass that writes it.
    for id in 0..resources.texture_count() {
        let Some(slot) = resources.texture(id as TextureId) else {
            continue;
        };
        if !slot.imported || slot.final_state == slot.initial_state {
            continue;
        }
        let Some(writer) = last_writer[id] else {
            continue;
        };
        let current = state[id];
        if let Some(barrier) = transition(id as TextureId, current, slot.final_state) {
            compiled[writer].post_barriers.push(barrier);
        }
    }

    let mut lifetimes = Vec::new();
    for (id, index) in lifetime_index.iter().enumerate() {
        let (Some(first), Some(last)) = (first_use[id], last_use[id]) else {
            continue;
        };
        for version in index {
            lifetimes.push(ResourceLifetime {
                texture: id as TextureId,
                version: *version,
                first_use: first,
                last_use: last,
            });
        }
    }

    (compiled, lifetimes)
}