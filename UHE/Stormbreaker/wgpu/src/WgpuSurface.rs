//! The configured surface: wgpu's swapchain, and the frame it hands out.
//!
//! ## Why this crate never names a windowing library
//!
//! A `wgpu::Surface` comes from a window, and building one needs a window handle.
//! The caller makes it -- `eyes` re-exports `winit::Window`, and a `Surface<'static>`
//! comes out of `Instance::create_surface(window)`. This crate only ever receives
//! the result, so the backend has no opinion about how a window was made and wgpu's
//! one implementation serves Vulkan, Metal, DX12 and the browser without a `#[cfg]`
//! anywhere in it.
//!
//! ## What a swapchain is here
//!
//! In wgpu the swapchain is not an object anybody holds: `Surface::configure` builds
//! one for a configuration, the images belong to the driver, and `get_current_texture`
//! hands back one of them. So this type owns the surface, the configuration, and the
//! RHI handles that name the acquired image -- which is everything the RHI's
//! `Swapchain` and `chassis::Gpu::swapchain` ask for.
//!
//! ## Why the image has two handles
//!
//! A render-graph pass attaches a `chassis::Texture` (`builder::import_texture`, then
//! `SlotTable::set_texture`), and the RHI's swapchain reports a `chassis::ImageView`.
//! Both are minted from the one acquired image, in the one registry the executor reads,
//! because a handle minted into a second registry would resolve to nothing -- or, worse,
//! to whatever occupies that slot in the real one.

use crate::wgpu_device::{from_wgpu_format, Adapter, Device, Queue};
use crate::wgpu_error::WgpuError;
use crate::wgpu_resources::WgpuResources;
use crate::wgpu_swapchain::{
    choose_alpha_mode, choose_extent, choose_format, choose_present_mode, classify, images_for,
    latency_for, AcquireStatus, IMAGE_COUNT,
};

/// A surface, configured and ready to hand out frames.
pub struct WgpuSurface {
    surface: wgpu::Surface<'static>,
    /// Kept, not just for the device it made: a reconfigure has to re-ask the
    /// surface what it offers, and only the adapter can answer that.
    adapter: Adapter,
    device: Device,
    /// wgpu 30 presents through the queue rather than through the surface texture.
    queue: Queue,
    config: wgpu::SurfaceConfiguration,
    format: chassis::TextureFormat,
    extent: chassis::Extent2d,
    /// The acquired frame. `None` between frames, and `Surface::configure` panics
    /// if one is alive, so it is taken before every reconfigure.
    frame: Option<wgpu::SurfaceTexture>,
    /// The handles naming `frame`, live only while it is.
    texture: Option<chassis::Texture>,
    view: Option<chassis::ImageView>,
    out_of_date: bool,
    frames: u64,
}

impl WgpuSurface {
    /// Configure `surface` against what the adapter says it offers.
    ///
    /// `extent` is the window's own size: wgpu exposes no `current_extent` to choose
    /// instead, and `choose_extent` clamps it into what `configure` accepts.
    pub fn new(
        surface: wgpu::Surface<'static>,
        adapter: Adapter,
        device: Device,
        queue: Queue,
        extent: chassis::Extent2d,
    ) -> Result<Self, WgpuError> {
        let (config, format) = plan(&surface, &adapter, &device, extent, None, None)?;
        let extent = chassis::Extent2d {
            width: config.width,
            height: config.height,
        };
        surface.configure(device.inner(), &config);
        Ok(Self {
            surface,
            adapter,
            device,
            queue,
            config,
            format,
            extent,
            frame: None,
            texture: None,
            view: None,
            out_of_date: false,
            frames: 0,
        })
    }

    pub fn extent(&self) -> chassis::Extent2d {
        self.extent
    }

    pub fn format(&self) -> chassis::TextureFormat {
        self.format
    }

    pub fn present_mode(&self) -> wgpu::PresentMode {
        self.config.present_mode
    }

    /// The images the configuration asks for.
    ///
    /// wgpu exposes the *goal* and clamps it to what the backend supports, so this
    /// is the count it asked for rather than a count it can be told: the RHI wants a
    /// number that is not zero, and zero is the one answer that would be a lie about
    /// a swapchain that exists.
    pub fn image_count(&self) -> u32 {
        images_for(self.config.desired_maximum_frame_latency)
    }

