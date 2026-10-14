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

use std::sync::{Arc, RwLock};

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
    /// `Arc<RwLock<_>>`, and the reason is `'static`.
    ///
    /// `grind::Jobs` spawns its workers when the pool is built, so a job cannot
    /// borrow from the frame that queued it -- `parallel_for` takes an `'static`
    /// closure. The recording threads therefore need to *own* a handle to each
    /// registry rather than borrow one, which means `Arc`.
    ///
    /// And they need `&mut` for creation, which means something between `Arc` and
    /// nothing: `Arc<Registry>` alone would need `Arc::make_mut`, and that
    /// requires `Clone` on the registry, and a registry owns driver objects that
    /// cannot be cloned. `RwLock` is the shape that fits -- creation takes
    /// `write()`, recording takes `read()`, and the `&mut self` on
    /// `Resources::create_*` already guarantees no creation overlaps a frame.
    ///
    /// The lock is taken once per pass, not once per handle lookup, so the cost
    /// is one uncontended atomic per registry per pass.
    buffers: Arc<RwLock<Buffers>>,
    textures: Arc<RwLock<Textures>>,
    shaders: Arc<RwLock<Shaders>>,
    pipelines: Arc<RwLock<Pipelines>>,
    descriptors: Arc<RwLock<Descriptors>>,
    /// Handles for the views `create_image_view` hands out.
    ///
    /// The RHI returns a `chassis::ImageView`, and the driver object behind it
    /// is destroyed with the texture. That is safe as long as nobody uses a view
    /// after its texture, which nothing checks -- so the registry exists at least
    /// to make the pairing explicit and to give the error a handle to name.
    image_views: Registry<vk::ImageView, chassis::ImageViewTag>,
    /// `VkPhysicalDeviceLimits::minUniformBufferOffsetAlignment`.
    ///
    /// Checked in `create_bind_group` rather than left to validation, because a
    /// misaligned offset produces
    /// `VUID-VkWriteDescriptorSet-descriptorType-00327` naming a *limit* and a
    /// *descriptor*, and not the binding or the offset that was wrong.
    ///
    /// One means "not told". A caller that never sets it gets no alignment check
    /// rather than a wrong one -- which is the honest default, since guessing a
    /// limit would reject offsets the device would have accepted.
    uniform_buffer_alignment: vk::DeviceSize,
}

impl VulkanResources {
    pub fn new(
        device: &ash::Device,
        memory_properties: vk::PhysicalDeviceMemoryProperties,
    ) -> Self {
        Self {
            device: device.clone(),
            buffers: Arc::new(RwLock::new(Buffers::new(device, memory_properties))),
            textures: Arc::new(RwLock::new(Textures::new(device, memory_properties))),
            // The device, so `Drop` can destroy the modules. A registry with no
            // device behind it can create nothing and leaks everything.
            shaders: Arc::new(RwLock::new(Shaders::with_device(Some(device)))),
            pipelines: Arc::new(RwLock::new(Pipelines::new(
                device,
                vk::PipelineCache::null(),
            ))),
            descriptors: Arc::new(RwLock::new(Descriptors::new(device))),
            image_views: Registry::new(),
            uniform_buffer_alignment: 1,
        }
    }

    pub fn device(&self) -> &ash::Device {
        &self.device
    }

