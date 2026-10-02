use thiserror::Error;

#[derive(Debug, Error)]
pub enum GfxError {
    #[error("failed to create surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("no suitable GPU adapter: {0}")]
    NoAdapter(#[from] wgpu::RequestAdapterError),
    #[error("failed to create device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("the surface is not supported by the selected adapter")]
    UnsupportedSurface,
    #[error("texture {width}x{height} exceeds the device limit of {max}")]
    TextureTooLarge { width: u32, height: u32, max: u32 },
    #[error(
        "invalid texture data: expected {expected} bytes for {width}x{height} RGBA8, got {actual}"
    )]
    InvalidTextureData {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },
}
