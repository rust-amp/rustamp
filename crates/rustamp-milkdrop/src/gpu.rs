// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Opening a graphics device: a real GPU when there is one, the system's
//! software adapter (Direct3D 12's WARP on Windows) when there is not, and a
//! plain error when neither exists. No window is involved.

use thiserror::Error;

/// A device to render with.
pub(crate) struct Gpu {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    /// The adapter's name, for the notes.
    pub(crate) name: String,
    /// Whether the adapter renders on the CPU.
    pub(crate) software: bool,
}

/// Why no device could be opened.
#[derive(Debug, Error)]
pub(crate) enum GpuError {
    /// No adapter, real or software.
    #[error("no graphics adapter was found: {0}")]
    NoAdapter(String),
    /// The adapter refused to open a device.
    #[error("the graphics adapter {adapter} could not be opened: {reason}")]
    NoDevice { adapter: String, reason: String },
}

impl Gpu {
    /// Opens a device on `backends`. `software_only` skips a real GPU.
    pub(crate) fn open(backends: wgpu::Backends, software_only: bool) -> Result<Self, GpuError> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = backends;
        let instance = wgpu::Instance::new(descriptor);
        let request = |force_fallback_adapter| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter,
                compatible_surface: None,
                ..wgpu::RequestAdapterOptions::default()
            }))
        };
        let adapter = if software_only {
            request(true)
        } else {
            request(false).or_else(|_| request(true))
        }
        .map_err(|error| GpuError::NoAdapter(error.to_string()))?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rustamp-milkdrop"),
            required_limits: adapter.limits(),
            ..wgpu::DeviceDescriptor::default()
        }))
        .map_err(|error| GpuError::NoDevice {
            adapter: info.name.clone(),
            reason: error.to_string(),
        })?;
        Ok(Self {
            device,
            queue,
            software: info.device_type == wgpu::DeviceType::Cpu,
            name: info.name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_backend_means_no_adapter_and_a_plain_error() {
        let Err(error) = Gpu::open(wgpu::Backends::empty(), false) else {
            panic!("a device without a backend");
        };
        assert!(matches!(error, GpuError::NoAdapter(_)), "{error}");
        assert!(error.to_string().contains("no graphics adapter"));
    }
}
