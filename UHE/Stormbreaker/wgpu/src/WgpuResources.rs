//! The wgpu implementation of `chassis::Resources`.
//!
//! ## The same trait, the same handles, a different object behind them
//!
//! This and `VulkanResources` are interchangeable behind `&mut dyn Resources`.
//! The renderer cannot tell them apart, which is the entire point of splitting
//! the RHI out of the engine crate -- and the reason a test that calls the
//! concrete methods proves nothing.
//!
//! ## Where wgpu is genuinely smaller
//!
//! There is no deletion queue. Vulkan needs one because a `VkBuffer` outlives
//! whatever submitted work might still read it, and freeing it early is a
//! use-after-free that only shows up under load. wgpu owns its objects: a
//! `wgpu::Buffer` dies with the device and is reference-counted until then. The
//! absence is not an omission, it is the single largest reason this backend will
//! end up about half the size of the Vulkan one.
//!
//! ## The one place wgpu costs more
//!
//! Format and usage translation. Vulkan names formats and usage bits directly;
//! wgpu has its own vocabulary (`Rg11b10Ufloat` for `R11b11G11B10Float`,
//! `...UnormSrgb` for `...Srgb`), and a mapping that silently substitutes a
//! near-enough format renders the wrong image rather than failing. Every
//! unmappable format is `None` here for that reason.

use std::borrow::Cow;

use chassis::Registry;
// `create_buffer_init` is not on `wgpu::Device`. It is an extension trait in
// `wgpu::util`, which is the crate's way of saying "convenience, not core API"
// -- and forgetting the import is a compile error rather than a silent
// fallback, which is the right way round.
use wgpu::util::DeviceExt;

use crate::wgpu_device::{self, Device};

/// A compiled pipeline.
///
/// wgpu owns the `RenderPipeline`, so as with buffers and textures this only has
/// to exist to keep the handle resolving.
pub use crate::wgpu_pipeline::PipelineResource;

/// What the RHI calls a buffer.
pub struct BufferResource {
    inner: wgpu::Buffer,
    size: u64,
    label: String,
}

