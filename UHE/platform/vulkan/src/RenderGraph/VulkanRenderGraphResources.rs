use crate::vulkan_render_graph_types::{BufferId, ImageState, TextureId};

/// One version of a resource: which pass produced it, and the state it left it
/// in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VersionEntry {
    pub version: u32,
    /// Declaration index of the producing pass, or `u32::MAX` for the version
    /// that exists before the graph runs (an import).
    pub producer: u32,
    pub state: ImageState,
}

#[derive(Clone, PartialEq, Debug)]
pub struct TextureSlot {
    pub name: String,
    /// Imported resources are the graph's roots: their producers are unknown, so
    /// they keep their declared state and culling hangs off them.
    pub imported: bool,
    pub initial_state: ImageState,
    pub final_state: ImageState,
    /// Oldest versions, trimmed once a version can no longer be read.
    pub version_log: Vec<VersionEntry>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct BufferSlot {
    pub name: String,
    pub imported: bool,
    pub version_log: Vec<VersionEntry>,
}

/// The graph's resource table and version log.
///
/// Versioning is the whole trick: a write bumps a resource's version, so two
/// writes of the same texture in one frame are two distinct resources and the
/// compiler can order them. That is what makes read/write order an edge set.
#[derive(Clone, Default, Debug)]
pub struct Resources {
    textures: Vec<TextureSlot>,
    buffers: Vec<BufferSlot>,
}

impl Resources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_texture(
        &mut self,
        name: impl Into<String>,
        initial_state: ImageState,
        final_state: ImageState,
    ) -> TextureId {
        self.textures.push(TextureSlot {
            name: name.into(),
            imported: false,
            initial_state,
            final_state,
            // Version 0 exists before the frame runs.
            version_log: vec![VersionEntry {
                version: 0,
                producer: u32::MAX,
                state: initial_state,
            }],
        });
        (self.textures.len() - 1) as TextureId
    }

    /// An external texture the graph does not own.
    ///
    /// `initial_state` and `final_state` differ for the swapchain image, which
    /// enters the frame undefined and has to leave in `Present`. Anything with
    /// equal states needs no boundary transition.
    pub fn import_texture(
        &mut self,
        name: impl Into<String>,
        initial_state: ImageState,
        final_state: ImageState,
    ) -> TextureId {
        self.textures.push(TextureSlot {
            name: name.into(),
            imported: true,
            initial_state,
            final_state,
            version_log: vec![VersionEntry {
                version: 0,
                producer: u32::MAX,
                state: initial_state,
            }],
        });
        (self.textures.len() - 1) as TextureId
    }

    pub fn create_buffer(&mut self, name: impl Into<String>) -> BufferId {
        self.buffers.push(BufferSlot {
            name: name.into(),
            imported: false,
            version_log: vec![VersionEntry {
                version: 0,
                producer: u32::MAX,
                state: ImageState::Undefined,
            }],
        });
        (self.buffers.len() - 1) as BufferId
    }

    pub fn import_buffer(&mut self, name: impl Into<String>) -> BufferId {
        self.buffers.push(BufferSlot {
            name: name.into(),
            imported: true,
            version_log: vec![VersionEntry {
                version: 0,
                producer: u32::MAX,
                state: ImageState::Undefined,
            }],
        });
        (self.buffers.len() - 1) as BufferId
    }

    pub fn texture(&self, id: TextureId) -> Option<&TextureSlot> {
        self.textures.get(id as usize)
    }

    pub fn buffer(&self, id: BufferId) -> Option<&BufferSlot> {
        self.buffers.get(id as usize)
    }

    pub fn textures(&self) -> &[TextureSlot] {
        &self.textures
    }

    pub fn buffers(&self) -> &[BufferSlot] {
        &self.buffers
    }

    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    pub fn texture_name(&self, id: TextureId) -> &str {
        self.textures
            .get(id as usize)
            .map_or("<unknown>", |t| t.name.as_str())
    }

