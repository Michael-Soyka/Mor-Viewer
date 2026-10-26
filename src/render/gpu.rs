use std::collections::{ HashMap, HashSet };

use eframe::egui::{ Color32, TextureId };
use eframe::egui_wgpu::{ RenderState, wgpu };
use wgpu::util::DeviceExt;

use crate::render::culling::{ CullView, Culling };
use crate::render::model::{ Model, Vertex, to_u32, to_usize };
use crate::render::paths::{ GpuPaths, PathLines, PathView };
use crate::render::pipeline::{ pipeline, uniform_entry };
use crate::render::sky::{ GpuSky, SKY_VERTICES };
use crate::render::target::{ Target, begin_pass };
use crate::render::texture::{ DecodedTexture, Textures, upload, upload_texture };
use crate::render::water::{ Water, WaterLevel, WaterPass, reflected };
use crate::render::weather::SkyImages;

/// Vertices per buffer: 40-byte vertices stay well under wgpu's 256 MiB default buffer limit.
const CHUNK_VERTICES: usize = 4_000_000;

/// Byte offset of the light color in a part's uniform, after the base color.
const LIGHT_OFFSET: wgpu::BufferAddress = 16;

/// Lightmap overlays add up on top of the plain pass.
const ADDITIVE: wgpu::BlendState = wgpu::BlendState
{
    color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
    alpha: wgpu::BlendComponent::REPLACE,
};

/// Lightmaps are 8-bit luminance.
const LIGHTMAP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Frame uniforms: view-projection, light, params, eye, ambient, fog, clip plane, water, background, sun, sky layers;
/// `Frame` in `model.wgsl`.
pub const FRAME_FLOATS: usize = 60;
pub type FrameUniforms = [f32; FRAME_FLOATS];

/// Pipeline a frame is drawn with.
#[ derive( Clone, Copy ) ]
pub enum Pass
{
    Wireframe,
    TwoSided,
    FrontFaces,
}

struct GpuPart
{
    /// Index into [`Gpu::vertices`]; the part's indices count from that buffer's start.
    chunk: usize,

    /// Light a lightmap overlay adds; `None` for the plain pass.
    light: Option<u32>,

    /// Base color, then the light's color.
    uniform: wgpu::Buffer,

    /// The part's records in [`Culling`], a cluster each, and its place among the parts.
    records: std::ops::Range<u32>,
    index:   u32,

    /// Drawn over the reflection of its level, see [`Water`].
    water: bool,

    bind_group: wgpu::BindGroup,
}

/// What every camera of a frame draws alike: the pipeline, the lightmap overlays, whether two-phase, the background.
#[ derive( Clone, Copy ) ]
struct Look<'a>
{
    pass:      Pass,
    lights:    Option<&'a HashSet<u32>>,
    two_phase: bool,
    clear:     wgpu::Color,

    /// The sky goes behind the frame, as it does in the textured modes.
    sky: bool,
}

/// One camera's pass over the frame.
#[ derive( Clone, Copy ) ]
struct Eye<'a>
{
    frame:   &'a wgpu::BindGroup,
    culling: &'a CullView,

    /// Front faces only.
    culled: &'a wgpu::RenderPipeline,

    water: WaterPass<'a>,
}

pub struct Gpu
{
    pub state:       RenderState,
    pub culling:     Culling,
    pub water:       Option<Water>,
    pub lightmapped: bool,
    pub paths:       Option<GpuPaths>,
    pub target:      Option<Target>,
    pub texture_id:  Option<TextureId>,

    /// Culling of the camera; the reflection has its own, see [`Water`].
    pub camera: CullView,

    fill:        wgpu::RenderPipeline,
    fill_culled: wgpu::RenderPipeline,
    wire:        wgpu::RenderPipeline,
    overlay:     wgpu::RenderPipeline,
    vertices:    Vec<wgpu::Buffer>,
    triangles:   wgpu::Buffer,
    edges:       wgpu::Buffer,
    frame:       wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    parts:       Vec<GpuPart>,

