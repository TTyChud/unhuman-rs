//! Test doubles. No device, no driver, no window.
//!
//! ## Why a recording command list and not a fake backend
//!
//! [`chassis::CommandList`] is a trait with no device behind it: it is handed a
//! list, not a queue. So the whole of `bifrost`'s replay path — which binds were
//! skipped, which were reissued, in what order — is testable with a struct that
//! appends to a `Vec`.
//!
//! What cannot be tested here is anything about a driver, and nothing in this
//! crate claims to. The per-backend proof that matters is Phase 1's: the same
//! frame recorded by both backends producing the same image. That needs a device,
//! so it does not live in this directory and its absence is not a gap in these
//! tests — it is a gap in the crate, and `session.md` lists it.
//!
//! ## Handles, for tests that need them to differ
//!
//! `chassis::Handle::new` is `pub(crate)`, so a test cannot mint an arbitrary
//! pipeline handle. It *can* use `chassis::Registry`, which is public and whose
//! `create` is the only sanctioned way to make one — the same way a backend does
//! it. So "pipeline 0 and pipeline 1 are different" is expressible here without
//! weakening anything in `bifrost` for the sake of a test.

use std::collections::HashMap;

use bifrost::drawlist::{MaterialId, MeshId, PipelineId};
use bifrost::replay::DrawResolver;
use chassis::{
    BindGroup, BindGroupTag, Buffer, BufferTag, CommandList, Extent2d, Pipeline, PipelineTag,
    RenderPassDesc, Texture,
};

/// A command list that records what it was asked to do.
#[derive(Default)]
pub struct RecordingList {
    pub scopes_opened: u32,
    pub pipelines: Vec<Pipeline>,
    pub groups: Vec<(u32, BindGroup)>,
    pub vertex_buffers: Vec<(Buffer, u64)>,
    pub index_buffers: Vec<(Buffer, u64)>,
    pub push_constants: u32,
    /// `(indices, instances)` per draw.
    pub draws: Vec<(u32, u32)>,
    pub dispatches: Vec<(u32, u32, u32)>,
}

impl RecordingList {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CommandList for RecordingList {
    fn begin_rendering(&mut self, _desc: &RenderPassDesc) {
        self.scopes_opened += 1;
    }

    fn end_rendering(&mut self) {}

    fn set_viewport(&mut self, _extent: Extent2d) {}

    fn set_scissor(&mut self, _x: u32, _y: u32, _width: u32, _height: u32) {}

    fn bind_pipeline(&mut self, pipeline: &Pipeline) {
        self.pipelines.push(*pipeline);
    }

    fn bind_group(&mut self, set: u32, group: &BindGroup) {
        self.groups.push((set, *group));
    }

    fn set_push_constants(&mut self, _data: &[u8]) {
        self.push_constants += 1;
    }

    fn bind_vertex_buffer(&mut self, _slot: u32, buffer: &Buffer, offset: u64) {
        self.vertex_buffers.push((*buffer, offset));
    }

    fn bind_index_buffer(&mut self, buffer: &Buffer, offset: u64) {
        self.index_buffers.push((*buffer, offset));
    }

    fn draw(&mut self, _vertices: u32, _instances: u32) {}

    fn draw_indexed(&mut self, indices: u32, instances: u32) {
        self.draws.push((indices, instances));
    }

    fn dispatch(&mut self, groups_x: u32, groups_y: u32, groups_z: u32) {
        self.dispatches.push((groups_x, groups_y, groups_z));
    }

    fn copy_buffer(&mut self, _src: &Buffer, _dst: &Buffer) {}

    fn copy_texture_to_buffer(&mut self, _src: &Texture, _dst: &Buffer) {}
}

/// Handles for tests that need several distinct ones.
pub struct Handles {
    registry: chassis::Registry<u32, PipelineTag>,
    groups: chassis::Registry<u32, BindGroupTag>,
    buffers: chassis::Registry<u32, BufferTag>,
}

impl Handles {
    pub fn new() -> Self {
        Self {
            registry: chassis::Registry::new(),
            groups: chassis::Registry::new(),
            buffers: chassis::Registry::new(),
        }
    }

    /// A distinct pipeline handle per id.
    pub fn pipeline(&mut self, id: u32) -> Pipeline {
        // `create` mints a new slot; asking twice for the same id would mint two
        // handles, so ids are consumed in call order and each test asks for
        // exactly as many as it needs.
        let _ = id;
        self.registry.create(0)
    }

    pub fn bind_group(&mut self) -> BindGroup {
        self.groups.create(0)
    }

    pub fn buffer(&mut self) -> Buffer {
        self.buffers.create(0)
    }
}

impl Default for Handles {
    fn default() -> Self {
        Self::new()
    }
}

/// One mesh's worth of resolvable state.
pub struct TestMesh {
    pub vertex: Buffer,
    pub vertex_offset: u64,
    pub index: Buffer,
    pub index_offset: u64,
    pub indices: u32,
}

/// Resolves ids to real, distinct handles.
pub struct TestResolver {
    pub handles: Handles,
    pub pipelines: HashMap<PipelineId, Pipeline>,
    pub groups: HashMap<MaterialId, BindGroup>,
    pub meshes: HashMap<MeshId, TestMesh>,
    /// Ids that resolve to nothing, so a test can prove a missing handle is an
    /// error rather than a skipped draw.
    #[allow(dead_code)]
    pub missing: Vec<PipelineId>,
}

impl TestResolver {
    pub fn new() -> Self {
        Self {
            handles: Handles::new(),
            pipelines: HashMap::new(),
            groups: HashMap::new(),
            meshes: HashMap::new(),
            missing: Vec::new(),
        }
    }

    pub fn add_pipeline(&mut self, id: PipelineId) {
        let handle = self.handles.pipeline(id.0);
        self.pipelines.insert(id, handle);
    }

    pub fn add_material(&mut self, id: MaterialId) {
        let handle = self.handles.bind_group();
        self.groups.insert(id, handle);
    }

    pub fn add_mesh(&mut self, id: MeshId, indices: u32) {
        let vertex = self.handles.buffer();
        let index = self.handles.buffer();
        self.meshes.insert(
            id,
            TestMesh {
                vertex,
                // Offsets differ per mesh so the tracker's offset comparison is
                // exercised: two meshes in one merged buffer differ *only* by
                // offset, and a tracker that compared handles alone would skip the
                // second bind.
                vertex_offset: u64::from(id.0) * 4096,
                index,
                index_offset: u64::from(id.0) * 2048,
                indices,
            },
        );
    }
}

impl Default for TestResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl DrawResolver for TestResolver {
    fn pipeline(&self, id: PipelineId) -> Option<Pipeline> {
        self.pipelines.get(&id).copied()
    }

    fn bind_group(&self, id: MaterialId) -> Option<BindGroup> {
        self.groups.get(&id).copied()
    }

    fn vertex_buffer(&self, mesh: MeshId) -> Option<(Buffer, u64)> {
        self.meshes.get(&mesh).map(|m| (m.vertex, m.vertex_offset))
    }

    fn index_buffer(&self, mesh: MeshId) -> Option<(Buffer, u64)> {
        self.meshes.get(&mesh).map(|m| (m.index, m.index_offset))
    }

    fn index_count(&self, mesh: MeshId) -> u32 {
        self.meshes.get(&mesh).map_or(0, |m| m.indices)
    }
}