    pub fn is_out_of_date(&self) -> bool {
        self.out_of_date
    }

    /// Frames presented.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The acquired image, as the handle a render-graph pass attaches.
    ///
    /// `None` between frames: the handle names one image of one swapchain, and
    /// presenting or reconfiguring retires it.
    pub fn texture(&self) -> Option<chassis::Texture> {
        if self.frame.is_some() {
            self.texture
        } else {
            None
        }
    }

    /// The acquired image as a view, or a null handle while nothing is acquired.
    pub fn image_view(&self) -> chassis::ImageView {
        if self.frame.is_some() {
            self.view.unwrap_or_else(chassis::ImageView::null)
        } else {
            chassis::ImageView::null()
        }
    }

    /// Rebuild for a new size, or after an out-of-date acquire.
    ///
    /// `false` when nothing needed doing. `Surface::configure` waits for the device
    /// to go idle, so a window drag's worth of resize events must not each pay for
    /// one: the C++ `ResizeSwapchain` is an empty body, which is the same idea
    /// without the check.
    pub fn reconfigure(&mut self, extent: chassis::Extent2d) -> Result<bool, WgpuError> {
        if extent == self.extent && !self.out_of_date {
            return Ok(false);
        }
        self.configure(extent)?;
        Ok(true)
    }

    /// Acquire the next image, rebuilding the configuration once if it is stale.
    ///
    /// One retry, not a loop: an out-of-date acquire is a resize, and a surface that
    /// is still out of date one configure later is one that cannot be configured --
    /// which a loop would turn into a spin with no frame in it.
    ///
    /// No timeout parameter, unlike `VulkanGpu::acquire`: wgpu 30's
    /// `Surface::get_current_texture` takes none (`wgpu-hal`'s does, the public API
    /// does not), so `AcquireStatus::Timeout` is the only timeout there is, and it is
    /// the backend's own.
    pub fn acquire(&mut self, resources: &mut WgpuResources) -> Result<AcquireStatus, WgpuError> {
        let mut status = self.take_frame(resources);
        if status.needs_reconfigure() {
            let extent = self.extent;
            self.configure(extent)?;
            status = self.take_frame(resources);
        }
        self.out_of_date = status.needs_reconfigure();
        Ok(status)
    }

    /// Present the acquired image, and release it.
    ///
    /// wgpu presents through the queue, and the queue does what Vulkan makes the
    /// backend spell out: the image is transitioned into a presentable state inside
    /// the call -- `wgpu-core`'s `prepare_surface_texture_for_present`, which also
    /// inserts a clear when nothing ever rendered to it. There is no barrier to write
    /// here and no second command list; `transition_to_present` exists on the Vulkan
    /// side only because Vulkan asks for it explicitly.
    pub fn present(&mut self, resources: &mut WgpuResources) -> Result<(), WgpuError> {
        let Some(frame) = self.frame.take() else {
            // Nothing acquired: the skipped or still-out-of-date path. Presenting now
            // would be presenting an image nobody holds.
            return Ok(());
        };
        self.queue.inner().present(frame);
        self.release(resources);
        self.frames += 1;
        Ok(())
    }

    /// Rebuild the configuration against the surface's live capabilities.
    fn configure(&mut self, extent: chassis::Extent2d) -> Result<(), WgpuError> {
        let planned = plan(
            &self.surface,
            &self.adapter,
            &self.device,
            extent,
            Some(self.format),
            Some(self.config.present_mode),
        );
        let (config, format) = match planned {
            Ok(planned) => planned,
            // The surface no longer offers what it is configured with, which is what
            // a window dragged to another display looks like. Re-choose rather than
            // fail the frame.
            Err(WgpuError::Unsupported { .. }) => plan(
                &self.surface,
                &self.adapter,
                &self.device,
                extent,
                None,
                None,
            )?,
            Err(other) => return Err(other),
        };
        let extent = chassis::Extent2d {
            width: config.width,
            height: config.height,
        };
        // The frame goes first: `Surface::configure` panics while a `SurfaceTexture`
        // from the old swapchain is alive. Its handles stay in the registry -- the
        // next acquire or present forgets them, and `texture`/`image_view` report
        // nothing in between, because no frame is held.
        self.frame = None;
        self.surface.configure(self.device.inner(), &config);
        self.config = config;
        self.format = format;
        self.extent = extent;
        self.out_of_date = false;
        Ok(())
    }

