use std::{
    cell::RefCell,
    collections::HashMap,
    fmt::Write as _,
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Instant, SystemTime},
};

use iced::{
    Element, Rectangle, wgpu,
    wgpu::util::DeviceExt as _,
    widget::shader::{self, Pipeline, Primitive, Program},
};
use mlua::{Table, Value};
use riced_extension_sdk::log;

use crate::runtime::Message;

use super::{Ctx, Layout};

const MAX_SOURCE: u64 = 1024 * 1024;
const FILE_CACHE: usize = 32;
const PIPELINE_CACHE: usize = 16;
const VALIDATION_CACHE: usize = 16;

const HEADER: &str = r#"
struct RicedVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn riced_vertex(@builtin(vertex_index) index: u32) -> RicedVertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[index];
    var output: RicedVertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = position * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return output;
}
"#;

struct CachedFile {
    modified: Option<SystemTime>,
    len: u64,
    source: Arc<str>,
}

thread_local! {
    static FILES: RefCell<HashMap<PathBuf, CachedFile>> = RefCell::new(HashMap::new());
    static VALIDATED: RefCell<HashMap<u64, Result<(), String>>> =
        RefCell::new(HashMap::new());
}

pub(super) fn shader(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let Some(fragment) = source(node, ctx)? else {
        return Ok(iced::widget::Space::new().into());
    };
    let extra = extra_args(node)?;
    let textures = textures(node, ctx)?;
    let source: Arc<str> = format!("{}\n{fragment}", full_header(&extra)).into();
    if let Err(error) = validate(&source) {
        log::error!(target: "shader", "{error}");
        return Ok(iced::widget::Space::new().into());
    }
    let mut widget = shader::Shader::new(ShaderProgram {
        key: hash(&source),
        source,
        extra: pack_extra(&extra).into(),
        animated: node.get::<Option<bool>>("animated")?.unwrap_or(false),
        textures,
        fps: match node.get::<Option<u16>>("fps")? {
            Some(0) => return Err(mlua::Error::runtime("shader fps must be from 1 to 240")),
            Some(fps) if fps > 240 => {
                return Err(mlua::Error::runtime("shader fps must be from 1 to 240"));
            }
            Some(fps) => fps,
            None => 60,
        },
    });
    if let Some(width) = layout.width {
        widget = widget.width(width);
    }
    if let Some(height) = layout.height {
        widget = widget.height(height);
    }
    Ok(widget.into())
}

