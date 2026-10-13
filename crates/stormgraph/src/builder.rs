use crate::resources::Resources;
use crate::types::{
    ColorAttachment, DepthAttachment, ImageState, LoadOp, PassDesc, PassType, Queue,
    ResourceAccess, TextureId, ValidationError,
};

/// The graph, as declared. Declaration only: no GPU work, no closures.
///
/// Built fresh every frame and recompiled every frame. There is deliberately no
/// compile cache — at realistic pass counts the compile is microseconds, and a
/// structural-hash cache is complexity with no measurable payoff.
pub struct RenderGraph {
    resources: Resources,
    passes: Vec<PassDesc>,
}

impl RenderGraph {
    pub fn new() -> Self {
        Self {
            resources: Resources::new(),
            passes: Vec::new(),
        }
    }

    pub fn resources(&self) -> &Resources {
        &self.resources
    }

    pub fn passes(&self) -> &[PassDesc] {
        &self.passes
    }

    /// A transient texture: created and destroyed by the frame.
    pub fn create_texture(
        &mut self,
        name: impl Into<String>,
        initial_state: ImageState,
        final_state: ImageState,
    ) -> TextureId {
        self.resources
            .create_texture(name, initial_state, final_state)
    }

    /// An external texture the graph does not own. Imports are the culling
    /// roots: a pass that never touches one contributes nothing to the frame.
    pub fn import_texture(
        &mut self,
        name: impl Into<String>,
        initial_state: ImageState,
        final_state: ImageState,
    ) -> TextureId {
        self.resources.import_texture(name, initial_state, final_state)
    }

    pub fn create_buffer(&mut self, name: impl Into<String>) -> crate::types::BufferId {
        self.resources.create_buffer(name)
    }

    pub fn import_buffer(&mut self, name: impl Into<String>) -> crate::types::BufferId {
        self.resources.import_buffer(name)
    }

    /// Opens a pass. Returns immediately; drop the builder to close the pass.
    pub fn pass(&mut self, name: impl Into<String>) -> PassBuilder<'_> {
        let index = self.passes.len();
        self.passes.push(PassDesc {
            name: name.into(),
            kind: PassType::Graphics,
            queue: Queue::Graphics,
            colors: Vec::new(),
            depth: None,
            reads: Vec::new(),
            writes: Vec::new(),
        });
        PassBuilder {
            graph: self,
            index,
        }
    }

    pub fn validate(&self) -> Vec<ValidationError> {
        crate::compiler::validate(&self.passes, &self.resources)
    }

    /// Validates, orders and barrier-synthesises. Returns the compiled frame or
    /// every error found — validation collects all of them before failing, so one
    /// run reports every mistake rather than the first.
    pub fn compile(&self) -> Result<crate::compiler::CompiledGraph, Vec<ValidationError>> {
        crate::compiler::compile(&self.passes, &self.resources)
    }

    /// Human-readable dump of the raw graph: resources with version counts,
    /// passes in declaration order with their reads and writes.
    pub fn dump(&self) -> String {
        let mut out = String::new();

        out.push_str("textures:\n");
        for (i, slot) in self.resources.textures().iter().enumerate() {
            out.push_str(&format!(
                "  [{i}] {} imported={} versions={} {} -> {}\n",
                slot.name,
                slot.imported,
                slot.version_log.len(),
                state_name(slot.initial_state),
                state_name(slot.final_state),
            ));
        }

        out.push_str("buffers:\n");
        for (i, slot) in self.resources.buffers().iter().enumerate() {
            out.push_str(&format!(
                "  [{i}] {} imported={} versions={}\n",
                slot.name,
                slot.imported,
                slot.version_log.len(),
            ));
        }

        out.push_str("passes:\n");
        for (i, pass) in self.passes.iter().enumerate() {
            out.push_str(&format!(
                "  [{i}] {} queue={:?} colors={} depth={} reads={} writes={}\n",
                pass.name,
                pass.queue,
                pass.colors.len(),
                pass.depth.is_some(),
                pass.reads.len(),
                pass.writes.len(),
            ));
            for access in &pass.reads {
                out.push_str(&format!(
                    "        R {}\n",
                    describe(access, &self.resources)
                ));
            }
            for access in &pass.writes {
                out.push_str(&format!(
                    "        W {}\n",
                    describe(access, &self.resources)
                ));
            }
        }

        out
    }

    pub fn reset(&mut self) {
        self.resources = Resources::new();
        self.passes.clear();
    }
}

