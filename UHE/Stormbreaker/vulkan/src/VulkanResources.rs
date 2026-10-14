//! The Vulkan implementation of `chassis::Resources`.
//!
//! ## Why this file is the important one
//!
//! It is the seam a second backend slots into. Everything above it -- the
//! renderer, the editor, `bifrost`, whatever comes after -- creates resources
//! through `chassis::Resources` and never names Vulkan. A wgpu implementation of
//! this trait is a different type in the same slot, chosen at startup, with
//! nothing above it changed.
//!
//! So the whole argument for a two-backend engine reduces to: is this trait
//! complete enough that implementing it twice is honest? A method that quietly
//! cannot be expressed on wgpu -- because wgpu has no equivalent of a per-handle
//! image view, say -- is a hole in the seam, and it is better to find it here
//! than after a renderer is written against it.
//!
//! ## What is deliberately absent
//!
//! No rendering policy. Materials, meshes and passes are the renderer's; this
//! owns device objects and nothing else.
//!
//! [`create_bind_group`] is not implemented. A bind group needs a declared set
//! layout, and nothing declares one yet. It returns an error that says so rather
//! than handing back a handle that binds nothing -- a bind group that silently
//! binds nothing renders the wrong image and is close to undebuggable.

use ash::vk;
use chassis::Registry;

use crate::vulkan_buffer::Buffers;
use crate::vulkan_descriptor_manager::Descriptors;
use crate::vulkan_error::BackendError;
use crate::vulkan_pipeline_state::Pipelines;
use crate::vulkan_shader::Shaders;
use crate::vulkan_texture::Textures;

/// Owns every device object the RHI creates.
pub struct VulkanResources {
    device: ash::Device,
    buffers: Buffers,
    textures: Textures,
    shaders: Shaders,
    pipelines: Pipelines,
    descriptors: Descriptors,
    /// Handles for the views `create_image_view` hands out.
    ///
    /// The RHI returns a `chassis::ImageView`, and the driver object behind it
    /// is destroyed with the texture. That is safe as long as nobody uses a view
    /// after its texture, which nothing checks -- so the registry exists at least
    /// to make the pairing explicit and to give the error a handle to name.
    image_views: Registry<vk::ImageView, chassis::ImageViewTag>,
}

impl VulkanResources {
    pub fn new(
        device: &ash::Device,
        memory_properties: vk::PhysicalDeviceMemoryProperties,
    ) -> Self {
        Self {
            device: device.clone(),
            buffers: Buffers::new(device, memory_properties),
            textures: Textures::new(device, memory_properties),
            // The device, so `Drop` can destroy the modules. A registry with no
            // device behind it can create nothing and leaks everything.
            shaders: Shaders::with_device(Some(device)),
            pipelines: Pipelines::new(device, vk::PipelineCache::null()),
            descriptors: Descriptors::new(device),
            image_views: Registry::new(),
        }
    }

    pub fn device(&self) -> &ash::Device {
        &self.device
    }

    pub fn buffers(&self) -> &Buffers {
        &self.buffers
    }

    pub fn textures(&self) -> &Textures {
        &self.textures
    }

    pub fn shaders(&self) -> &Shaders {
        &self.shaders
    }

    pub fn pipelines(&self) -> &Pipelines {
        &self.pipelines
    }

    pub fn descriptors(&self) -> &Descriptors {
        &self.descriptors
    }

    /// How many views were handed out and not yet accounted for.
    ///
    /// A growing count is a leak; a count that never returns to zero means views
    /// outlive their textures, which is a use-after-free rather than a leak, and
    /// is worse.
    /// Mint an RHI handle for a view this backend did not create from a
    /// `Texture`.
    ///
    /// For swapchain images, which are owned by the swapchain rather than by the
    /// texture registry -- so there is no `chassis::Texture` to hand to
    /// `create_image_view`, but the RHI still needs a handle it can pass to a
    /// render pass. The registry stores the pairing and destroys nothing; the
    /// swapchain owns the view and frees it with the image it looks at.
    pub fn register_image_view(&mut self, view: ash::vk::ImageView) -> chassis::ImageView {
        self.image_views.create(view)
    }