fn source(node: &Table, ctx: &Ctx) -> mlua::Result<Option<Arc<str>>> {
    let inline = node.get::<Option<String>>("source")?;
    let path = node.get::<Option<String>>("path")?;
    match (inline, path) {
        (Some(_), Some(_)) => Err(mlua::Error::runtime(
            "shader accepts source or path, not both",
        )),
        (None, None) => Err(mlua::Error::runtime("shader requires source or path")),
        (Some(source), None) => {
            if source.trim().is_empty() {
                return Err(mlua::Error::runtime("shader source must not be empty"));
            }
            Ok(Some(source.into()))
        }
        (None, Some(path)) => {
            if path.trim().is_empty() {
                return Err(mlua::Error::runtime("shader path must not be empty"));
            }
            match read(Path::new(&path), ctx.profile()) {
                Ok(source) => Ok(Some(source)),
                Err(error) => {
                    log::error!(target: "shader", "{error}");
                    Ok(None)
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct ShaderTexture {
    path: PathBuf,
    key: u64,
}

fn textures(node: &Table, ctx: &Ctx) -> mlua::Result<[Option<ShaderTexture>; 2]> {
    let Some(table) = node.get::<Option<Table>>("textures")? else {
        return Ok([None, None]);
    };
    let mut result = [None, None];
    let mut count = 0;
    for value in table.sequence_values::<String>() {
        let value = value?;
        if count >= result.len() {
            return Err(mlua::Error::runtime(
                "shader textures accepts at most two paths",
            ));
        }
        if value.trim().is_empty() {
            return Err(mlua::Error::runtime(
                "shader texture paths must not be empty",
            ));
        }
        let path = Path::new(&value);
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            ctx.profile().unwrap_or(Path::new(".")).join(path)
        };
        let metadata = fs::metadata(&path).map_err(|error| {
            mlua::Error::runtime(format!(
                "cannot read shader texture {}: {error}",
                path.display()
            ))
        })?;
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        metadata.len().hash(&mut hasher);
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
            .hash(&mut hasher);
        result[count] = Some(ShaderTexture {
            path,
            key: hasher.finish(),
        });
        count += 1;
    }
    if table.raw_len() != count {
        return Err(mlua::Error::runtime(
            "shader textures must be an array of file paths",
        ));
    }
    Ok(result)
}

fn extra_args(node: &Table) -> mlua::Result<Vec<(String, f32)>> {
    let Some(table) = node.get::<Option<Table>>("extraArgs")? else {
        return Ok(Vec::new());
    };
    let mut args = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let Value::String(key) = key else {
            return Err(mlua::Error::runtime("extraArgs keys must be strings"));
        };
        let key = key.to_str()?.to_owned();
        if !valid_extra_name(&key) {
            return Err(mlua::Error::runtime(format!(
                "invalid extraArgs key {key:?}; use a WGSL identifier that is not resolution, time, mouse, hover, _pad, or starts with riced_"
            )));
        }
        let value = match value {
            Value::Integer(value) => value as f64,
            Value::Number(value) => value,
            _ => {
                return Err(mlua::Error::runtime(format!(
                    "extraArgs.{key} must be a finite number"
                )));
            }
        };
        if !value.is_finite() || value < f32::MIN as f64 || value > f32::MAX as f64 {
            return Err(mlua::Error::runtime(format!(
                "extraArgs.{key} must be a finite number"
            )));
        }
        args.push((key, value as f32));
    }
    if args.len() > 16 {
        return Err(mlua::Error::runtime("extraArgs supports at most 16 values"));
    }
    args.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(args)
}

fn valid_extra_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
        && !matches!(name, "resolution" | "time" | "mouse" | "hover" | "_pad")
        && !name.starts_with("riced_")
}

fn full_header(extra: &[(String, f32)]) -> String {
    let padded = padded_fields(extra.len());
    let mut header = String::from(
        "struct Riced {\n    resolution: vec2<f32>,\n    time: f32,\n    hover: f32,\n    mouse: vec2<f32>,\n    _pad: vec2<f32>,\n",
    );
    for (name, _) in extra {
        let _ = writeln!(header, "    {name}: f32,");
    }
    for index in extra.len()..padded {
        let _ = writeln!(header, "    riced_pad{index}: f32,");
    }
    header.push_str("};\n\n@group(0) @binding(0) var<uniform> riced: Riced;\n");
    header.push_str(
        "@group(0) @binding(1) var riced_texture0: texture_2d<f32>;\n\
         @group(0) @binding(2) var riced_sampler0: sampler;\n\
         @group(0) @binding(3) var riced_texture1: texture_2d<f32>;\n\
         @group(0) @binding(4) var riced_sampler1: sampler;\n",
    );
    header.push_str(HEADER);
    header
}

fn padded_fields(len: usize) -> usize {
    len.max(1).div_ceil(4) * 4
}

fn pack_extra(extra: &[(String, f32)]) -> Vec<f32> {
    let mut values = extra.iter().map(|(_, value)| *value).collect::<Vec<_>>();
    values.resize(padded_fields(extra.len()), 0.0);
    values
}

fn read(path: &Path, profile: Option<&Path>) -> Result<Arc<str>, String> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        profile.unwrap_or(Path::new(".")).join(path)
    };
    let metadata = fs::metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    if metadata.len() > MAX_SOURCE {
        return Err(format!(
            "{}: shader exceeds the 1 MiB limit",
            path.display()
        ));
    }
    let modified = metadata.modified().ok();
    let len = metadata.len();
    FILES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(cached) = cache.get(&path)
            && cached.modified == modified
            && cached.len == len
        {
            return Ok(cached.source.clone());
        }
        let source: Arc<str> = fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?
            .into();
        if cache.len() >= FILE_CACHE {
            cache.clear();
        }
        cache.insert(
            path.clone(),
            CachedFile {
                modified,
                len,
                source: source.clone(),
            },
        );
        Ok(source)
    })
}