    /// The game's sky behind the frame, see [`Gpu::set_sky`].
    sky: Option<GpuSky>,
}

/// Vertices split into buffers under the device's size limit, a run of whole parts each.
struct VertexChunks
{
    ranges:      Vec<std::ops::Range<usize>>,
    part_chunks: Vec<usize>,

    /// The model's indices, each part's counted from its chunk's start.
    triangles: Vec<u32>,
    edges:     Vec<u32>,
}

impl VertexChunks
{
    fn new( model: &Model ) -> Self
    {
        let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
        let mut part_chunks = Vec::with_capacity( model.parts.len() );

        for part in &model.parts
        {
            let triangles = model.triangles.get( to_usize( part.triangles.start )..to_usize( part.triangles.end ) ).unwrap_or_default();
            let edges     = model.edges.get( to_usize( part.edges.start )..to_usize( part.edges.end ) ).unwrap_or_default();
            let indices   = triangles.iter().chain( edges ).map( |&index| to_usize( index ) );
            let span      = indices.fold( None, |span: Option<( usize, usize )>, index| Some( span.map_or( ( index, index + 1 ), |( low, high )| ( low.min( index ), high.max( index + 1 ) ) ) ) );
            let ( low, high ) = span.unwrap_or( ( 0, 0 ) );

            match ranges.last_mut()
            {
                Some( range ) if high.max( range.end ) - low.min( range.start ) <= CHUNK_VERTICES =>
                {
                    range.start = range.start.min( low );
                    range.end = range.end.max( high );
                }
                _ => ranges.push( low..high ),
            }
            part_chunks.push( ranges.len() - 1 );
        }
        if ranges.is_empty()
        {
            ranges.push( 0..model.vertices.len() );
        }

        let mut triangles = model.triangles.clone();
        let mut edges     = model.edges.clone();

        for ( part, &chunk ) in model.parts.iter().zip( &part_chunks )
        {
            let start = to_u32( ranges[ chunk ].start );
            let lists = [ ( &mut triangles, &part.triangles ), ( &mut edges, &part.edges ) ];
            for ( list, range ) in lists
            {
                for index in list.get_mut( to_usize( range.start )..to_usize( range.end ) ).unwrap_or_default()
                {
                    *index -= start;
                }
            }
        }

        let chunks = Self { ranges, part_chunks, triangles, edges };

        chunks
    }
}

