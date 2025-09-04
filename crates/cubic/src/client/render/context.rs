use std::time::{SystemTime, UNIX_EPOCH};

pub struct Context {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    preferred_format: wgpu::TextureFormat,
}

impl Context {
    pub fn new<W>(window: W) -> impl Future<Output = Self>
    where
        W: raw_window_handle::HasWindowHandle
            + raw_window_handle::HasDisplayHandle
            + Send
            + Sync
            + 'static,
    {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance.create_surface(window).unwrap();

        async move {
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    ..Default::default()
                })
                .await
                .unwrap();

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .unwrap();

            let preferred_format = surface.get_capabilities(&adapter).formats[0];
            Self {
                surface,
                device,
                queue,
                preferred_format,
            }
        }
    }

    pub fn resize(&self, width: u32, height: u32) {
        self.surface.configure(
            &self.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.preferred_format,
                width,
                height,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![self.preferred_format],
            },
        );
    }

    pub fn draw(&self) {
        let output = self.surface.get_current_texture().unwrap();
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("encoder"),
            });

        let time_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        let red_freq = 0.5; // Red oscillates slowly
        let green_freq = 0.7; // Green oscillates at medium speed  
        let blue_freq = 1.0; // Blue oscillates faster

        // Phase offsets to prevent all colors from being in sync
        let red_phase = 0.0;
        let green_phase = std::f64::consts::PI * 2.0 / 3.0; // 120 degrees
        let blue_phase = std::f64::consts::PI * 4.0 / 3.0; // 240 degrees

        // Generate sine waves and map to 0-255 range
        let r = (time_seconds * red_freq + red_phase).sin();
        let g = (time_seconds * green_freq + green_phase).sin();
        let b = (time_seconds * blue_freq + blue_phase).sin();

        {
            let _rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rp"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
}