fn hash(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

fn validate(source: &str) -> Result<(), String> {
    let key = hash(source);
    VALIDATED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(result) = cache.get(&key) {
            return result.clone();
        }
        let result = (|| {
            let module = naga::front::wgsl::parse_str(source).map_err(|error| error.to_string())?;
            let mut validator = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::default(),
            );
            validator
                .validate(&module)
                .map_err(|error| error.to_string())?;
            fragment_entry(&module)
        })();
        if cache.len() >= VALIDATION_CACHE {
            cache.clear();
        }
        cache.insert(key, result.clone());
        result
    })
}

fn fragment_entry(module: &naga::Module) -> Result<(), String> {
    for variable in module.global_variables.iter().map(|(_, variable)| variable) {
        if let Some(binding) = &variable.binding
            && (binding.group != 0 || !(0..=4).contains(&binding.binding))
        {
            return Err("shader resources must use group 0 bindings 0 to 4".to_owned());
        }
    }
    let entry = module
        .entry_points
        .iter()
        .find(|entry| entry.name == "fs_main" && entry.stage == naga::ShaderStage::Fragment)
        .ok_or("shader must define @fragment fn fs_main")?;
    let argument = entry
        .function
        .arguments
        .first()
        .filter(|_| entry.function.arguments.len() == 1)
        .ok_or("fs_main must take one @location(0) uv: vec2<f32> argument")?;
    let Some(naga::Binding::Location {
        location: 0,
        interpolation,
        sampling,
        ..
    }) = argument.binding.clone()
    else {
        return Err("fs_main must take one @location(0) uv: vec2<f32> argument".to_owned());
    };
    if !matches!(interpolation, None | Some(naga::Interpolation::Perspective))
        || !matches!(sampling, None | Some(naga::Sampling::Center))
        || !is_vector(module, argument.ty, naga::VectorSize::Bi)
    {
        return Err("fs_main must take one @location(0) uv: vec2<f32> argument".to_owned());
    }
    let result = entry
        .function
        .result
        .as_ref()
        .ok_or("fs_main must return @location(0) vec4<f32>")?;
    if !matches!(
        result.binding,
        Some(naga::Binding::Location { location: 0, .. })
    ) || !is_vector(module, result.ty, naga::VectorSize::Quad)
    {
        return Err("fs_main must return @location(0) vec4<f32>".to_owned());
    }
    Ok(())
}

fn is_vector(module: &naga::Module, ty: naga::Handle<naga::Type>, size: naga::VectorSize) -> bool {
    matches!(
        module.types[ty].inner,
        naga::TypeInner::Vector {
            size: actual,
            scalar: naga::Scalar {
                kind: naga::ScalarKind::Float,
                width: 4,
            },
        } if actual == size
    )
}

#[derive(Default)]
struct ShaderState {
    gpu: Arc<Mutex<Option<(u64, GpuState)>>>,
}

struct ShaderProgram {
    source: Arc<str>,
    key: u64,
    extra: Arc<[f32]>,
    textures: [Option<ShaderTexture>; 2],
    animated: bool,
    fps: u16,
}

impl Program<Message> for ShaderProgram {
    type State = ShaderState;
    type Primitive = ShaderPrimitive;

    fn update(
        &self,
        _state: &mut Self::State,
        _event: &iced::Event,
        _bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        self.animated.then(|| {
            let step = std::time::Duration::from_secs_f32(1.0 / f32::from(self.fps));
            shader::Action::request_redraw_at(std::time::Instant::now() + step)
        })
    }

    fn draw(
        &self,
        state: &Self::State,
        cursor: iced::mouse::Cursor,
        bounds: Rectangle,
    ) -> Self::Primitive {
        let (mouse, hover) = match cursor.position_in(bounds) {
            Some(point) => (point, 1.0),
            None => (iced::Point::new(-1.0, -1.0), 0.0),
        };
        ShaderPrimitive {
            source: self.source.clone(),
            key: self.key,
            extra: self.extra.clone(),
            textures: self.textures.clone(),
            mouse,
            hover,
            size: bounds.size(),
            gpu: state.gpu.clone(),
        }
    }
}