    pub fn live_image_views(&self) -> usize {
        self.image_views.len()
    }

    /// The registries a command list needs, borrowed for one frame.
    pub fn executor_resources(&self) -> crate::vulkan_render_graph_executor::ExecutorResources<'_> {
        crate::vulkan_render_graph_executor::ExecutorResources {
            pipelines: &self.pipelines,
            textures: &self.textures,
            buffers: &self.buffers,
            descriptors: &self.descriptors,
        }
    }
}

/// Map a backend error onto the RHI's.
impl From<BackendError> for chassis::Error {
    fn from(error: BackendError) -> Self {
        // `Unsupported`, not a generic backend variant: the RHI's error set is
        // small on purpose, and "this device cannot do that" is the honest shape
        // for most of what fails here. A blanket string variant would let a
        // caller stop checking and the message is all it would have had.
        chassis::Error::Unsupported(error.to_string())
    }
}

impl chassis::Resources for VulkanResources {
    fn create_buffer(
        &mut self,
        desc: chassis::BufferDesc,
        initial: Option<&[u8]>,
    ) -> chassis::Result<chassis::Buffer> {
        self.buffers
            .create(&self.device, &desc, initial)
            .map_err(Into::into)
    }

    fn create_texture(&mut self, desc: chassis::TextureDesc) -> chassis::Result<chassis::Texture> {
        // `TextureDesc` has no label field. That is a gap in the RHI worth
        // closing -- a texture with no name is a texture nobody can find in a
        // validation message -- but for now the label comes from nothing and the
        // backend says so in the resource label rather than inventing one.
        self.textures
            .create(&self.device, &desc, "texture")
            .map_err(Into::into)
    }

    fn create_image_view(
        &mut self,
        texture: &chassis::Texture,
    ) -> chassis::Result<chassis::ImageView> {
        let view = self
            .textures
            .create_view(&self.device, *texture)
            .map_err(chassis::Error::from)?;
        Ok(self.image_views.create(view.handle()))
    }

    fn create_shader(&mut self, _label: &str, spirv: &[u8]) -> chassis::Result<chassis::Shader> {
        // `forge` is the only producer and it has already checked the magic
        // number, so re-validating here would be a second opinion about bytes
        // that are not in question.
        let module = forge::Module::from_bytes(spirv).ok_or(chassis::Error::InvalidDescriptor(
            "not a SPIR-V module".to_owned(),
        ))?;

        // Stage is taken from the module rather than asked for: the RHI hands
        // over SPIR-V and a label, and guessing a stage here means a vertex
        // module created as a fragment one is accepted and fails at pipeline
        // creation instead of here. Both stages are claimed and the pipeline
        // picks, because a module's entry point does not say which it is.
        self.shaders
            .create_from_words(
                &self.device,
                &module.words,
                vk::ShaderStageFlags::ALL_GRAPHICS,
            )
            .map_err(Into::into)
    }

    fn create_pipeline(
        &mut self,
        desc: chassis::PipelineDesc,
    ) -> chassis::Result<chassis::Pipeline> {
        crate::vulkan_graphic_pipeline::create(&mut self.pipelines, &desc, &self.shaders)
            .map_err(Into::into)
    }

    fn create_bind_group(
        &mut self,
        _layout: &[u32],
        _buffers: &[(chassis::Buffer, u64)],
    ) -> chassis::Result<chassis::BindGroup> {
        Err(chassis::Error::Unsupported(
            "bind groups need a declared set layout, which nothing declares yet".to_owned(),
        ))
    }

    fn destroy_buffer(&mut self, buffer: chassis::Buffer) {
        self.buffers.destroy(&self.device, buffer);
    }

    fn destroy_texture(&mut self, texture: chassis::Texture) {
        self.textures.destroy(&self.device, texture);
    }

    fn destroy_pipeline(&mut self, pipeline: chassis::Pipeline) {
        self.pipelines.destroy(&self.device, pipeline);
    }

    fn destroy_shader(&mut self, shader: chassis::Shader) {
        self.shaders.destroy(&self.device, shader);
    }

    fn destroy_bind_group(&mut self, _group: chassis::BindGroup) {}
}
