use eframe::egui_wgpu::{RenderState, wgpu};
use photocraft_c3::Model;
use std::{num::NonZeroU64, sync::Arc};

pub struct Image {
    pub size: [u32; 2],
    pub pixels: Vec<u8>,
}
struct Texture {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    size: [u32; 2],
}
struct Geometry {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    sorted: wgpu::Buffer,
    count: u32,
    min: [f32; 3],
    max: [f32; 3],
}
pub struct Gpu {
    rs: RenderState,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    view_bind: wgpu::BindGroup,
    opaque: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    edited: Option<Texture>,
    textures: Vec<Option<Texture>>,
    geometries: Vec<Option<Geometry>>,
    model: Option<Arc<Model>>,
    target: Option<(wgpu::Texture, wgpu::TextureView, wgpu::TextureView, egui::TextureId, [u32; 2])>,
    sorted_rotation: Option<([f32; 4], u8)>,
}
fn fbytes(values: impl IntoIterator<Item = f32>) -> Vec<u8> {
    values.into_iter().flat_map(f32::to_ne_bytes).collect()
}
fn ibytes(values: impl IntoIterator<Item = u32>) -> Vec<u8> {
    values.into_iter().flat_map(u32::to_ne_bytes).collect()
}
fn buffer(device: &wgpu::Device, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("c3 geometry"), contents: data, usage })
}
impl Gpu {
    pub fn new(rs: RenderState) -> Self {
        let device = &rs.device;
        let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3 view"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: NonZeroU64::new(80) },
                count: None,
            }],
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3 material"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("c3 repeat sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3 camera"),
            size: 80,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3 camera"),
            layout: &view_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3 preview"),
            source: wgpu::ShaderSource::Wgsl(include_str!("preview.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("c3 preview"),
            bind_group_layouts: &[Some(&view_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("c3 preview"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 32,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2],
                    })],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = pipeline("opaque", true);
        let transparent = pipeline("transparent", false);
        Self {
            rs,
            layout,
            sampler,
            uniform,
            view_bind,
            opaque,
            transparent,
            edited: None,
            textures: Vec::new(),
            geometries: Vec::new(),
            model: None,
            target: None,
            sorted_rotation: None,
        }
    }
    fn texture(&self, image: &Image) -> Texture {
        let texture = self.rs.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("c3 texture"),
            size: wgpu::Extent3d { width: image.size[0], height: image.size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.write_texture(&texture, image);
        let view = texture.create_view(&Default::default());
        let bind = self.rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3 material"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        Texture { texture, bind, size: image.size }
    }
    fn write_texture(&self, texture: &wgpu::Texture, image: &Image) {
        self.rs.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &image.pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(image.size[0] * 4), rows_per_image: Some(image.size[1]) },
            wgpu::Extent3d { width: image.size[0], height: image.size[1], depth_or_array_layers: 1 },
        );
    }
    pub fn upload(&mut self, image: &Image) {
        if let Some(t) = self.edited.as_ref().filter(|t| t.size == image.size) {
            self.write_texture(&t.texture, image);
        } else {
            self.edited = Some(self.texture(image));
        }
    }
    pub fn model(&mut self, model: Arc<Model>, textures: &[Option<Image>]) {
        self.geometries = model
            .meshes
            .iter()
            .map(|m| {
                if m.indices.is_empty() || m.vertices.is_empty() {
                    return None;
                }
                let vertices = fbytes(m.vertices.iter().flat_map(|v| v.position.into_iter().chain(v.normal).chain(v.uv)));
                let indices = ibytes(m.indices.iter().copied());
                let mut min = [f32::INFINITY; 3];
                let mut max = [f32::NEG_INFINITY; 3];
                for v in m.indices.iter().filter_map(|i| m.vertices.get(*i as usize)) {
                    for ((lo, hi), p) in min.iter_mut().zip(&mut max).zip(v.position) {
                        *lo = lo.min(p);
                        *hi = hi.max(p);
                    }
                }
                Some(Geometry {
                    vertices: buffer(&self.rs.device, &vertices, wgpu::BufferUsages::VERTEX),
                    indices: buffer(&self.rs.device, &indices, wgpu::BufferUsages::INDEX),
                    sorted: buffer(&self.rs.device, &indices, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST),
                    count: m.indices.len() as u32,
                    min,
                    max,
                })
            })
            .collect();
        let neutral = Image { size: [1, 1], pixels: vec![180, 180, 180, 255] };
        self.textures = textures.iter().map(|i| Some(self.texture(i.as_ref().unwrap_or(&neutral)))).collect();
        self.model = Some(model);
        self.sorted_rotation = None;
    }
    pub fn secondary_textures(&mut self, textures: &[Option<Image>]) {
        self.textures = textures.iter().map(|image| image.as_ref().map(|image| self.texture(image))).collect();
    }
    pub fn clear(&mut self) {
        self.model = None;
        self.geometries.clear();
        self.textures.clear();
        self.edited = None;
    }
    pub fn draw(&mut self, size: [u32; 2], state: &super::State, selected: &[bool], visible: &[bool]) -> Option<egui::TextureId> {
        let (camera, lit, alpha, flip_v) = (&state.camera, state.lit, state.alpha, state.flip_v);
        let model = self.model.as_ref()?;
        let edited = self.edited.as_ref()?;
        let max = self.rs.device.limits().max_texture_dimension_2d.min(2048);
        let size = size.map(|s| s.clamp(1, max));
        if self.target.as_ref().is_none_or(|t| t.4 != size) {
            let texture = self.rs.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("c3 viewport"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let depth = self.rs.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("c3 depth"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let depth = depth.create_view(&Default::default());
            let mut renderer = self.rs.renderer.write();
            let id = match self.target.as_ref() {
                Some(t) => {
                    renderer.update_egui_texture_from_wgpu_texture(&self.rs.device, &view, wgpu::FilterMode::Linear, t.3);
                    t.3
                }
                None => renderer.register_native_texture(&self.rs.device, &view, wgpu::FilterMode::Linear),
            };
            self.target = Some((texture, view, depth, id, size));
        }
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for (i, g) in self.geometries.iter().enumerate().filter(|(i, _)| visible.get(*i).copied().unwrap_or(true)) {
            let _ = i;
            if let Some(g) = g {
                for (lo, v) in min.iter_mut().zip(g.min) {
                    *lo = lo.min(v);
                }
                for (hi, v) in max.iter_mut().zip(g.max) {
                    *hi = hi.max(v);
                }
            }
        }
        if min.iter().any(|v| !v.is_finite()) {
            min = model.min;
            max = model.max;
        }
        let center = std::array::from_fn::<_, 3, _>(|i| (min[i] + max[i]) * 0.5);
        let radius = photocraft_c3::math::sub(max, min).iter().map(|v| v * v).sum::<f32>().sqrt().max(0.0001) * 0.5;
        let uniforms = fbytes(
            camera
                .rotation
                .into_iter()
                .chain(center)
                .chain([1. / radius])
                .chain([camera.distance, camera.pan[0], camera.pan[1], 2.4142137 * (size[0] as f32 / size[1] as f32).min(1.)])
                .chain([size[0] as f32 / size[1] as f32, f32::from(lit), f32::from(alpha), f32::from(flip_v)])
                .chain([f32::from(state.up_axis), 0., 0., 0.]),
        );
        self.rs.queue.write_buffer(&self.uniform, 0, &uniforms);
        // Sort translucent triangles far-to-near in view space. Opaque fragments
        // write depth first, so even transparent hair occludes correctly behind skin.
        if alpha == 0 && self.sorted_rotation != Some((camera.rotation, state.up_axis)) {
            let qmat = photocraft_c3::math::quaternion_matrix(camera.rotation, [0.; 3]).ok()?;
            for (mesh, geometry) in model.meshes.iter().zip(&self.geometries) {
                let Some(geometry) = geometry else {
                    continue;
                };
                let mut triangles: Vec<_> = mesh
                    .indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|t| {
                        let z = t
                            .iter()
                            .filter_map(|i| mesh.vertices.get(*i as usize))
                            .map(|v| {
                                let [x, y, z] = v.position;
                                let p = match state.up_axis {
                                    0 => [x, -z, -y],
                                    1 => [x, z, -y],
                                    2 => [x, y, z],
                                    _ => [x, -y, z],
                                };
                                photocraft_c3::math::transform(p, &qmat)[2]
                            })
                            .sum::<f32>();
                        (z, t)
                    })
                    .collect();
                triangles.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
                let bytes = ibytes(triangles.into_iter().flat_map(|(_, t)| t.iter().copied()));
                self.rs.queue.write_buffer(&geometry.sorted, 0, &bytes);
            }
            self.sorted_rotation = Some((camera.rotation, state.up_axis));
        }
        let target = self.target.as_ref()?;
        let mut encoder = self.rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("c3 preview") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("c3 preview"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.1,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.2,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.view_bind, &[]);
            for (pipeline, sorted) in [(&self.opaque, false), (&self.transparent, true)] {
                pass.set_pipeline(pipeline);
                for (i, geometry) in self.geometries.iter().enumerate() {
                    if !visible.get(i).copied().unwrap_or(true) {
                        continue;
                    }
                    let Some(g) = geometry else {
                        continue;
                    };
                    let texture =
                        if selected.get(i).copied().unwrap_or(true) { edited } else { self.textures.get(i).and_then(Option::as_ref).unwrap_or(edited) };
                    pass.set_bind_group(1, &texture.bind, &[]);
                    pass.set_vertex_buffer(0, g.vertices.slice(..));
                    pass.set_index_buffer(if sorted { g.sorted.slice(..) } else { g.indices.slice(..) }, wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..g.count, 0, 0..1);
                }
            }
        }
        self.rs.queue.submit([encoder.finish()]);
        Some(target.3)
    }
}
impl Drop for Gpu {
    fn drop(&mut self) {
        if let Some(t) = self.target.take() {
            self.rs.renderer.write().free_texture(&t.3);
        }
    }
}