    pub fn buffer_name(&self, id: BufferId) -> &str {
        self.buffers
            .get(id as usize)
            .map_or("<unknown>", |b| b.name.as_str())
    }

    /// Appends a version produced by `producer`.
    pub fn push_version(
        &mut self,
        access: crate::vulkan_render_graph_types::ResourceAccess,
        producer: u32,
        state: ImageState,
    ) {
        let version = self.version(access) + 1;
        let entry = VersionEntry {
            version,
            producer,
            state,
        };
        match access {
            crate::vulkan_render_graph_types::ResourceAccess::Texture(id, _) => {
                if let Some(slot) = self.textures.get_mut(id as usize) {
                    slot.version_log.push(entry);
                }
            }
            crate::vulkan_render_graph_types::ResourceAccess::Buffer(id, _) => {
                if let Some(slot) = self.buffers.get_mut(id as usize) {
                    slot.version_log.push(entry);
                }
            }
        }
    }

    /// The current version of a resource, as bound at declaration time.
    pub fn version(&self, access: crate::vulkan_render_graph_types::ResourceAccess) -> u32 {
        match access {
            crate::vulkan_render_graph_types::ResourceAccess::Texture(id, version) => self
                .textures
                .get(id as usize)
                .and_then(|t| t.version_log.get(version as usize))
                .map_or(0, |v| v.version),
            crate::vulkan_render_graph_types::ResourceAccess::Buffer(id, version) => self
                .buffers
                .get(id as usize)
                .and_then(|b| b.version_log.get(version as usize))
                .map_or(0, |v| v.version),
        }
    }

    /// State a given version leaves its resource in.
    pub fn version_state(
        &self,
        access: crate::vulkan_render_graph_types::ResourceAccess,
    ) -> Option<ImageState> {
        match access {
            crate::vulkan_render_graph_types::ResourceAccess::Texture(id, version) => self
                .textures
                .get(id as usize)
                .and_then(|t| t.version_log.get(version as usize))
                .map(|v| v.state),
            crate::vulkan_render_graph_types::ResourceAccess::Buffer(..) => None,
        }
    }

    /// Which pass produced a given version, or `None` for the pre-frame one.
    pub fn producer(
        &self,
        access: crate::vulkan_render_graph_types::ResourceAccess,
    ) -> Option<u32> {
        let producer = match access {
            crate::vulkan_render_graph_types::ResourceAccess::Texture(id, version) => self
                .textures
                .get(id as usize)
                .and_then(|t| t.version_log.get(version as usize))
                .map(|v| v.producer),
            crate::vulkan_render_graph_types::ResourceAccess::Buffer(id, version) => self
                .buffers
                .get(id as usize)
                .and_then(|b| b.version_log.get(version as usize))
                .map(|v| v.producer),
        }?;
        (producer != u32::MAX).then_some(producer)
    }

    /// Highest version declared so far for a resource.
    pub fn latest_version(&self, access: crate::vulkan_render_graph_types::ResourceAccess) -> u32 {
        let log = match access {
            crate::vulkan_render_graph_types::ResourceAccess::Texture(id, _) => {
                self.textures.get(id as usize).map(|t| &t.version_log)
            }
            crate::vulkan_render_graph_types::ResourceAccess::Buffer(id, _) => {
                self.buffers.get(id as usize).map(|b| &b.version_log)
            }
        };
        log.and_then(|log| log.last()).map_or(0, |v| v.version)
    }
}
impl Resources {
    pub fn latest_texture_version(&self, id: TextureId) -> u32 {
        self.textures
            .get(id as usize)
            .and_then(|t| t.version_log.last())
            .map_or(0, |v| v.version)
    }

    pub fn latest_buffer_version(&self, id: BufferId) -> u32 {
        self.buffers
            .get(id as usize)
            .and_then(|b| b.version_log.last())
            .map_or(0, |v| v.version)
    }
}