impl Gpu
{
    pub fn new( state: &RenderState, model: &Model, textures: &Textures ) -> Self
    {
        let device = &state.device;
        let shader_source = wgpu::ShaderModuleDescriptor
        {
            label:  Some( "model" ),
            source: wgpu::ShaderSource::Wgsl( include_str!( "shaders/model.wgsl" ).into() ),
        };
        let shader = device.create_shader_module( shader_source );

        let frame_entries = [ uniform_entry( 0, wgpu::ShaderStages::VERTEX_FRAGMENT ) ];
        let frame_layout  = device.create_bind_group_layout( &wgpu::BindGroupLayoutDescriptor { label: Some( "model frame" ), entries: &frame_entries } );

        let part_entries =
        [
            wgpu::BindGroupLayoutEntry
            {
                binding:    0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty:         wgpu::BindingType::Texture
                {
                    sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled:   false,
                },
                count:      None,
            },
            wgpu::BindGroupLayoutEntry
            {
                binding:    1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty:         wgpu::BindingType::Sampler( wgpu::SamplerBindingType::Filtering ),
                count:      None,
            },
            uniform_entry( 2, wgpu::ShaderStages::FRAGMENT ),
            wgpu::BindGroupLayoutEntry
            {
                binding:    3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty:         wgpu::BindingType::Texture
                {
                    sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled:   false,
                },
                count:      None,
            },
        ];
        let part_layout = device.create_bind_group_layout( &wgpu::BindGroupLayoutDescriptor { label: Some( "model part" ), entries: &part_entries } );
        let layout_descriptor = wgpu::PipelineLayoutDescriptor
        {
            label:              Some( "model" ),
            bind_group_layouts: &[ Some( &frame_layout ), Some( &part_layout ) ],
            immediate_size:     0,
        };
        let layout = device.create_pipeline_layout( &layout_descriptor );
        let fill        = pipeline( device, &layout, &shader, "fs_main", wgpu::PrimitiveTopology::TriangleList, None, None );
        let fill_culled = pipeline( device, &layout, &shader, "fs_main", wgpu::PrimitiveTopology::TriangleList, Some( wgpu::Face::Back ), None );
        let wire        = pipeline( device, &layout, &shader, "fs_wire", wgpu::PrimitiveTopology::LineList, None, None );
        let overlay     = pipeline( device, &layout, &shader, "fs_lightmap", wgpu::PrimitiveTopology::TriangleList, None, Some( ADDITIVE ) );

        let buffer = |label: &str, contents: &[u8], usage: wgpu::BufferUsages|
        {
            device.create_buffer_init( &wgpu::util::BufferInitDescriptor { label: Some( label ), contents, usage } )
        };
        let split         = VertexChunks::new( model );
        let vertices      = split.ranges.iter().map( |range| buffer( "model vertices", bytemuck::cast_slice( &model.vertices[ range.clone() ] ), wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST ) ).collect();
        let triangles     = buffer( "model triangles", bytemuck::cast_slice( &non_empty( &split.triangles ) ), wgpu::BufferUsages::INDEX );
        let edges         = buffer( "model edges", bytemuck::cast_slice( &non_empty( &split.edges ) ), wgpu::BufferUsages::INDEX );
        let frame         = buffer( "model frame", bytemuck::cast_slice( &[ 0.0f32; FRAME_FLOATS ] ), wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST );
        let frame_binding = [ wgpu::BindGroupEntry { binding: 0, resource: frame.as_entire_binding() } ];
        let frame_group   = device.create_bind_group( &wgpu::BindGroupDescriptor { label: Some( "model frame" ), layout: &frame_layout, entries: &frame_binding } );

        let sampler_descriptor = wgpu::SamplerDescriptor
        {
            label:          Some( "model albedo" ),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter:     wgpu::FilterMode::Linear,
            min_filter:     wgpu::FilterMode::Linear,
            mipmap_filter:  wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        };
        let sampler = device.create_sampler( &sampler_descriptor );
        let white   = upload_texture( state, &DecodedTexture { width: 1, height: 1, mips: vec![ vec![ 255; 4 ] ] } );
        let neutral = upload( state, 1, 1, &[ vec![ 0 ] ], LIGHTMAP_FORMAT );
        let baked: Vec<wgpu::TextureView> = model.lightmaps.iter().map( |image| upload( state, image.width, image.height, &image.mips, LIGHTMAP_FORMAT ) ).collect();

        let views: HashMap<&str, wgpu::TextureView> = textures.iter().map( |( name, texture )| ( name.as_str(), upload_texture( state, texture ) ) ).collect();

        let ( culling, records ) = Culling::new( device, model );
        let mut parts = Vec::new();

        for ( ( index, ( part, &chunk ) ), records ) in ( 0.. ).zip( model.parts.iter().zip( &split.part_chunks ) ).zip( records )
        {
            let key  = part.texture.as_deref().map( str::to_lowercase );
            let view = key.and_then( |key| views.get( key.as_str() ) ).unwrap_or( &white );
            let lightmap = part.lightmap.and_then( |( index, _ )| baked.get( index ) ).unwrap_or( &neutral );
            let mirrors  = if part.mirrors { 1.0 } else { 0.0 };
            let values   = [ part.color, [ 1.0; 4 ], [ part.water.unwrap_or_default(), mirrors, 0.0, 0.0 ] ];
            let color    = buffer( "model part", bytemuck::cast_slice( &values ), wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST );
            let entries =
            [
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView( view ) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler( &sampler ) },
                wgpu::BindGroupEntry { binding: 2, resource: color.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView( lightmap ) },
            ];
            let bind_group = device.create_bind_group( &wgpu::BindGroupDescriptor { label: Some( "model part" ), layout: &part_layout, entries: &entries } );
            let light = part.lightmap.map( |( _, light )| light );
            parts.push( GpuPart { chunk, light, uniform: color, records, index, water: part.water.is_some(), bind_group } );
        }

