use imgui::internal::RawWrapper;
use imgui::DrawIdx;
use imgui::DrawVert;

const MAX_INDEX_BUFFER_SIZE: u64 = 1024*1024;
const MAX_VERTEX_BUFFER_SIZE: u64 = 1024*1024;

#[derive(Clone, Copy)]
struct Vertex(DrawVert);

unsafe impl bytemuck::Zeroable for Vertex {}

unsafe impl bytemuck::Pod for Vertex {}

macro_rules! size_of {
    ($T:ty) => {
        std::mem::size_of::<$T>()
    };
}
macro_rules! offset_of {
    ($T:ty,$field:tt) => {{
        let elem: $T = std::mem::zeroed();
        &elem.$field as *const _ as usize - &elem as *const _ as usize
    }};
}

struct Texture {
    bind_group: wgpu::BindGroup,
}
impl Texture {
    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bind_group_layout: &wgpu::BindGroupLayout,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Self {
        let texture_extent = wgpu::Extent3d {
            width,
            height,
            depth: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: texture_extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsage::SAMPLED | wgpu::TextureUsage::COPY_DST,
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        queue.write_texture(
            wgpu::TextureCopyView {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
            },
            pixels,
            wgpu::TextureDataLayout {
                offset: 0,
                bytes_per_row: (width * 4) as u32,
                rows_per_image: 0,
            },
            texture_extent,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
            label: None,
        });
        Self { bind_group }
    }
}