    /// Forget the last frame's handles, then acquire one image and name it.
    fn take_frame(&mut self, resources: &mut WgpuResources) -> AcquireStatus {
        self.release(resources);
        let (frame, status) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, AcquireStatus::Success),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, AcquireStatus::Suboptimal),
            other => return classify(&other),
        };
        let texture = resources.register_texture(
            frame.texture.clone(),
            self.format,
            chassis::Extent {
                width: self.extent.width,
                height: self.extent.height,
                depth: 1,
            },
            "swapchain",
        );
        let view = resources.register_image_view(
            frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default()),
        );
        self.texture = Some(texture);
        self.view = Some(view);
        self.frame = Some(frame);
        status
    }

    /// Drop the frame's handles and its image.
    ///
    /// Forgetting is not destroying -- wgpu reference-counts, and the image is the
    /// surface's -- but the handles must stop resolving, because the next acquire
    /// names a different one.
    fn release(&mut self, resources: &mut WgpuResources) {
        if let Some(texture) = self.texture.take() {
            resources.forget_texture(texture);
        }
        if let Some(view) = self.view.take() {
            resources.forget_image_view(view);
        }
        self.frame = None;
    }
}

impl chassis::Swapchain for WgpuSurface {
    fn extent(&self) -> chassis::Extent2d {
        self.extent
    }

    fn format(&self) -> chassis::TextureFormat {
        self.format
    }

    fn current_image_view(&self) -> chassis::ImageView {
        self.image_view()
    }

    fn frame_index(&self) -> u32 {
        // Zero, and not a fabricated index: wgpu hands out a *texture*, and neither
        // the public API nor `SurfaceTexture` says which swapchain image it was.
        // Vulkan returns the index because `vkAcquireNextImageKHR` is asked for one.
        0
    }

    fn image_count(&self) -> u32 {
        self.image_count()
    }

    fn is_out_of_date(&self) -> bool {
        self.out_of_date
    }
}

/// The configuration a surface should be given, from its live capabilities.
///
/// A free function rather than a method: it needs the surface, the adapter and the
/// device at once, and as a method it would borrow all of the caller to read three
/// fields that do not overlap.
fn plan(
    surface: &wgpu::Surface<'static>,
    adapter: &Adapter,
    device: &Device,
    extent: chassis::Extent2d,
    format: Option<chassis::TextureFormat>,
    present_mode: Option<wgpu::PresentMode>,
) -> Result<(wgpu::SurfaceConfiguration, chassis::TextureFormat), WgpuError> {
    let capabilities = adapter.capabilities(surface);
    let chosen = choose_format(&capabilities.formats, format)?;
    let extent = choose_extent(extent, device.max_texture_size());
    // `Undefined` when the surface offers a format the RHI has no name for -- which
    // wgpu's own guarantee (BGRA8, sRGB or not, always present) makes unreachable in
    // practice. `Undefined` is the RHI's word for exactly this; a near-enough name
    // would report the wrong colour space as a success.
    let rhi_format = from_wgpu_format(chosen).unwrap_or(chassis::TextureFormat::Undefined);
    Ok((
        wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: chosen,
            // `Auto` resolves to sRGB for a `*Srgb` format, which is what the C++
            // gets by asking for `eB8G8R8A8Srgb` plus `eSrgbNonlinear` directly.
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: extent.width,
            height: extent.height,
            present_mode: choose_present_mode(&capabilities.present_modes, present_mode)?,
            desired_maximum_frame_latency: latency_for(IMAGE_COUNT),
            alpha_mode: choose_alpha_mode(&capabilities.alpha_modes),
            view_formats: Vec::new(),
        },
        rhi_format,
    ))
}