    pub fn buffers(&self) -> std::sync::RwLockReadGuard<'_, Buffers> {
        self.buffers.read().expect("a poisoned registry lock")
    }

    /// The write guard, for creation.
    ///
    /// Uncontended by construction: `Resources::create_*` takes `&mut self`, so
    /// nothing else can be holding a read guard at the same time.
    pub fn buffers_mut(&mut self) -> std::sync::RwLockWriteGuard<'_, Buffers> {
        self.buffers.write().expect("a poisoned registry lock")
    }

    /// An owned handle, for a recording thread that outlives this borrow.
    pub fn shared_buffers(&self) -> Arc<RwLock<Buffers>> {
        Arc::clone(&self.buffers)
    }

    pub fn textures(&self) -> std::sync::RwLockReadGuard<'_, Textures> {
        self.textures.read().expect("a poisoned registry lock")
    }

    /// The write guard, for creation.
    ///
    /// Uncontended by construction: `Resources::create_*` takes `&mut self`, so
    /// nothing else can be holding a read guard at the same time.
    pub fn textures_mut(&mut self) -> std::sync::RwLockWriteGuard<'_, Textures> {
        self.textures.write().expect("a poisoned registry lock")
    }

    /// An owned handle, for a recording thread that outlives this borrow.
    pub fn shared_textures(&self) -> Arc<RwLock<Textures>> {
        Arc::clone(&self.textures)
    }

    pub fn shaders(&self) -> std::sync::RwLockReadGuard<'_, Shaders> {
        self.shaders.read().expect("a poisoned registry lock")
    }

    /// The write guard, for creation.
    ///
    /// Uncontended by construction: `Resources::create_*` takes `&mut self`, so
    /// nothing else can be holding a read guard at the same time.
    pub fn shaders_mut(&mut self) -> std::sync::RwLockWriteGuard<'_, Shaders> {
        self.shaders.write().expect("a poisoned registry lock")
    }

    /// An owned handle, for a recording thread that outlives this borrow.
    pub fn shared_shaders(&self) -> Arc<RwLock<Shaders>> {
        Arc::clone(&self.shaders)
    }

    pub fn pipelines(&self) -> std::sync::RwLockReadGuard<'_, Pipelines> {
        self.pipelines.read().expect("a poisoned registry lock")
    }

    /// The write guard, for creation.
    ///
    /// Uncontended by construction: `Resources::create_*` takes `&mut self`, so
    /// nothing else can be holding a read guard at the same time.
    pub fn pipelines_mut(&mut self) -> std::sync::RwLockWriteGuard<'_, Pipelines> {
        self.pipelines.write().expect("a poisoned registry lock")
    }

    /// An owned handle, for a recording thread that outlives this borrow.
    pub fn shared_pipelines(&self) -> Arc<RwLock<Pipelines>> {
        Arc::clone(&self.pipelines)
    }

    pub fn descriptors(&self) -> std::sync::RwLockReadGuard<'_, Descriptors> {
        self.descriptors.read().expect("a poisoned registry lock")
    }

    /// The write guard, for creation.
    ///
    /// Uncontended by construction: `Resources::create_*` takes `&mut self`, so
    /// nothing else can be holding a read guard at the same time.
    pub fn descriptors_mut(&mut self) -> std::sync::RwLockWriteGuard<'_, Descriptors> {
        self.descriptors.write().expect("a poisoned registry lock")
    }

    /// An owned handle, for a recording thread that outlives this borrow.
    pub fn shared_descriptors(&self) -> Arc<RwLock<Descriptors>> {
        Arc::clone(&self.descriptors)
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

    /// Records the device's uniform buffer offset alignment.
    ///
    /// Required before `create_bind_group` will validate offsets. `VulkanGpu`
    /// sets it from `Capabilities`; a caller that skips it gets no check, which
    /// is the deliberate consequence of not knowing the limit.
    pub fn with_uniform_buffer_alignment(mut self, alignment: vk::DeviceSize) -> Self {
        self.uniform_buffer_alignment = alignment.max(1);
        self
    }

    /// Distinct descriptor set layouts created so far.
    ///
    /// A count that climbs per frame means the layout hash is missing a field,
    /// and every pass is making its own layout -- at which point a set allocated
    /// against one layout cannot be bound to a pipeline built with another.
    pub fn descriptor_layout_count(&self) -> usize {
        self.descriptors().layout_count()
    }

    pub fn live_image_views(&self) -> usize {
        self.image_views.len()
    }

    /// The registries a command list needs, borrowed for one frame.
    pub fn executor_resources(&self) -> crate::vulkan_render_graph_executor::ExecutorResources {
        crate::vulkan_render_graph_executor::ExecutorResources {
            pipelines: self.shared_pipelines(),
            textures: self.shared_textures(),
            buffers: self.shared_buffers(),
            descriptors: self.shared_descriptors(),
        }
    }
}

/// The RHI's binding declaration as the descriptor manager's.
///
/// A field-by-field copy rather than a `From` impl, because the two are separate
/// types on purpose: the RHI's carries `stages`, which a set layout does not, and
/// a `From` would invite someone to reach for it and lose the stage mask without
/// being told.
fn convert_binding(binding: &chassis::BindingDesc) -> crate::vulkan_descriptor_manager::Binding {
    crate::vulkan_descriptor_manager::Binding {
        binding: binding.binding,
        count: binding.count,
        kind: binding.kind,
    }
}