pub struct Renderer {
    texture_bind_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    index_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    uniform_buffer_bind_group: wgpu::BindGroup,
    indices_byte_buffer: Vec<u8>,
    vertices_byte_buffer: Vec<u8>,
    textures: imgui::Textures<Texture>,
}
impl Renderer {
    pub fn upload_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> imgui::TextureId {
        let texture = Texture::new(
            device,
            queue,
            &self.texture_bind_layout,
            width,
            height,
            data,
        );
        self.textures.insert(texture)
    }
    pub fn reload_font_texture(
        &mut self,
        imgui: &mut imgui::Context,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let mut fonts = imgui.fonts();

        self.textures.remove(fonts.tex_id);

        let texture_data = fonts.build_rgba32_texture();
        let texture = Texture::new(
            device,
            queue,
            &self.texture_bind_layout,
            texture_data.width,
            texture_data.height,
            texture_data.data,
        );
        fonts.tex_id = self.textures.insert(texture);
        fonts.clear_tex_data();
    }
    pub fn render<'a>(
        &'a mut self,
        queue: &wgpu::Queue,
        render_pass: &mut wgpu::RenderPass<'a>,
        draw_data: &imgui::DrawData,
    ) {
        let left = draw_data.display_pos[0];
        let right = draw_data.display_pos[0] + draw_data.display_size[0];
        let top = draw_data.display_pos[1];
        let bottom = draw_data.display_pos[1] + draw_data.display_size[1];
        let matrix = [
            (2.0 / (right - left)),
            0.0,
            0.0,
            0.0,
            0.0,
            (2.0 / (top - bottom)),
            0.0,
            0.0,
            0.0,
            0.0,
            -1.0,
            0.0,
            (right + left) / (left - right),
            (top + bottom) / (bottom - top),
            0.0,
            1.0,
        ];
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&matrix));
        let mut offsets = Vec::<(u64, u64)>::new();
        for draw_list in draw_data.draw_lists() {
            offsets.push((
                self.append_indices(draw_list.idx_buffer()).unwrap(),
                self.append_vertices(draw_list.vtx_buffer()).unwrap(),
            ))
        }
        self.upload_buffers(queue);
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_index_buffer(self.index_buffer.slice(..));
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_bind_group(0, &self.uniform_buffer_bind_group, &[]);
        for draw_list in draw_data.draw_lists() {
            let (idx_offset, vtx_offset) = *offsets.first().unwrap();
            offsets.remove(0);
            let mut idx_begin = idx_offset as u32;
            for draw_cmd in draw_list.commands() {
                match draw_cmd {
                    imgui::DrawCmd::Elements { count, cmd_params } => {
                        let scissor = (
                            cmd_params.clip_rect[0].max(0.0).floor() as u32,
                            cmd_params.clip_rect[1].max(0.0).floor() as u32,
                            (cmd_params.clip_rect[2] - cmd_params.clip_rect[0])
                                .abs()
                                .ceil() as u32,
                            (cmd_params.clip_rect[3] - cmd_params.clip_rect[1])
                                .abs()
                                .ceil() as u32,
                        );
                        render_pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
                        let texture = self.textures.get(cmd_params.texture_id).unwrap();
                        render_pass.set_bind_group(1, texture.bind_group(), &[]);
                        let idx_end = idx_begin + count as u32;
                        render_pass.draw_indexed(idx_begin..idx_end, vtx_offset as i32, 0..1);
                        idx_begin = idx_end;
                    }
                    imgui::DrawCmd::RawCallback { callback, raw_cmd } => unsafe {
                        callback(draw_list.raw(), raw_cmd);
                    },
                    _ => {}
                }
            }
        }
    }
    pub fn new(
        imgui: &mut imgui::Context,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        swap_chain_texture_format: wgpu::TextureFormat,
    ) -> Self {
        let uniform_buffer_bind_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: wgpu::BindingType::UniformBuffer {
                        dynamic: false,
                        min_binding_size: wgpu::BufferSize::new(4 * 16),
                    },
                    count: None,
                }],
            });
        let texture_bind_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStage::FRAGMENT,
                        ty: wgpu::BindingType::SampledTexture {
                            multisampled: false,
                            component_type: wgpu::TextureComponentType::Float,
                            dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStage::FRAGMENT,
                        ty: wgpu::BindingType::Sampler { comparison: false },
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&uniform_buffer_bind_layout, &texture_bind_layout],
            push_constant_ranges: &[],
        });

        let vs_module = device.create_shader_module(wgpu::include_spirv!("imgui.vert.spv"));
        let fs_module = device.create_shader_module(wgpu::include_spirv!("imgui.frag.spv"));

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex_stage: wgpu::ProgrammableStageDescriptor {
                module: &vs_module,
                entry_point: "main",
            },
            fragment_stage: Some(wgpu::ProgrammableStageDescriptor {
                module: &fs_module,
                entry_point: "main",
            }),
            rasterization_state: Some(wgpu::RasterizationStateDescriptor {
                front_face: wgpu::FrontFace::Cw,
                cull_mode: wgpu::CullMode::None,
                ..Default::default()
            }),
            primitive_topology: wgpu::PrimitiveTopology::TriangleList,
            color_states: &[wgpu::ColorStateDescriptor {
                format: swap_chain_texture_format,
                color_blend: wgpu::BlendDescriptor {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha_blend: wgpu::BlendDescriptor {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Add,
                },
                write_mask: wgpu::ColorWrite::ALL,
            }],
            depth_stencil_state: None,
            vertex_state: wgpu::VertexStateDescriptor {
                index_format: wgpu::IndexFormat::Uint16,
                vertex_buffers: &[wgpu::VertexBufferDescriptor {
                    stride: size_of!(DrawVert) as wgpu::BufferAddress,
                    step_mode: wgpu::InputStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttributeDescriptor {
                            format: wgpu::VertexFormat::Float2,
                            offset: unsafe { offset_of!(DrawVert, pos) } as u64,
                            shader_location: 0,
                        },
                        wgpu::VertexAttributeDescriptor {
                            format: wgpu::VertexFormat::Float2,
                            offset: unsafe { offset_of!(DrawVert, uv) } as u64,
                            shader_location: 1,
                        },
                        wgpu::VertexAttributeDescriptor {
                            format: wgpu::VertexFormat::Uint,
                            offset: unsafe { offset_of!(DrawVert, col) } as u64,
                            shader_location: 2,
                        },
                    ],
                }],
            },
            sample_count: 1,
            sample_mask: !0,
            alpha_to_coverage_enabled: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: MAX_INDEX_BUFFER_SIZE,
            usage: wgpu::BufferUsage::INDEX | wgpu::BufferUsage::COPY_DST,
            mapped_at_creation: false,
        });
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: MAX_VERTEX_BUFFER_SIZE,
            usage: wgpu::BufferUsage::VERTEX | wgpu::BufferUsage::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: size_of!(f32) as u64 * 16,
            usage: wgpu::BufferUsage::UNIFORM | wgpu::BufferUsage::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_buffer_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &uniform_buffer_bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(uniform_buffer.slice(..)),
            }],
            label: None,
        });
        let mut renderer = Self {
            texture_bind_layout,
            pipeline,
            index_buffer,
            vertex_buffer,
            uniform_buffer,
            uniform_buffer_bind_group,
            indices_byte_buffer: Vec::with_capacity(MAX_INDEX_BUFFER_SIZE as usize),
            vertices_byte_buffer: Vec::with_capacity(MAX_VERTEX_BUFFER_SIZE as usize),
            textures: imgui::Textures::<Texture>::new(),
        };
        renderer.reload_font_texture(imgui, device, queue);
        renderer
    }
    fn upload_buffers(&mut self, queue: &wgpu::Queue) {
        let indices_byte_length = self.indices_byte_buffer.len();
        self.indices_byte_buffer
            .resize(indices_byte_length + (4 - indices_byte_length % 4), 0);
        queue.write_buffer(&self.index_buffer, 0, self.indices_byte_buffer.as_slice());

        let vertices_byte_length = self.vertices_byte_buffer.len();
        self.vertices_byte_buffer
            .resize(vertices_byte_length + (4 - vertices_byte_length % 4), 0);

        queue.write_buffer(&self.vertex_buffer, 0, self.vertices_byte_buffer.as_slice());
        self.indices_byte_buffer.resize(0, 0);
        self.vertices_byte_buffer.resize(0, 0);
    }
    fn append_indices(&mut self, indices: &[DrawIdx]) -> Option<u64> {
        let offset = self.indices_byte_buffer.len();
        let bytes: &[u8] = bytemuck::cast_slice(indices);
        if offset + bytes.len() < MAX_INDEX_BUFFER_SIZE as usize {
            self.indices_byte_buffer.extend_from_slice(bytes);
            Some((offset / size_of!(DrawIdx)) as u64)
        } else {
            None
        }
    }
    fn append_vertices(&mut self, vertices: &[DrawVert]) -> Option<u64> {
        let offset = self.vertices_byte_buffer.len();
        let vertices =
            unsafe { std::slice::from_raw_parts(vertices.as_ptr() as *mut Vertex, vertices.len()) };
        let bytes: &[u8] = bytemuck::cast_slice(vertices);
        if offset + bytes.len() < MAX_VERTEX_BUFFER_SIZE as usize {
            self.vertices_byte_buffer.extend_from_slice(bytes);
            Some((offset / size_of!(DrawVert)) as u64)
        } else {
            None
        }
    }
}
