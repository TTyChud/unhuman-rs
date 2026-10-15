//! The wgpu swapchain.
//!
//! Surface creation, configuration, acquire, present, resize and teardown for
//! the wgpu backend. A `Gpu` without a surface stays headless; this type only
//! exists when a window (or a raw window handle) was handed to the constructor.

use raw_window_handle::RawWindowHandle;

use chassis::Error as RhiError;

/// A real wgpu surface plus its configuration.
pub struct WgpuSwapchain {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    current_texture: Option<wgpu::SurfaceTexture>,
    current_view: Option<wgpu::TextureView>,
    out_of_date: bool,
}

pub type Swapchain = WgpuSwapchain;

impl WgpuSwapchain {
    /// Build a surface from a raw window handle. No display handle is required:
    /// the platform falls back to the instance's configured display.
    pub fn from_raw_handle(
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        handle: RawWindowHandle,
        width: u32,
        height: u32,
    ) -> Result<Self, RhiError> {
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: None,
                raw_window_handle: handle,
            })
        }
        .map_err(|error| RhiError::Unsupported(format!("wgpu create_surface: {error}")))?;

        let mut config = surface
            .get_default_config(adapter, width, height)
            .ok_or_else(|| {
                RhiError::Unsupported("surface is not supported by this adapter".to_owned())
            })?;
        config.usage |= wgpu::TextureUsages::RENDER_ATTACHMENT;

        Ok(Self {
            surface,
            config,
            current_texture: None,
            current_view: None,
            out_of_date: false,
        })
    }

    /// Build a surface from an already-created wgpu surface and config.
    pub fn from_surface(surface: wgpu::Surface<'static>, config: wgpu::SurfaceConfiguration) -> Self {
        Self {
            surface,
            config,
            current_texture: None,
            current_view: None,
            out_of_date: false,
        }
    }

    pub fn surface(&self) -> &wgpu::Surface<'static> {
        &self.surface
    }

    pub fn config(&self) -> &wgpu::SurfaceConfiguration {
        &self.config
    }

    pub fn extent(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn image_count(&self) -> u32 {
        2
    }

    pub fn is_out_of_date(&self) -> bool {
        self.out_of_date
    }

    pub fn set_out_of_date(&mut self, value: bool) {
        self.out_of_date = value;
    }

    /// Apply the current configuration to the surface. Panics are avoided by
    /// skipping zero-sized configs.
    pub fn configure(&mut self, device: &wgpu::Device) {
        if self.config.width == 0 || self.config.height == 0 {
            return;
        }
        self.surface.configure(device, &self.config);
        self.out_of_date = false;
    }

    pub fn reconfigure(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.config.width = width;
        self.config.height = height;
        self.configure(device);
    }

    /// Talk to the surface directly. The pull of the current texture happens in
    /// `WgpuGpu::acquire`; this is the storage side of it.
    pub fn take_current_texture(&mut self) -> Option<wgpu::SurfaceTexture> {
        self.current_texture.take()
    }

    pub fn set_current_texture(&mut self, texture: wgpu::SurfaceTexture) {
        self.current_texture = Some(texture);
    }

    pub fn set_current_view(&mut self, view: Option<wgpu::TextureView>) {
        self.current_view = view;
    }

    pub fn current_view(&self) -> Option<&wgpu::TextureView> {
        self.current_view.as_ref()
    }

    pub fn has_current_texture(&self) -> bool {
        self.current_texture.is_some()
    }
}

/// Map a wgpu create-surface error to a chassis error.
pub fn map_create_surface_error(error: wgpu::CreateSurfaceError) -> RhiError {
    RhiError::Unsupported(format!("wgpu create_surface: {error}"))
}