impl Default for RenderGraph {
    fn default() -> Self {
        Self::new()
    }
}

fn describe(access: &ResourceAccess, resources: &Resources) -> String {
    let name = match access {
        ResourceAccess::Texture(id, _) => resources.texture_name(*id),
        ResourceAccess::Buffer(id, _) => resources.buffer_name(*id),
    };
    access.label(name)
}

fn state_name(state: ImageState) -> &'static str {
    match state {
        ImageState::Undefined => "undefined",
        ImageState::ColorAttachment => "color",
        ImageState::DepthAttachment => "depth",
        ImageState::ShaderRead => "shader-read",
        ImageState::TransferSrc => "transfer-src",
        ImageState::TransferDst => "transfer-dst",
        ImageState::Present => "present",
        ImageState::Storage => "storage",
    }
}

/// Builds one pass. Holds `&mut RenderGraph` for its lifetime, so it cannot
/// outlive the graph and cannot be used to open a second pass.
pub struct PassBuilder<'a> {
    graph: &'a mut RenderGraph,
    index: usize,
}

impl PassBuilder<'_> {
    pub fn kind(self, kind: PassType) -> Self {
        self.graph.passes[self.index].kind = kind;
        self
    }

    pub fn queue(self, queue: Queue) -> Self {
        self.graph.passes[self.index].queue = queue;
        self
    }

    /// Declares a colour target. Implies a write in `ColorAttachment` state, so
    /// the caller does not restate it.
    pub fn color(mut self, texture: TextureId, clear: [f32; 4]) -> Self {
        self.write_texture_with(texture, ImageState::ColorAttachment);
        self.graph.passes[self.index].colors.push(ColorAttachment {
            texture,
            load: LoadOp::Clear,
            clear,
            resolve: crate::types::NO_RESOURCE,
        });
        self
    }

    /// Declares a colour target whose previous contents must be preserved.
    pub fn color_load(mut self, texture: TextureId) -> Self {
        self.write_texture_with(texture, ImageState::ColorAttachment);
        self.graph.passes[self.index].colors.push(ColorAttachment {
            texture,
            load: LoadOp::Load,
            clear: [0.0; 4],
            resolve: crate::types::NO_RESOURCE,
        });
        self
    }

    pub fn depth(mut self, texture: TextureId, clear_depth: f32) -> Self {
        self.write_texture_with(texture, ImageState::DepthAttachment);
        self.graph.passes[self.index].depth = Some(DepthAttachment {
            texture,
            load: LoadOp::Clear,
            clear_depth,
        });
        self
    }

    /// Reads a texture in `ShaderRead` state, bound to the version current at
    /// declaration time.
    pub fn read_texture(self, texture: TextureId) -> Self {
        let version = self.graph.resources.latest_texture_version(texture);
        self.graph.passes[self.index]
            .reads
            .push(ResourceAccess::Texture(texture, version));
        self
    }

    /// Writes a texture in an explicit state. Use for non-attachment writes:
    /// `Storage` for an image, `TransferDst` for an upload target.
    pub fn write_texture(mut self, texture: TextureId, state: ImageState) -> Self {
        self.write_texture_with(texture, state);
        self
    }

    pub fn read_buffer(self, buffer: crate::types::BufferId) -> Self {
        let version = self.graph.resources.latest_buffer_version(buffer);
        self.graph.passes[self.index]
            .reads
            .push(ResourceAccess::Buffer(buffer, version));
        self
    }

    pub fn write_buffer(self, buffer: crate::types::BufferId) -> Self {
        let previous = self.graph.resources.latest_buffer_version(buffer);
        self.graph
            .resources
            .push_version(ResourceAccess::Buffer(buffer, previous), self.index as u32, ImageState::Storage);
        self.graph.passes[self.index]
            .writes
            .push(ResourceAccess::Buffer(buffer, previous + 1));
        self
    }

    /// Bumps the resource version and records the write. A texture written
    /// twice in one frame becomes two versions, which is what lets the compiler
    /// order them instead of collapsing them.
    fn write_texture_with(&mut self, texture: TextureId, state: ImageState) {
        let previous = self.graph.resources.latest_texture_version(texture);
        self.graph.resources.push_version(
            ResourceAccess::Texture(texture, previous),
            self.index as u32,
            state,
        );
        self.graph.passes[self.index]
            .writes
            .push(ResourceAccess::Texture(texture, previous + 1));
    }
}