use eframe::egui_wgpu::{RenderState, wgpu};
use photocraft_c3::Model;
use std::{num::NonZeroU64, sync::Arc};

pub struct Image {
    pub size: [u32; 2],
    pub pixels: Vec<u8>,
}
#[derive(Clone)]
struct Texture {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    clamped: wgpu::BindGroup,
    size: [u32; 2],
}
struct Geometry {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    sorted: wgpu::Buffer,
    count: u32,
    min: [f32; 3],
    max: [f32; 3],
    material: Option<(wgpu::Buffer, wgpu::BindGroup)>,
}
#[derive(PartialEq)]
struct AnimationKey {
    frames: Vec<(Option<u32>, u32, u32)>,
    rotation: [f32; 4],
    eye: [f32; 3],
    up_axis: u8,
    seed: u32,
    client_materials: bool,
    visible: Vec<bool>,
    texture_sizes: Vec<[u32; 2]>,
    framing: Option<([f32; 3], [f32; 3])>,
}
type NativeKey = (u32, u32, bool, bool, u8, bool);
type LiveTexture = (Vec<usize>, Vec<(usize, usize)>, Texture);

pub struct Gpu {
    rs: RenderState,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    clamp_sampler: wgpu::Sampler,
    material_layout: wgpu::BindGroupLayout,
    mix_textures: Vec<[Texture; 3]>,
    mix_fallback: Option<Texture>,
    uniform: wgpu::Buffer,
    view_bind: wgpu::BindGroup,
    opaque: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    edited: Option<Texture>,
    fallback: Option<Texture>,
    textures: Vec<Option<Texture>>,
    geometries: Vec<Option<Geometry>>,
    model: Option<Arc<Model>>,
    target: Option<(wgpu::Texture, wgpu::TextureView, wgpu::TextureView, egui::TextureId, [u32; 2])>,
    sorted_rotation: Option<([f32; 4], u8)>,
    live: std::collections::HashMap<usize, LiveTexture>,
    framing: Option<([f32; 3], [f32; 3])>,
    frame_model: bool,
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    native: std::collections::HashMap<NativeKey, wgpu::RenderPipeline>,
    posed: Vec<Option<photocraft_c3::animation::SampledMesh>>,
    animation_key: Option<AnimationKey>,
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
    pub fn fit(&mut self, model_only: bool) {
        self.framing = None;
        self.frame_model = model_only;
        self.animation_key = None;
    }
    pub fn invalidate_animation(&mut self) {
        self.animation_key = None;
    }
    fn prepare_materials(&mut self, loaded: &super::Loaded, seconds: f64, state: &super::State) {
        let Some(fallback) = &self.mix_fallback else {
            return;
        };
        for (index, geometry) in self.geometries.iter_mut().enumerate() {
            let Some(geometry) = geometry else {
                continue;
            };
            let material = loaded.materials.get(index).cloned().unwrap_or_default();
            let sprite = loaded.model.meshes.get(index).is_some_and(|mesh| matches!(mesh.variant.as_str(), "Particles" | "Trail"));
            let mode = if state.client_materials && !sprite { material.mix_mode } else { 0 };
            let sample = self.posed.get(index).and_then(Option::as_ref);
            let mut uv_steps = sample.map(|s| s.uv_steps).unwrap_or([[0.; 2]; 3]);
            if state.flip_v {
                for step in &mut uv_steps {
                    if let Some(v) = step.get_mut(1) {
                        *v = -*v;
                    }
                }
            }
            let style = loaded
                .animations
                .iter()
                .find(|a| a.range.contains(&index))
                .and_then(|a| a.asset.mesh_style(index.saturating_sub(a.range.start)))
                .unwrap_or_default();
            let values = fbytes(
                [mode as f32, seconds as f32, sample.map(|s| s.alpha).unwrap_or(1.), f32::from(style.skinned)]
                    .into_iter()
                    .chain(material.mix_data)
                    .chain(uv_steps.into_iter().flatten())
                    .chain([0.; 2]),
            );
            if geometry.material.is_none() {
                let uniform = buffer(&self.rs.device, &values, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
                let views = std::array::from_fn::<_, 3, _>(|stage| {
                    self.mix_textures.get(index).and_then(|stages| stages.get(stage)).unwrap_or(fallback).texture.create_view(&Default::default())
                });
                let bind = self.rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("c3 mixed texture material"),
                    layout: &self.material_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&views[0]) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                        wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&views[1]) },
                        wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&views[2]) },
                    ],
                });
                geometry.material = Some((uniform, bind));
            }
            if let Some((uniform, _)) = &geometry.material {
                self.rs.queue.write_buffer(uniform, 0, &values);
            }
        }
    }
    fn native_pipeline(&self, (src, dst, write, test, cull, reflected): NativeKey) -> wgpu::RenderPipeline {
        let factor = |value| match value {
            1 => wgpu::BlendFactor::Zero,
            2 => wgpu::BlendFactor::One,
            3 => wgpu::BlendFactor::Src,
            4 => wgpu::BlendFactor::OneMinusSrc,
            5 | 12 => wgpu::BlendFactor::SrcAlpha,
            6 | 13 => wgpu::BlendFactor::OneMinusSrcAlpha,
            7 => wgpu::BlendFactor::DstAlpha,
            8 => wgpu::BlendFactor::OneMinusDstAlpha,
            9 => wgpu::BlendFactor::Dst,
            10 => wgpu::BlendFactor::OneMinusDst,
            11 => wgpu::BlendFactor::SrcAlphaSaturated,
            14 => wgpu::BlendFactor::Constant,
            15 => wgpu::BlendFactor::OneMinusConstant,
            _ => wgpu::BlendFactor::One,
        };
        let dst = if src == 12 {
            6
        } else if src == 13 {
            5
        } else {
            dst
        };
        self.rs.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("c3 client material"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 56,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x4,4=>Float32x2],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                // C3 exterior triangles are clockwise in the preview's
                // right-handed camera convention. The selectable up-axis
                // reflections reverse that winding once more.
                front_face: if reflected { wgpu::FrontFace::Ccw } else { wgpu::FrontFace::Cw },
                cull_mode: match cull {
                    2 => Some(wgpu::Face::Back),
                    3 => Some(wgpu::Face::Front),
                    _ => None,
                },
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // Native depth-test disable also disables depth writes.
                depth_write_enabled: Some(write && test),
                depth_compare: Some(if test { wgpu::CompareFunction::LessEqual } else { wgpu::CompareFunction::Always }),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some("client"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent { src_factor: factor(src), dst_factor: factor(dst), operation: wgpu::BlendOperation::Add },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        })
    }
    fn animate(&mut self, loaded: &super::Loaded, seconds: f64, state: &super::State) {
        if loaded.animations.is_empty() {
            return;
        }
        let frames = loaded
            .animations
            .iter()
            .map(|animation| {
                (
                    animation.timing.frame(seconds, animation.asset.frame_count),
                    if animation.asset.is_animated() { animation.timing.step(seconds) } else { 0 },
                    if animation.asset.has_continuous_uv() { (seconds * 1000.).clamp(0., 86_400_000.) as u32 } else { 0 },
                )
            })
            .collect::<Vec<_>>();
        let key = AnimationKey {
            frames,
            rotation: state.camera.rotation,
            eye: [state.camera.distance, state.camera.pan[0], state.camera.pan[1]],
            up_axis: state.up_axis,
            seed: state.seed,
            client_materials: state.client_materials,
            visible: super::effects::visibility(loaded, state),
            texture_sizes: loaded.textures.iter().map(|image| image.as_ref().map(|image| image.size).unwrap_or([0; 2])).collect(),
            framing: self.framing,
        };
        if self.animation_key.as_ref() == Some(&key) {
            return;
        }
        let inverse = [-state.camera.rotation[0], -state.camera.rotation[1], -state.camera.rotation[2], state.camera.rotation[3]];
        let rotation = photocraft_c3::math::quaternion_matrix(inverse, [0.; 3]).unwrap_or(photocraft_c3::math::IDENTITY);
        let basis = |v| {
            let [x, y, z] = photocraft_c3::math::direction(v, &rotation);
            match state.up_axis {
                0 => [x, -z, -y],
                1 => [x, -z, y],
                2 => [x, y, z],
                _ => [x, -y, z],
            }
        };
        let billboard = [basis([1., 0., 0.]), basis([0., 1., 0.])];
        let (min, max) = self.framing.unwrap_or((loaded.model.min, loaded.model.max));
        let center = std::array::from_fn::<_, 3, _>(|i| (min[i] + max[i]) * 0.5);
        let radius = photocraft_c3::math::sub(max, min).iter().map(|v| v * v).sum::<f32>().sqrt().max(0.0001) * 0.5;
        let eye = basis([-state.camera.pan[0] * radius, -state.camera.pan[1] * radius, state.camera.distance * radius]);
        let eye = std::array::from_fn(|i| center[i] + eye[i]);
        let world_up = match state.up_axis {
            0 => [0., 0., -1.],
            1 => [0., 0., 1.],
            2 => [0., 1., 0.],
            _ => [0., -1., 0.],
        };
        let view = photocraft_c3::animation::View { axes: billboard, eye: Some(eye), world_up, effect_billboard: 0, visible: u128::MAX, texture_size: [0; 2] };
        for (index, sample) in super::effects::sample(loaded, seconds, view, state.seed, &key.visible) {
            let mesh_index = index;
            if let Some(Some(geometry)) = self.geometries.get_mut(index) {
                if !sample.vertices.is_empty() {
                    let suppress_normals = state.client_materials
                        && loaded.animations.iter().any(|animation| animation.effect.is_some() && animation.range.contains(&mesh_index))
                        && loaded.model.meshes.get(mesh_index).is_some_and(|mesh| matches!(mesh.variant.as_str(), "PHY" | "PHY4"));
                    let skinned = loaded
                        .animations
                        .iter()
                        .find(|animation| animation.range.contains(&mesh_index))
                        .and_then(|animation| animation.asset.mesh_style(mesh_index.saturating_sub(animation.range.start)))
                        .is_some_and(|style| style.skinned);
                    let vertices = fbytes(sample.vertices.iter().enumerate().flat_map(|(index, vertex)| {
                        let mut color = sample.colors.get(index).copied().unwrap_or([1.; 4]);
                        let mode = loaded.materials.get(mesh_index).map(|m| m.mix_mode).unwrap_or(0);
                        if !state.client_materials || !(matches!(mode, 8 | 34 | 36 | 39 | 40) || (mode == 23 && skinned)) {
                            color[3] *= sample.alpha;
                        }
                        vertex
                            .position
                            .into_iter()
                            .chain(if suppress_normals { [0.; 3] } else { vertex.normal })
                            .chain(vertex.uv)
                            .chain(color)
                            .chain(sample.uv1.get(index).copied().unwrap_or([0.; 2]))
                    }));
                    if vertices.len() as u64 <= geometry.vertices.size() {
                        self.rs.queue.write_buffer(&geometry.vertices, 0, &vertices);
                    }
                }
                let indices = ibytes(sample.indices.iter().copied());
                if indices.len() as u64 <= geometry.indices.size() && !indices.is_empty() {
                    self.rs.queue.write_buffer(&geometry.indices, 0, &indices);
                }
                geometry.count = sample.indices.len() as u32;
                geometry.min = [f32::INFINITY; 3];
                geometry.max = [f32::NEG_INFINITY; 3];
                for vertex in sample.indices.iter().filter_map(|index| sample.vertices.get(*index as usize)) {
                    for ((low, high), value) in geometry.min.iter_mut().zip(&mut geometry.max).zip(vertex.position) {
                        *low = low.min(value);
                        *high = high.max(value);
                    }
                }
            }
            if let Some(slot) = self.posed.get_mut(index) {
                *slot = Some(sample);
            }
        }
        self.sorted_rotation = None;
        self.animation_key = Some(key);
    }
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
        let clamp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("c3 particle clamp sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3 texture mixing"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: NonZeroU64::new(112) },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
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
            bind_group_layouts: &[Some(&view_layout), Some(&layout), Some(&material_layout)],
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
                        array_stride: 56,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x4,4=>Float32x2],
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
            clamp_sampler,
            material_layout,
            mix_textures: Vec::new(),
            mix_fallback: None,
            uniform,
            view_bind,
            opaque,
            transparent,
            edited: None,
            fallback: None,
            textures: Vec::new(),
            geometries: Vec::new(),
            model: None,
            target: None,
            sorted_rotation: None,
            live: Default::default(),
            framing: None,
            frame_model: false,
            shader,
            pipeline_layout,
            native: Default::default(),
            posed: Vec::new(),
            animation_key: None,
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
        let clamped = self.rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3 clamped material"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.clamp_sampler) },
            ],
        });
        Texture { texture, bind, clamped, size: image.size }
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
        if let Some(t) = self.fallback.as_ref().filter(|t| t.size == image.size) {
            self.write_texture(&t.texture, image);
        } else {
            self.fallback = Some(self.texture(image));
        }
        self.edited = self.fallback.clone();
    }
    pub fn model(&mut self, model: Arc<Model>, textures: &[Option<Arc<Image>>], mix: &[[Option<Arc<Image>>; 3]]) {
        self.geometries = model
            .meshes
            .iter()
            .map(|m| {
                if m.indices.is_empty() || m.vertices.is_empty() {
                    return None;
                }
                let vertices = fbytes(m.vertices.iter().flat_map(|v| v.position.into_iter().chain(v.normal).chain(v.uv).chain([1.; 4]).chain([0.; 2])));
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
                    vertices: buffer(&self.rs.device, &vertices, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST),
                    indices: buffer(&self.rs.device, &indices, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST),
                    sorted: buffer(&self.rs.device, &indices, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST),
                    count: m.indices.len() as u32,
                    material: None,
                    min,
                    max,
                })
            })
            .collect();
        self.live.clear();
        self.posed = vec![None; model.meshes.len()];
        self.animation_key = None;
        self.framing = Some((model.min, model.max));
        self.frame_model = false;
        self.secondary_textures(textures, mix);
        self.model = Some(model);
        self.sorted_rotation = None;
    }
    pub fn secondary_textures(&mut self, textures: &[Option<Arc<Image>>], mix: &[[Option<Arc<Image>>; 3]]) {
        let neutral = self.texture(&Image { size: [1, 1], pixels: vec![255; 4] });
        let white = self.texture(&Image { size: [1, 1], pixels: vec![255; 4] });
        let mut shared = std::collections::HashMap::new();
        self.textures = textures
            .iter()
            .map(|image| {
                Some(
                    image
                        .as_ref()
                        .map(|image| shared.entry(Arc::as_ptr(image)).or_insert_with(|| self.texture(image)).clone())
                        .unwrap_or_else(|| neutral.clone()),
                )
            })
            .collect();
        self.mix_textures = mix
            .iter()
            .map(|stages| {
                std::array::from_fn(|stage| {
                    stages
                        .get(stage)
                        .and_then(Option::as_ref)
                        .map(|image| shared.entry(Arc::as_ptr(image)).or_insert_with(|| self.texture(image)).clone())
                        .unwrap_or_else(|| white.clone())
                })
            })
            .collect();
        self.mix_fallback = Some(white);
        for (slots, mix, texture) in self.live.values() {
            for index in slots {
                if let Some(slot) = self.textures.get_mut(*index) {
                    *slot = Some(texture.clone());
                }
            }
            for (index, stage) in mix {
                if let Some(slot) = self.mix_textures.get_mut(*index).and_then(|stages| stages.get_mut(*stage)) {
                    *slot = texture.clone();
                }
            }
        }
        for geometry in self.geometries.iter_mut().flatten() {
            geometry.material = None;
        }
    }
    pub fn upload_slots(&mut self, asset: usize, slots: &[usize], mix: &[(usize, usize)], image: &Image) {
        let reused = self.live.get(&asset).is_some_and(|(_, _, texture)| texture.size == image.size);
        let texture = if let Some((_, _, texture)) = self.live.get(&asset).filter(|(_, _, texture)| texture.size == image.size) {
            self.write_texture(&texture.texture, image);
            texture.clone()
        } else {
            self.texture(image)
        };
        for index in slots {
            if let Some(slot) = self.textures.get_mut(*index) {
                *slot = Some(texture.clone());
            }
        }
        for (index, stage) in mix {
            if let Some(slot) = self.mix_textures.get_mut(*index).and_then(|stages| stages.get_mut(*stage)) {
                *slot = texture.clone();
            }
        }
        if !reused && !mix.is_empty() {
            for geometry in self.geometries.iter_mut().flatten() {
                geometry.material = None;
            }
        }
        self.live.insert(asset, (slots.to_vec(), mix.to_vec(), texture));
    }
    pub fn activate_texture(&mut self, asset: usize) {
        if let Some((_, _, texture)) = self.live.get(&asset) {
            self.edited = Some(texture.clone());
        }
    }
    pub fn forget_texture(&mut self, asset: usize, original: &[Option<Arc<Image>>], mix: &[[Option<Arc<Image>>; 3]]) {
        self.live.remove(&asset);
        self.secondary_textures(original, mix);
    }
    pub fn clear(&mut self) {
        self.model = None;
        self.geometries.clear();
        self.textures.clear();
        self.edited = None;
        self.fallback = None;
        self.live.clear();
        self.framing = None;
    }
    pub fn draw(&mut self, size: [u32; 2], state: &super::State, loaded: &super::Loaded, seconds: f64) -> Option<egui::TextureId> {
        self.animate(loaded, seconds, state);
        self.prepare_materials(loaded, seconds, state);
        let native_material = |index: usize| state.client_materials && loaded.animations.iter().any(|animation| animation.range.contains(&index));
        let mut client_order = (0..self.geometries.len()).collect::<Vec<_>>();
        client_order.sort_by_key(|index| {
            loaded
                .animations
                .iter()
                .enumerate()
                .find(|(_, a)| a.range.contains(index))
                .map(|(file, a)| (file, a.asset.mesh_style(index.saturating_sub(a.range.start)).map(|s| s.draw_order).unwrap_or(4)))
                .unwrap_or((usize::MAX, 4))
        });
        let selected = &loaded.selected;
        let mut visible = super::effects::visibility(loaded, state);
        for (index, sample) in self.posed.iter().enumerate() {
            let mode = loaded.materials.get(index).map(|material| material.mix_mode).unwrap_or(0);
            let skin = loaded.model.meshes.get(index).is_some_and(|mesh| !matches!(mesh.variant.as_str(), "Particles" | "Trail"));
            if sample
                .as_ref()
                .is_some_and(|sample| !sample.visible || (state.client_materials && skin && sample.alpha <= 0. && !matches!(mode, 8 | 36 | 39 | 40)))
                && let Some(slot) = visible.get_mut(index)
            {
                *slot = false;
            }
        }
        for (index, material) in loaded.materials.iter().enumerate() {
            let key = native_key(loaded, index, material, state.up_axis);
            if !self.native.contains_key(&key) {
                let pipeline = self.native_pipeline(key);
                self.native.insert(key, pipeline);
            }
        }
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
        if self.framing.is_none() {
            let model_visible = super::workspace::effective_visibility(loaded);
            let bounds = |model_only: bool| {
                let mut min = [f32::INFINITY; 3];
                let mut max = [f32::NEG_INFINITY; 3];
                for (i, g) in self.geometries.iter().enumerate() {
                    let include = if model_only {
                        model_visible.get(i).copied().unwrap_or(true)
                            && loaded.model.meshes.get(i).is_none_or(|mesh| !matches!(mesh.variant.as_str(), "Particles" | "Trail"))
                            && !loaded.animations.iter().any(|a| a.effect.is_some() && a.range.contains(&i))
                    } else {
                        visible.get(i).copied().unwrap_or(true)
                    };
                    if include && let Some(g) = g {
                        for (lo, v) in min.iter_mut().zip(g.min) {
                            *lo = lo.min(v);
                        }
                        for (hi, v) in max.iter_mut().zip(g.max) {
                            *hi = hi.max(v);
                        }
                    }
                }
                min.iter().chain(&max).all(|v| v.is_finite()).then_some((min, max))
            };
            // Fit model ignores attached effects without changing their visibility.
            // Effect-only files and completely hidden models fall back to the scene.
            self.framing = bounds(self.frame_model).or_else(|| bounds(false)).or(Some((model.min, model.max)));
        }
        // Keep the initial composition while isolating pieces or switching textures.
        let (min, max) = self.framing.unwrap_or((model.min, model.max));
        let center = std::array::from_fn::<_, 3, _>(|i| (min[i] + max[i]) * 0.5);
        let radius = photocraft_c3::math::sub(max, min).iter().map(|v| v * v).sum::<f32>().sqrt().max(0.0001) * 0.5;
        let uniforms = fbytes(
            camera
                .rotation
                .into_iter()
                .chain(center)
                .chain([1. / radius])
                .chain([camera.distance, camera.pan[0], camera.pan[1], 2.4142137 * camera.magnification * (size[0] as f32 / size[1] as f32).min(1.)])
                .chain([size[0] as f32 / size[1] as f32, f32::from(lit), f32::from(alpha), f32::from(flip_v)])
                .chain([f32::from(state.up_axis), 0., 0., 0.]),
        );
        self.rs.queue.write_buffer(&self.uniform, 0, &uniforms);
        // Sort translucent triangles far-to-near in view space. Opaque fragments
        // write depth first, so even transparent hair occludes correctly behind skin.
        if alpha == 0 && self.sorted_rotation != Some((camera.rotation, state.up_axis)) {
            let qmat = photocraft_c3::math::quaternion_matrix(camera.rotation, [0.; 3]).ok()?;
            for (index, (mesh, geometry)) in model.meshes.iter().zip(&self.geometries).enumerate() {
                if native_material(index) || !visible.get(index).copied().unwrap_or(true) {
                    continue;
                }
                let Some(geometry) = geometry else {
                    continue;
                };
                let (vertices, indices) =
                    self.posed.get(index).and_then(Option::as_ref).map(|sample| (&sample.vertices, &sample.indices)).unwrap_or((&mesh.vertices, &mesh.indices));
                let mut triangles: Vec<_> = indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|t| {
                        let z = t
                            .iter()
                            .filter_map(|i| vertices.get(*i as usize))
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
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(state.background[0]),
                            g: f64::from(state.background[1]),
                            b: f64::from(state.background[2]),
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
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
            pass.set_blend_constant(wgpu::Color::WHITE);
            for (pipeline, sorted) in [(&self.opaque, false), (&self.transparent, true)] {
                pass.set_pipeline(pipeline);
                for (i, geometry) in self.geometries.iter().enumerate() {
                    if !visible.get(i).copied().unwrap_or(true) || native_material(i) {
                        continue;
                    }
                    let Some(g) = geometry else {
                        continue;
                    };
                    let texture =
                        if selected.get(i).copied().unwrap_or(true) { edited } else { self.textures.get(i).and_then(Option::as_ref).unwrap_or(edited) };
                    let clamped = loaded.model.meshes.get(i).is_some_and(|mesh| matches!(mesh.variant.as_str(), "Particles" | "Trail"));
                    pass.set_bind_group(1, if clamped { &texture.clamped } else { &texture.bind }, &[]);
                    if let Some((_, bind)) = &g.material {
                        pass.set_bind_group(2, bind, &[]);
                    }
                    pass.set_vertex_buffer(0, g.vertices.slice(..));
                    pass.set_index_buffer(if sorted { g.sorted.slice(..) } else { g.indices.slice(..) }, wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..g.count, 0, 0..1);
                }
            }
            if state.client_materials {
                for i in client_order {
                    if !visible.get(i).copied().unwrap_or(true) || !native_material(i) {
                        continue;
                    }
                    let Some(g) = self.geometries.get(i).and_then(Option::as_ref) else {
                        continue;
                    };
                    let material = loaded.materials.get(i).cloned().unwrap_or_default();
                    let key = native_key(loaded, i, &material, state.up_axis);
                    let Some(pipeline) = self.native.get(&key) else {
                        continue;
                    };
                    let texture =
                        if selected.get(i).copied().unwrap_or(false) { edited } else { self.textures.get(i).and_then(Option::as_ref).unwrap_or(edited) };
                    pass.set_pipeline(pipeline);
                    let clamped = loaded.model.meshes.get(i).is_some_and(|mesh| matches!(mesh.variant.as_str(), "Particles" | "Trail"));
                    pass.set_bind_group(1, if clamped { &texture.clamped } else { &texture.bind }, &[]);
                    if let Some((_, bind)) = &g.material {
                        pass.set_bind_group(2, bind, &[]);
                    }
                    pass.set_vertex_buffer(0, g.vertices.slice(..));
                    pass.set_index_buffer(g.indices.slice(..), wgpu::IndexFormat::Uint32);
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

fn native_key(loaded: &super::Loaded, index: usize, material: &photocraft_c3::material::Material, axis: u8) -> NativeKey {
    let two_sided = loaded
        .animations
        .iter()
        .find(|animation| animation.range.contains(&index))
        .and_then(|animation| animation.asset.mesh_style(index.saturating_sub(animation.range.start)))
        .is_some_and(|style| style.two_sided);
    let cull = if material.cull == 0 { if two_sided { 1 } else { 2 } } else { material.cull };
    let sprite = loaded.model.meshes.get(index).is_some_and(|mesh| matches!(mesh.variant.as_str(), "Particles" | "Trail"));
    (material.blend[0], material.blend[1], material.depth_write && !sprite, material.depth_test, cull, matches!(axis, 0 | 3))
}