/// The `vk::DescriptorType` a layout declares for `binding`.
///
/// An error rather than a default, because the two failure modes are different:
/// a binding that is not declared at all is a caller mistake, and one that is
/// declared as something `create_bind_group` cannot write yet is a limitation.
/// Both name the binding.
fn declared_type(
    layout: &chassis::DescriptorSetLayoutDesc,
    binding: u32,
) -> chassis::Result<vk::DescriptorType> {
    let declared = layout
        .bindings
        .iter()
        .find(|declared| declared.binding == binding)
        .ok_or_else(|| {
            chassis::Error::InvalidDescriptor(format!(
                "binding {binding} was written but the layout does not declare it"
            ))
        })?;
    Ok(match declared.kind {
        chassis::DescriptorKind::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
        chassis::DescriptorKind::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
        other => {
            return Err(chassis::Error::InvalidDescriptor(format!(
                "binding {binding} is a {other:?}; create_bind_group writes buffers only"
            )));
        }
    })
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
        let device = self.device.clone();
        self.buffers_mut()
            .create(&device, &desc, initial)
            .map_err(Into::into)
    }

    fn create_texture(&mut self, desc: chassis::TextureDesc) -> chassis::Result<chassis::Texture> {
        // `TextureDesc` has no label field. That is a gap in the RHI worth
        // closing -- a texture with no name is a texture nobody can find in a
        // validation message -- but for now the label comes from nothing and the
        // backend says so in the resource label rather than inventing one.
        // The device is cloned out first: `textures_mut()` borrows all of `self`,
        // so passing `&self.device` at the same time is two overlapping borrows
        // of the same object. An `ash::Device` clone is a refcount bump.
        let device = self.device.clone();
        self.textures_mut()
            .create(&device, &desc, "texture")
            .map_err(Into::into)
    }

    fn create_image_view(
        &mut self,
        texture: &chassis::Texture,
    ) -> chassis::Result<chassis::ImageView> {
        let device = self.device.clone();
        let view = self
            .textures_mut()
            .create_view(&device, *texture)
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
        let device = self.device.clone();
        self.shaders_mut()
            .create_from_words(&device, &module.words, vk::ShaderStageFlags::ALL_GRAPHICS)
            .map_err(Into::into)
    }

    fn create_pipeline(
        &mut self,
        desc: chassis::PipelineDesc,
    ) -> chassis::Result<chassis::Pipeline> {
        // Guards taken up front, then handed to the builder. Taking them in
        // argument position would borrow `self` mutably and immutably at once,
        // which is two overlapping borrows of the same struct -- not a lock
        // conflict, just a borrow checker one.
        // Two guards, one read and one write, on two *different fields*.
        //
        // Taken through the fields rather than through `self.shaders()` and
        // `self.pipelines_mut()`: those borrow the whole struct, one `&` and one
        // `&mut`, and the borrow checker is right to refuse. At field level they
        // are genuinely disjoint, so the split is real rather than asserted.
        let shaders = self.shaders.read().expect("registry lock");
        let mut pipelines = self.pipelines.write().expect("registry lock");
        crate::vulkan_graphic_pipeline::create(&mut pipelines, &desc, &shaders).map_err(Into::into)
    }

    fn create_bind_group(
        &mut self,
        layout: &chassis::DescriptorSetLayoutDesc,
        buffers: &[(u32, chassis::Buffer, u64)],
    ) -> chassis::Result<chassis::BindGroup> {
        // The same descriptor the pipeline declares, so the set and the layout it
        // was allocated against come from one description. `Descriptors` hashes it,
        // so a pipeline and its bind group built from equal descriptors share one
        // `vk::DescriptorSetLayout` -- which is what makes binding a set to a
        // pipeline legal at all.
        let set_layout = self
            .descriptors_mut()
            .layout(&crate::vulkan_descriptor_manager::BindGroupLayoutDesc {
                bindings: layout.bindings.iter().map(convert_binding).collect(),
            })
            .map_err(chassis::Error::from)?;

        let group = self
            .descriptors_mut()
            .allocate(set_layout)
            .map_err(chassis::Error::from)?;

        let set = self
            .descriptors()
            .set(group)
            .expect("just allocated, so it is in the registry")
            .handle();

        // Two passes, not one.
        //
        // `WriteDescriptorSet::buffer_info` *borrows* the `DescriptorBufferInfo`,
        // so building both in a single loop keeps a borrow of `infos` live across
        // the next iteration's `infos.push` -- which may reallocate. The borrow
        // checker is right to refuse and there is no way to make it wrong in a
        // way that is also correct in general, so: collect the infos, then point
        // the writes at them.
        let mut infos = Vec::with_capacity(buffers.len());
        let mut types = Vec::with_capacity(buffers.len());
        for (binding, buffer, offset) in buffers {
            let buffers = self.buffers();
            let resource = buffers.get(*buffer).ok_or(chassis::Error::InvalidHandle)?;
            // Refused rather than clamped.
            //
            // `offset >= size` gives either a zero range or a range that runs off
            // the end, and both are `vkUpdateDescriptorSets` validation errors:
            // `VUID-VkDescriptorBufferInfo-range-00341` and
            // `...-offset-00340`. This used to be `saturating_sub`, which turned
            // an offset of 256 into a range of 0 -- the call returned `Ok` and the
            // descriptor was written pointing at nothing.
            //
            // Clamping is worse than useless here: the message names neither the
            // buffer nor the offset, and a descriptor that reads nothing is
            // indistinguishable from one reading a cleared buffer.
            let size = resource.size();
            // Alignment first: a misaligned offset that is also past the end
            // should name the alignment, which is the thing that is wrong about
            // it in both cases.
            let alignment = match declared_type(layout, *binding)? {
                vk::DescriptorType::UNIFORM_BUFFER => self.uniform_buffer_alignment,
                _ => 1,
            };
            if alignment > 1 && *offset % alignment != 0 {
                return Err(chassis::Error::InvalidDescriptor(format!(
                    "binding {binding}: offset {offset} is not a multiple of this \
                     device's minUniformBufferOffsetAlignment ({alignment})"
                )));
            }
            if *offset >= size {
                return Err(chassis::Error::InvalidDescriptor(format!(
                    "binding {binding}: offset {offset} is at or past the end of a \
                     {size}-byte buffer"
                )));
            }
            infos.push(
                vk::DescriptorBufferInfo::default()
                    .buffer(resource.handle())
                    .offset(*offset)
                    // The range runs to the end of the buffer. `WHOLE_SIZE` would
                    // run past the allocation for any non-zero offset, which
                    // validation reports as a range exceeding the buffer --
                    // confusingly attributed to the descriptor rather than to the
                    // offset.
                    .range(size - *offset),
            );
            types.push(declared_type(layout, *binding)?);
        }

        let writes: Vec<vk::WriteDescriptorSet<'_>> = infos
            .iter()
            .zip(types)
            .enumerate()
            .map(|(slot, (info, descriptor_type))| {
                let binding = buffers[slot].0;
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(binding)
                    // Must match what the layout declares. `vkUpdateDescriptorSets`
                    // does not check -- it writes -- and the mismatch surfaces at
                    // draw time as "descriptor type mismatch", naming neither the
                    // layout nor the write.
                    .descriptor_type(descriptor_type)
                    .buffer_info(std::slice::from_ref(info))
            })
            .collect();

        if !writes.is_empty() {
            self.descriptors_mut()
                .update(group, &writes, &[])
                .map_err(chassis::Error::from)?;
        }
        Ok(group)
    }

    fn destroy_buffer(&mut self, buffer: chassis::Buffer) {
        let device = self.device.clone();
        self.buffers_mut().destroy(&device, buffer);
    }

    fn destroy_texture(&mut self, texture: chassis::Texture) {
        let device = self.device.clone();
        self.textures_mut().destroy(&device, texture);
    }

    fn destroy_pipeline(&mut self, pipeline: chassis::Pipeline) {
        let device = self.device.clone();
        self.pipelines_mut().destroy(&device, pipeline);
    }

    fn destroy_shader(&mut self, shader: chassis::Shader) {
        let device = self.device.clone();
        self.shaders_mut().destroy(&device, shader);
    }

    fn destroy_bind_group(&mut self, _group: chassis::BindGroup) {}
}