#[derive(Debug)]
struct ShaderPrimitive {
    source: Arc<str>,
    key: u64,
    extra: Arc<[f32]>,
    textures: [Option<ShaderTexture>; 2],
    mouse: iced::Point,
    hover: f32,
    size: iced::Size,
    gpu: Arc<Mutex<Option<(u64, GpuState)>>>,
}

#[derive(Debug)]
struct GpuState {
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    _textures: Vec<wgpu::Texture>,
}

impl Primitive for ShaderPrimitive {
    type Pipeline = ShaderPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _bounds: &Rectangle,
        _viewport: &shader::Viewport,
    ) {
        let time = pipeline.start.elapsed().as_secs_f32();
        pipeline.render(device, &self.source, self.key);

        let size = (32 + self.extra.len() * 4) as u64;
        let key = gpu_key(size, &self.textures);
        let mut gpu = self.gpu.lock().unwrap();
        if gpu.as_ref().is_none_or(|(existing, _)| *existing != key) {
            let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("riced shader uniforms"),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut textures = Vec::new();
            let mut views = [pipeline.dummy_view.clone(), pipeline.dummy_view.clone()];
            for (index, texture) in self.textures.iter().enumerate() {
                let Some(texture) = texture else {
                    continue;
                };
                match upload_texture(device, queue, &texture.path) {
                    Some((texture, view)) => {
                        views[index] = view;
                        textures.push(texture);
                    }
                    None => log::warn!(
                        target: "shader",
                        "cannot decode shader texture {}",
                        texture.path.display()
                    ),
                }
            }
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("riced shader uniforms"),
                layout: &pipeline.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&views[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&views[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                    },
                ],
            });
            *gpu = Some((
                key,
                GpuState {
                    uniforms,
                    bind_group,
                    _textures: textures,
                },
            ));
        }
        if let Some((_, gpu)) = gpu.as_ref() {
            let mut bytes = Vec::with_capacity(size as usize);
            for value in [
                self.size.width,
                self.size.height,
                time,
                self.hover,
                self.mouse.x,
                self.mouse.y,
                0.0,
                0.0,
            ]
            .into_iter()
            .chain(self.extra.iter().copied())
            {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            queue.write_buffer(&gpu.uniforms, 0, &bytes);
        }
    }

    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        let gpu = self.gpu.lock().unwrap();
        let Some((_, gpu)) = gpu.as_ref() else {
            return false;
        };
        let Some(render_pipeline) = pipeline.pipelines.get(&self.key) else {
            return false;
        };
        render_pass.set_pipeline(render_pipeline);
        render_pass.set_bind_group(0, &gpu.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        true
    }
}

fn gpu_key(size: u64, textures: &[Option<ShaderTexture>; 2]) -> u64 {
    let mut hasher = DefaultHasher::new();
    size.hash(&mut hasher);
    for texture in textures {
        texture
            .as_ref()
            .map(|texture| texture.key)
            .hash(&mut hasher);
    }
    hasher.finish()
}

fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
) -> Option<(wgpu::Texture, wgpu::TextureView)> {
    let image = image::open(path).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("riced shader texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        image.as_raw(),
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Some((texture, view))
}

struct ShaderPipeline {
    start: Instant,
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    _dummy: wgpu::Texture,
    dummy_view: wgpu::TextureView,
    pipelines: HashMap<u64, wgpu::RenderPipeline>,
}

impl Pipeline for ShaderPipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("riced shader bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("riced shader sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let dummy = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("riced shader dummy"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[255, 255, 255, 255],
        );
        let dummy_view = dummy.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            start: Instant::now(),
            format,
            layout,
            sampler,
            _dummy: dummy,
            dummy_view,
            pipelines: HashMap::new(),
        }
    }
}

impl ShaderPipeline {
    fn render(&mut self, device: &wgpu::Device, source: &str, key: u64) -> &wgpu::RenderPipeline {
        if !self.pipelines.contains_key(&key) {
            if self.pipelines.len() >= PIPELINE_CACHE {
                self.pipelines.clear();
            }
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("riced shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("riced shader"),
                bind_group_layouts: &[&self.layout],
                push_constant_ranges: &[],
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("riced shader"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("riced_vertex"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            });
            self.pipelines.insert(key, pipeline);
        }
        self.pipelines.get(&key).expect("shader pipeline inserted")
    }
}