        let mut levels = Vec::<WaterLevel>::new();

        for part in &model.parts
        {
            let Some( height ) = part.water.filter( |_| part.mirrors ) else { continue };
            let boxes = part.draws.iter().filter_map( |draw| model.clusters.get( to_usize( draw.cluster ) ) ).map( |cluster| ( cluster.min, cluster.max ) );
            match levels.iter_mut().find( |level| level.height == height )
            {
                Some( level ) => level.boxes.extend( boxes ),
                None => levels.push( WaterLevel { height, boxes: boxes.collect() } ),
            }
        }

        // The reflection is read as the albedo is: a texture and its sampler.
        let reflection_entries = &part_entries[ ..2 ];
        let has_water          = model.parts.iter().any( |part| part.water.is_some() );
        let water              = has_water.then( || Water::new( device, &shader, [ &frame_layout, &part_layout ], reflection_entries, &culling, levels ) );
        let camera             = culling.view( device );

        let gpu = Self
        {
            state: state.clone(),
            fill,
            fill_culled,
            wire,
            overlay,
            vertices,
            triangles,
            edges,
            frame,
            frame_group,
            parts,
            culling,
            camera,
            water,
            lightmapped: !model.lightmaps.is_empty(),
            paths:      None,
            target:     None,
            texture_id: None,
            sky:        None,
        };

