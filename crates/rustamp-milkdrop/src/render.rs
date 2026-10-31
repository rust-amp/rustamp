// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! One frame: the engine renders into a texture, and the pixels come back.

use lactea::{Engine, EngineConfig, EngineError, FrameClock, TextureProvider, Transition};

use crate::gpu::Gpu;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// wgpu's rule for a buffer that receives a texture: rows on 256-byte steps.
fn padded_row(width: u32) -> u32 {
    (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

/// The engine and the texture and buffer of the current size.
pub(crate) struct Renderer {
    gpu: Gpu,
    engine: Engine,
    target: Target,
}

struct Target {
    size: (u32, u32),
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

impl Target {
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("milkdrop target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("milkdrop readback"),
            size: u64::from(padded_row(size.0)) * u64::from(size.1),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            size,
            texture,
            view,
            readback,
        }
    }
}

impl Renderer {
    /// An engine of `size` on `gpu`. `seed` drives the engine's random streams.
    pub(crate) fn new(
        gpu: Gpu,
        size: (u32, u32),
        fps: u32,
        seed: u64,
        textures: Option<Box<dyn TextureProvider>>,
    ) -> Result<Self, EngineError> {
        let mut engine = Engine::new(
            &gpu.device,
            &gpu.queue,
            EngineConfig {
                width: size.0,
                height: size.1,
                seed,
                fps,
                output_format: FORMAT,
                // MilkDrop 3 files are recognised by their contents, and what
                // projectM's rules refuse is tried once under MilkDrop 2's.
                auto_compat: true,
                ..EngineConfig::default()
            },
        )?;
        if let Some(textures) = textures {
            engine.set_texture_provider(textures);
        }
        let target = Target::new(&gpu.device, size);
        Ok(Self {
            gpu,
            engine,
            target,
        })
    }

    /// The name of the adapter in use.
    pub(crate) fn adapter(&self) -> &str {
        &self.gpu.name
    }

    /// Whether the frames are rendered on the CPU.
    pub(crate) fn is_software(&self) -> bool {
        self.gpu.software
    }

    /// The device, for its error hooks.
    pub(crate) fn device(&self) -> &wgpu::Device {
        &self.gpu.device
    }

    /// The engine, to load presets, feed audio and read the analysis.
    pub(crate) fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// Loads a preset; the notes of the engine's report go to `note`.
    pub(crate) fn load(
        &mut self,
        bytes: &[u8],
        name: &str,
        transition: Transition,
        note: &mut dyn FnMut(String),
    ) -> Result<(), String> {
        let report = self
            .engine
            .load_preset(bytes, name, transition)
            .map_err(|error| error.to_string())?;
        for fallback in &report.shader_fallbacks {
            note(format!(
                "{name}: the {:?} shader was replaced ({}): {}",
                fallback.pass, fallback.fallback, fallback.reason
            ));
        }
        for texture in &report.unresolved_textures {
            note(format!("{name}: texture {texture} was not found"));
        }
        for warning in &report.warnings {
            note(format!("{name}: {warning}"));
        }
        Ok(())
    }

    /// Follows a new size. Nothing happens when it is the current one.
    pub(crate) fn resize(&mut self, size: (u32, u32)) -> Result<(), EngineError> {
        if size == self.target.size {
            return Ok(());
        }
        self.engine.resize(size.0, size.1)?;
        self.target = Target::new(&self.gpu.device, size);
        Ok(())
    }

    /// Renders one frame and returns it: `width`, `height`, tightly packed RGBA.
    pub(crate) fn render(&mut self, clock: FrameClock) -> Result<(u32, u32, Vec<u8>), String> {
        let (width, height) = self.target.size;
        let padded = padded_row(width);
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.engine.render(&mut encoder, &self.target.view, clock);
        encoder.copy_texture_to_buffer(
            self.target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.target.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue.submit(Some(encoder.finish()));

        let slice = self.target.readback.slice(..);
        let (sender, receiver) = crossbeam_channel::bounded(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            // The receiver is waited on below, so this send cannot be lost
            // unless that wait has already given up.
            let _ = sender.send(result);
        });
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| error.to_string())?;
        receiver
            .recv()
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let mapped = slice
            .get_mapped_range()
            .map_err(|error| error.to_string())?;
        let row = width as usize * 4;
        let mut pixels = Vec::with_capacity(row * height as usize);
        for line in mapped.chunks_exact(padded as usize) {
            pixels.extend_from_slice(&line[..row]);
        }
        drop(mapped);
        self.target.readback.unmap();
        Ok((width, height, pixels))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_padded_to_the_copy_alignment() {
        assert_eq!(padded_row(64), 256, "exactly one step");
        assert_eq!(padded_row(65), 512, "one pixel over");
        assert_eq!(padded_row(640), 2560);
        assert_eq!(padded_row(1), 256);
    }
}