impl BufferResource {
    pub fn inner(&self) -> &wgpu::Buffer {
        &self.inner
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

/// A texture and the view made with it.
///
/// The view lives here rather than in a separate registry because wgpu makes no
/// other claim on it and it dies with the texture anyway. The RHI can still ask
/// for a *second* view with a different aspect, and those are tracked
/// separately -- a view must never outlive what it views.
pub struct TextureResource {
    inner: wgpu::Texture,
    view: wgpu::TextureView,
    format: chassis::TextureFormat,
    extent: chassis::Extent,
    label: String,
}

impl TextureResource {
    pub fn inner(&self) -> &wgpu::Texture {
        &self.inner
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn format(&self) -> chassis::TextureFormat {
        self.format
    }

    pub fn extent(&self) -> chassis::Extent {
        self.extent
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

pub struct ShaderResource {
    inner: wgpu::ShaderModule,
    label: String,
}

impl ShaderResource {
    pub fn inner(&self) -> &wgpu::ShaderModule {
        &self.inner
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

/// Owns every wgpu object the RHI creates.
pub struct WgpuResources {
    device: Device,
    buffers: Registry<BufferResource, chassis::BufferTag>,
    textures: Registry<TextureResource, chassis::TextureTag>,
    views: Registry<wgpu::TextureView, chassis::ImageViewTag>,
    shaders: Registry<ShaderResource, chassis::ShaderTag>,
    pipelines: Registry<PipelineResource, chassis::PipelineTag>,
}

impl WgpuResources {
    pub fn new(device: &Device) -> Self {
        Self {
            device: device.clone(),
            buffers: Registry::new(),
            textures: Registry::new(),
            views: Registry::new(),
            shaders: Registry::new(),
            pipelines: Registry::new(),
        }
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn get_buffer(&self, handle: chassis::Buffer) -> Option<&BufferResource> {
        self.buffers.get(handle)
    }

    pub fn get_texture(&self, handle: chassis::Texture) -> Option<&TextureResource> {
        self.textures.get(handle)
    }

    pub fn get_shader(&self, handle: chassis::Shader) -> Option<&ShaderResource> {
        self.shaders.get(handle)
    }

    /// Stores a compiled pipeline. Called by `wgpu_pipeline`, which builds one
    /// and needs somewhere to put it.
    pub(crate) fn register_pipeline(
        &mut self,
        pipeline: crate::wgpu_pipeline::PipelineResource,
    ) -> chassis::Pipeline {
        self.pipelines.create(pipeline)
    }

    pub fn get_pipeline(&self, handle: chassis::Pipeline) -> Option<&PipelineResource> {
        self.pipelines.get(handle)
    }

    /// Live objects, for a leak check in a test.
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        (
            self.buffers.len(),
            self.textures.len(),
            self.views.len(),
            self.shaders.len(),
        )
    }

    /// Forgets a handle without destroying anything.
    ///
    /// wgpu needs no destroy, so this is all `destroy_*` has to do -- but the
    /// handle must still stop resolving, or a stale one returns a dropped
    /// object and the next use is a use-after-free rather than a clean error.
    fn forget_buffer(&mut self, handle: chassis::Buffer) {
        self.buffers.remove(handle);
    }

    fn forget_texture(&mut self, handle: chassis::Texture) {
        self.textures.remove(handle);
    }

    fn forget_shader(&mut self, handle: chassis::Shader) {
        self.shaders.remove(handle);
    }
}

impl chassis::Resources for WgpuResources {
    fn create_buffer(
        &mut self,
        desc: chassis::BufferDesc,
        initial: Option<&[u8]>,
    ) -> chassis::Result<chassis::Buffer> {
        let usage = wgpu_device::to_wgpu_buffer_usage(desc.usage);
        if usage.is_empty() {
            // Vulkan has the same rule and the same reason: a buffer with no
            // usage is a driver object that can do nothing, and some drivers
            // accept it and then ignore every command recorded against it.
            return Err(chassis::Error::Unsupported(
                "a buffer needs at least one usage".to_owned(),
            ));
        }
        if desc.size == 0 {
            return Err(chassis::Error::Unsupported(
                "a buffer of zero size is not a thing".to_owned(),
            ));
        }

        let label = format!("buffer({} bytes)", desc.size);
        let inner = match initial {
            Some(bytes) => {
                // `create_buffer_init` uploads and keeps the staging copy alive
                // until the first submit that reads it. Without initial data a
                // plain buffer is correct and cheaper, so the two paths are
                // kept apart rather than always paying for the staging one.
                self.device
                    .inner()
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&label),
                        contents: bytes,
                        usage,
                    })
            }
            None => self.device.inner().create_buffer(&wgpu::BufferDescriptor {
                label: Some(&label),
                size: desc.size,
                usage,
                mapped_at_creation: false,
            }),
        };

        Ok(self.buffers.create(BufferResource {
            inner,
            size: desc.size,
            label,
        }))
    }

    fn create_texture(&mut self, desc: chassis::TextureDesc) -> chassis::Result<chassis::Texture> {
        let Some(format) = wgpu_device::to_wgpu_format(desc.format) else {
            // No substitution. A near-enough format renders the wrong image,
            // which is a colour bug that sends you looking at the shader.
            return Err(chassis::Error::Unsupported(format!(
                "{:?} has no wgpu equivalent",
                desc.format
            )));
        };
        let usage = wgpu_device::to_wgpu_texture_usage(desc.usage);

        let label = "texture".to_owned();
        let size = wgpu::Extent3d {
            width: desc.extent.width.max(1),
            height: desc.extent.height.max(1),
            depth_or_array_layers: desc.extent.depth.max(1),
        };
        let inner = self
            .device
            .inner()
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(&label),
                size,
                mip_level_count: desc.mip_levels.max(1),
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            });

        let view = inner.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(self.textures.create(TextureResource {
            inner,
            view,
            format: desc.format,
            extent: desc.extent,
            label,
        }))
    }

    fn create_image_view(
        &mut self,
        texture: &chassis::Texture,
    ) -> chassis::Result<chassis::ImageView> {
        let Some(resource) = self.textures.get(*texture) else {
            return Err(chassis::Error::InvalidHandle);
        };
        // A second view, not the one made with the texture. A sampler wants a
        // different aspect or mip range than a render target does, and handing
        // back the first view would be wrong for that without saying so.
        let view = resource
            .inner()
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(self.views.create(view))
    }

    fn create_shader(&mut self, label: &str, spirv: &[u8]) -> chassis::Result<chassis::Shader> {
        let module = forge::Module::from_bytes(spirv)
            .ok_or_else(|| chassis::Error::InvalidDescriptor("not SPIR-V".to_owned()))?;

        // The magic number is checked above by `forge`, and that check is doing
        // more than tidiness: wgpu's `create_shader_module` is infallible, so a
        // module it dislikes goes to an error scope, and with no scope installed
        // it *panics*. Vulkan's returns `Result` and does not. Validating here is
        // what makes the two backends agree on the common case -- truncated and
        // wrong-magic input.
        //
        // What it does not catch is a module with a correct magic number and a
        // corrupt body. That panics here and is not caught, deliberately: a panic
        // is loud and immediate, and wrapping it in a catch would turn a
        // specific failure into a silent wrong answer.
        let inner = self
            .device
            .inner()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                // SPIR-V, straight from forge. The same bytes the Vulkan backend
                // takes: one compiler, two consumers. naga validates on the way
                // in, so a malformed module is reported here rather than as an
                // opaque failure at pipeline creation.
                source: wgpu::ShaderSource::SpirV(Cow::Owned(module.words)),
            });
        Ok(self.shaders.create(ShaderResource {
            inner,
            label: label.to_owned(),
        }))
    }

    fn create_pipeline(
        &mut self,
        desc: chassis::PipelineDesc,
    ) -> chassis::Result<chassis::Pipeline> {
        crate::wgpu_pipeline::create(self, &desc)
    }

    fn create_bind_group(
        &mut self,
        _layout: &chassis::DescriptorSetLayoutDesc,
        _buffers: &[(u32, chassis::Buffer, u64)],
    ) -> chassis::Result<chassis::BindGroup> {
        // Same answer as the Vulkan backend, for the same reason: a bind group
        // needs a declared layout and nothing declares one yet. Returning an
        // error says so; returning a handle that binds nothing renders the
        // wrong image.
        Err(chassis::Error::Unsupported(
            "bind groups need a declared set layout, which nothing declares yet".to_owned(),
        ))
    }

    fn destroy_buffer(&mut self, buffer: chassis::Buffer) {
        // No `destroy_buffer`: wgpu reference-counts. Only the handle has to
        // stop resolving.
        self.forget_buffer(buffer);
    }

    fn destroy_texture(&mut self, texture: chassis::Texture) {
        self.forget_texture(texture);
    }

    fn destroy_pipeline(&mut self, pipeline: chassis::Pipeline) {
        self.pipelines.remove(pipeline);
    }

    fn destroy_shader(&mut self, shader: chassis::Shader) {
        self.forget_shader(shader);
    }

    fn destroy_bind_group(&mut self, _group: chassis::BindGroup) {}
}