        gpu
    }

    /// New vertices for a model of one chunk, as a character is.
    pub fn set_vertices( &self, vertices: &[Vertex] )
    {
        let Some( buffer ) = self.vertices.first() else { return };
        self.state.queue.write_buffer( buffer, 0, bytemuck::cast_slice( vertices ) );
    }

    pub fn set_light_colors( &self, colors: &HashMap<u32, Color32> )
    {
        for part in &self.parts
        {
            let Some( color ) = part.light.and_then( |light| colors.get( &light ) ) else { continue };
            let [ red, green, blue, _ ] = color.to_normalized_gamma_f32();
            let light = [ red, green, blue, 1.0 ];
            self.state.queue.write_buffer( &part.uniform, LIGHT_OFFSET, bytemuck::cast_slice( &light ) );
        }
    }

    /// Lines of their own over the frame uniforms only, depth-tested against the model.
    pub fn set_paths( &mut self, lines: &PathLines )
    {
        self.paths = Some( GpuPaths::new( &self.state.device, lines ) );
    }

    /// The game's sky from `images`: a cube around the eye that the frame draws over.
    pub fn set_sky( &mut self, images: &SkyImages )
    {
        self.sky = Some( GpuSky::new( &self.state, images ) );
    }

    pub fn resize( &mut self, size: [u32; 2] )
    {
        let device = &self.state.device;
        let target = Target::new( device, size );
        self.culling.resize( device, &mut self.camera, &target.depth, size );

        if let Some( water ) = &mut self.water
        {
            water.resize( device, &self.culling, size );
        }

        let mut renderer = self.state.renderer.write();

        match self.texture_id
        {
            Some( id ) => renderer.update_egui_texture_from_wgpu_texture( device, &target.resolve, wgpu::FilterMode::Linear, id ),
            None => self.texture_id = Some( renderer.register_native_texture( device, &target.resolve, wgpu::FilterMode::Linear ) ),
        }
        self.target = Some( target );
    }

    /// `lights` turns the lightmap overlays on, all but the switched off ones.
    /// `occlusion` draws last frame's visible clusters first, then only what their depth leaves visible.
    /// `mirror`, for the textured modes, draws the see-through water, over the reflection of its level if `reflections`.
    pub fn draw( &self, uniforms: &FrameUniforms, pass: Pass, paths: PathView, lights: Option<&HashSet<u32>>, occlusion: bool, background: Color32, mirror: bool, reflections: bool )
    {
        let Some( target ) = &self.target else { return };
        let queue     = &self.state.queue;
        let wireframe = matches!( pass, Pass::Wireframe );
        let two_phase = occlusion && !wireframe;
        let mut encoder = self.state.device.create_command_encoder( &wgpu::CommandEncoderDescriptor { label: Some( "model" ) } );

        let [ r, g, b, a ] = background.to_normalized_gamma_f32();
        let clear = wgpu::Color { r: f64::from( r ), g: f64::from( g ), b: f64::from( b ), a: f64::from( a ) };

        let mut uniforms = *uniforms;
        uniforms[ 48..51 ].copy_from_slice( &[ r, g, b ] );

        let water  = self.water.as_ref().filter( |_| mirror && !wireframe ).and_then( |water| Some( ( water, water.reflection.as_ref()? ) ) );
        let sky    = mirror && !wireframe && uniforms[ 59 ] > 0.5;
        let look   = Look { pass, lights: lights.filter( |_| !wireframe ), two_phase, clear, sky };

        if let Some( ( water, reflection ) ) = water
            && reflections
            && let Some( level ) = water.level( [ uniforms[ 24 ], uniforms[ 25 ], uniforms[ 26 ] ] )
        {
            uniforms[ 44 ] = level;
            uniforms[ 45 ] = 1.0;
            let mirrored = reflected( &uniforms, level );
            queue.write_buffer( &water.frame, 0, bytemuck::cast_slice( &mirrored ) );
            water.culling.prepare( queue, &mirrored[ ..16 ], &mirrored[ 40..44 ], two_phase );
            let eye = Eye { frame: &water.frame_group, culling: &water.culling, culled: &water.mirrored, water: WaterPass::Hidden };
            self.draw_eye( &mut encoder, &reflection.target, eye, look, None );
        }

        queue.write_buffer( &self.frame, 0, bytemuck::cast_slice( &uniforms ) );
        self.camera.prepare( queue, &uniforms[ ..16 ], &uniforms[ 40..44 ], two_phase );
        let surface = water.map_or( WaterPass::Plain, |( water, reflection )| WaterPass::Mirror( &water.surface, &reflection.group ) );
        let eye     = Eye { frame: &self.frame_group, culling: &self.camera, culled: &self.fill_culled, water: surface };
        self.draw_eye( &mut encoder, target, eye, look, Some( paths ) );

        let read = self.camera.read_back( &mut encoder );

        queue.submit( [ encoder.finish() ] );
        if read
        {
            self.camera.map();
        }
    }

    /// One camera's frame into `target`: its culling, the parts and the overlays, and the `paths` lines if any.
    fn draw_eye( &self, encoder: &mut wgpu::CommandEncoder, target: &Target, eye: Eye, look: Look, paths: Option<PathView> )
    {
        if look.two_phase
        {
            self.culling.early( eye.culling, encoder );
            let mut render_pass = begin_pass( encoder, target, Some( look.clear ), false );
            self.draw_sky( &mut render_pass, eye, look );
            let mut bound = self.draw_plain( &mut render_pass, eye, look.pass );
            self.draw_water( &mut render_pass, eye, &mut bound );
        }
        self.culling.late( eye.culling, encoder, look.two_phase );

        let mut render_pass = begin_pass( encoder, target, ( !look.two_phase ).then_some( look.clear ), true );
        if !look.two_phase
        {
            self.draw_sky( &mut render_pass, eye, look );
        }
        let mut bound = self.draw_plain( &mut render_pass, eye, look.pass );

        if let Some( switched_off ) = look.lights
        {
            render_pass.set_pipeline( &self.overlay );
            for part in &self.parts
            {
                let Some( light ) = part.light else { continue };
                if !switched_off.contains( &light )
                {
                    self.draw_part( &mut render_pass, eye, part, false, &mut bound );
                }
            }
        }
        self.draw_water( &mut render_pass, eye, &mut bound );

        let Some( ( gpu_paths, paths ) ) = self.paths.as_ref().zip( paths ) else { return };
        render_pass.set_pipeline( &gpu_paths.pipeline );
        render_pass.set_vertex_buffer( 0, gpu_paths.vertices.slice( .. ) );
        render_pass.set_index_buffer( gpu_paths.indices.slice( .. ), wgpu::IndexFormat::Uint32 );
        for ( range, shown ) in [ ( gpu_paths.cells.clone(), paths.cells ), ( gpu_paths.links.clone(), paths.links ) ]
        {
            if shown && !range.is_empty()
            {
                render_pass.draw_indexed( range, 0, 0..1 );
            }
        }
    }

    /// Parts without lightmaps, as far as the culling left them, and the water if `eye` draws it as them; returns the bound vertex chunk.
    fn draw_plain( &self, render_pass: &mut wgpu::RenderPass, eye: Eye, pass: Pass ) -> Option<usize>
    {
        let ( pipeline, indices ) = match pass
        {
            Pass::Wireframe => ( &self.wire, &self.edges ),
            Pass::TwoSided => ( &self.fill, &self.triangles ),
            Pass::FrontFaces => ( eye.culled, &self.triangles ),
        };
        render_pass.set_bind_group( 0, eye.frame, &[] );
        render_pass.set_pipeline( pipeline );
        render_pass.set_index_buffer( indices.slice( .. ), wgpu::IndexFormat::Uint32 );

        let edges     = matches!( pass, Pass::Wireframe );
        let plain     = matches!( eye.water, WaterPass::Plain );
        let mut bound = None;

        for part in self.parts.iter().filter( |part| part.light.is_none() && ( plain || !part.water ) )
        {
            self.draw_part( render_pass, eye, part, edges, &mut bound );
        }

        bound
    }

    /// The sky behind everything else, in a pass that starts anew.
    fn draw_sky( &self, render_pass: &mut wgpu::RenderPass, eye: Eye, look: Look )
    {
        let Some( sky ) = self.sky.as_ref().filter( |_| look.sky ) else { return };
        render_pass.set_pipeline( &sky.pipeline );
        render_pass.set_bind_group( 0, eye.frame, &[] );
        render_pass.set_bind_group( 1, &sky.group, &[] );
        render_pass.draw( 0..SKY_VERTICES, 0..1 );
    }

    /// The see-through water over what is drawn so far, when `eye` mirrors it.
    fn draw_water( &self, render_pass: &mut wgpu::RenderPass, eye: Eye, bound: &mut Option<usize> )
    {
        let WaterPass::Mirror( surface, reflection ) = eye.water else { return };
        render_pass.set_pipeline( surface );
        render_pass.set_bind_group( 2, reflection, &[] );
        render_pass.set_index_buffer( self.triangles.slice( .. ), wgpu::IndexFormat::Uint32 );

        for part in self.parts.iter().filter( |part| part.water )
        {
            self.draw_part( render_pass, eye, part, false, bound );
        }
    }

    /// A part's draws, one per cluster; the culled ones have no instances.
    fn draw_part( &self, render_pass: &mut wgpu::RenderPass, eye: Eye, part: &GpuPart, edges: bool, bound: &mut Option<usize> )
    {
        let Some( vertices ) = self.vertices.get( part.chunk ) else { return };
        if part.records.is_empty()
        {
            return;
        }
        if *bound != Some( part.chunk )
        {
            render_pass.set_vertex_buffer( 0, vertices.slice( .. ) );
            *bound = Some( part.chunk );
        }
        render_pass.set_bind_group( 1, &part.bind_group, &[] );
        self.culling.draw( eye.culling, render_pass, part.index, &part.records, edges );
    }
}

impl Drop for Gpu
{
    fn drop( &mut self )
    {
        if let Some( id ) = self.texture_id
        {
            self.state.renderer.write().free_texture( &id );
        }
    }
}

/// wgpu rejects zero-sized buffers.
pub fn non_empty( indices: &[u32] ) -> Vec<u32>
{
    if indices.is_empty() { vec![ 0 ] } else { indices.to_vec() }
}
