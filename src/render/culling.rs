use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{ AtomicU8, Ordering };

use eframe::egui_wgpu::wgpu;
use wgpu::util::DeviceExt;

use crate::render::model::{ Draw, Model, to_u32, to_usize };

const WORKGROUP: u32 = 64;
const TILE:      u32 = 8;

/// Bytes of one `DrawIndexedIndirectArgs`.
const ARGS_SIZE: wgpu::BufferAddress = 20;

/// Bytes of a part's draw count.
const COUNT_SIZE: wgpu::BufferAddress = 4;

/// Bytes of the counters: visible clusters, drawn triangles.
const COUNTERS_SIZE: wgpu::BufferAddress = 8;

/// Bytes of `Cull` in `culling.wgsl`.
const UNIFORM_SIZE: wgpu::BufferAddress = 96;

/// Cluster state and uniform flags, as in `culling.wgsl`.
const VISIBLE:   u32 = 1;
const OCCLUSION: u32 = 1;
const TWO_PHASE: u32 = 2;

/// Readback of the counters.
const IDLE:    u8 = 0;
const PENDING: u8 = 1;
const READY:   u8 = 2;

const PYRAMID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// What the last culled frame drew.
#[ derive( Clone, Copy ) ]
pub struct Visible
{
    pub clusters:  u32,
    pub triangles: u32,
}

/// Depth pyramid of the frame size, with the bind groups that read it.
struct Pyramid
{
    sizes: Vec<[u32; 2]>,
    base:  wgpu::BindGroup,
    down:  Vec<wgpu::BindGroup>,
    cull:  wgpu::BindGroup,
}

/// Cluster culling on the GPU: the frustum and, two-phase, the depth of what is already drawn.
/// Writes the indirect draws of every part's clusters into a [`CullView`], one per camera.
pub struct Culling
{
    early_clusters: wgpu::ComputePipeline,
    early_draws:    wgpu::ComputePipeline,
    cull_clusters:  wgpu::ComputePipeline,
    late_draws:     wgpu::ComputePipeline,
    base:           wgpu::ComputePipeline,
    down:           wgpu::ComputePipeline,
    cull_layout:    wgpu::BindGroupLayout,
    base_layout:    wgpu::BindGroupLayout,
    down_layout:    wgpu::BindGroupLayout,
    clusters:       wgpu::Buffer,
    records:        wgpu::Buffer,
    cluster_count:  u32,
    record_count:   u32,
    part_count:     u32,

    /// The device draws only the packed draws of a part; without it the cleared rest are drawn empty.
    counted: bool,
}

/// What one camera culls with: its frustum, last frame's visible clusters, its depth pyramid and the draws packed for it.
pub struct CullView
{
    uniform:  wgpu::Buffer,
    states:   wgpu::Buffer,
    indirect: wgpu::Buffer,
    counts:   wgpu::Buffer,
    counters: wgpu::Buffer,
    readback: wgpu::Buffer,
    pyramid:  Option<Pyramid>,
    readout:  Arc<AtomicU8>,
    visible:  Option<Visible>,
}

impl Culling
{
    /// Also the range of records of each part, in the model's order.
    pub fn new( device: &wgpu::Device, model: &Model ) -> ( Self, Vec<Range<u32>> )
    {
        let mut records = Vec::new();
        let mut ranges  = Vec::with_capacity( model.parts.len() );

        for ( index, part ) in ( 0.. ).zip( &model.parts )
        {
            let start   = to_u32( records.len() );
            let overlay = u32::from( part.lightmap.is_some() );
            let record  = |draw: &Draw| [ draw.cluster, overlay, draw.triangles.start, draw.triangles.end - draw.triangles.start, draw.edges.start, draw.edges.end - draw.edges.start, index, start ];
            records.extend( part.draws.iter().map( record ) );
            ranges.push( start..to_u32( records.len() ) );
        }

        let mut boxes: Vec<[f32; 8]> = model.clusters.iter().map( |cluster| [ cluster.min[ 0 ], cluster.min[ 1 ], cluster.min[ 2 ], 0.0, cluster.max[ 0 ], cluster.max[ 1 ], cluster.max[ 2 ], 0.0 ] ).collect();

        // wgpu rejects empty bindings.
        if records.is_empty()
        {
            records.push( [ 0; 8 ] );
        }
        if boxes.is_empty()
        {
            boxes.push( [ 0.0; 8 ] );
        }

        let cluster_count = to_u32( boxes.len() );
        let record_count  = to_u32( records.len() );

        let buffer = |label: &str, contents: &[u8], usage: wgpu::BufferUsages|
        {
            device.create_buffer_init( &wgpu::util::BufferInitDescriptor { label: Some( label ), contents, usage } )
        };
        let storage  = wgpu::BufferUsages::STORAGE;
        let clusters = buffer( "cull clusters", bytemuck::cast_slice( &boxes ), storage );
        let records  = buffer( "cull records", bytemuck::cast_slice( &records ), storage );

        let unfilterable = wgpu::BindingType::Texture
        {
            sample_type:    wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled:   false,
        };
        let level = wgpu::BindingType::StorageTexture
        {
            access:         wgpu::StorageTextureAccess::WriteOnly,
            format:         PYRAMID_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let depth = wgpu::BindingType::Texture
        {
            sample_type:    wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled:   true,
        };
        let cull_entries =
        [
            entry( 0, buffer_type( wgpu::BufferBindingType::Uniform ) ),
            entry( 1, buffer_type( wgpu::BufferBindingType::Storage { read_only: true } ) ),
            entry( 2, buffer_type( wgpu::BufferBindingType::Storage { read_only: false } ) ),
            entry( 3, buffer_type( wgpu::BufferBindingType::Storage { read_only: true } ) ),
            entry( 4, buffer_type( wgpu::BufferBindingType::Storage { read_only: false } ) ),
            entry( 5, buffer_type( wgpu::BufferBindingType::Storage { read_only: false } ) ),
            entry( 6, unfilterable ),
            entry( 7, buffer_type( wgpu::BufferBindingType::Storage { read_only: false } ) ),
        ];
        let base_entries = [ entry( 0, depth ), entry( 1, level ) ];
        let down_entries = [ entry( 1, level ), entry( 2, unfilterable ) ];
        let cull_layout  = device.create_bind_group_layout( &wgpu::BindGroupLayoutDescriptor { label: Some( "cull" ), entries: &cull_entries } );
        let base_layout  = device.create_bind_group_layout( &wgpu::BindGroupLayoutDescriptor { label: Some( "pyramid base" ), entries: &base_entries } );
        let down_layout  = device.create_bind_group_layout( &wgpu::BindGroupLayoutDescriptor { label: Some( "pyramid down" ), entries: &down_entries } );

        let cull_source = wgpu::ShaderModuleDescriptor
        {
            label:  Some( "culling" ),
            source: wgpu::ShaderSource::Wgsl( include_str!( "shaders/culling.wgsl" ).into() ),
        };
        let pyramid_source = wgpu::ShaderModuleDescriptor
        {
            label:  Some( "pyramid" ),
            source: wgpu::ShaderSource::Wgsl( include_str!( "shaders/hiz.wgsl" ).into() ),
        };
        let cull_shader    = device.create_shader_module( cull_source );
        let pyramid_shader = device.create_shader_module( pyramid_source );

        let culling = Self
        {
            early_clusters: compute( device, &cull_layout, &cull_shader, "early_clusters" ),
            early_draws:    compute( device, &cull_layout, &cull_shader, "early_draws" ),
            cull_clusters:  compute( device, &cull_layout, &cull_shader, "cull_clusters" ),
            late_draws:     compute( device, &cull_layout, &cull_shader, "late_draws" ),
            base:           compute( device, &base_layout, &pyramid_shader, "base" ),
            down:           compute( device, &down_layout, &pyramid_shader, "down" ),
            cull_layout,
            base_layout,
            down_layout,
            clusters,
            records,
            cluster_count,
            record_count,
            part_count: to_u32( model.parts.len().max( 1 ) ),
            counted: device.features().contains( wgpu::Features::MULTI_DRAW_INDIRECT_COUNT ),
        };

        ( culling, ranges )
    }

    /// A camera of its own, with every cluster taken as visible until its first cull.
    pub fn view( &self, device: &wgpu::Device ) -> CullView
    {
        let empty = |label: &str, size: wgpu::BufferAddress, usage: wgpu::BufferUsages|
        {
            device.create_buffer( &wgpu::BufferDescriptor { label: Some( label ), size, usage, mapped_at_creation: false } )
        };
        let states  = vec![ VISIBLE; to_usize( self.cluster_count ) ];
        let storage = wgpu::BufferUsages::STORAGE;
        let packed  = storage | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST;
        let states  = device.create_buffer_init( &wgpu::util::BufferInitDescriptor { label: Some( "cull states" ), contents: bytemuck::cast_slice( &states ), usage: storage } );

        let view = CullView
        {
            uniform:  empty( "cull frame", UNIFORM_SIZE, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST ),
            states,
            indirect: empty( "cull draws", 2 * wgpu::BufferAddress::from( self.record_count ) * ARGS_SIZE, packed ),
            counts:   empty( "cull draw counts", 2 * COUNT_SIZE * wgpu::BufferAddress::from( self.part_count ), packed ),
            counters: empty( "cull counters", COUNTERS_SIZE, storage | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST ),
            readback: empty( "cull readback", COUNTERS_SIZE, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST ),
            pyramid:  None,
            readout:  Arc::new( AtomicU8::new( IDLE ) ),
            visible:  None,
        };

        view
    }

    /// Clusters of the model; one means there is nothing to cull.
    pub fn cluster_count( &self ) -> u32
    {
        self.cluster_count
    }

    /// The packed draws of part `index` for `view`, whose records are `records`: its triangles, or its edges.
    pub fn draw( &self, view: &CullView, render_pass: &mut wgpu::RenderPass, index: u32, records: &Range<u32>, edges: bool )
    {
        let list   = u32::from( edges );
        let offset = wgpu::BufferAddress::from( ( list * self.record_count ) + records.start ) * ARGS_SIZE;
        let count  = records.end - records.start;

        if self.counted
        {
            let count_offset = wgpu::BufferAddress::from( ( list * self.part_count ) + index ) * COUNT_SIZE;
            render_pass.multi_draw_indexed_indirect_count( &view.indirect, offset, &view.counts, count_offset, count );
        }
        else
        {
            render_pass.multi_draw_indexed_indirect( &view.indirect, offset, count );
        }
    }

    /// A pyramid of `view` for frames of `size`, built from `depth`, the multisampled depth they are drawn with.
    pub fn resize( &self, device: &wgpu::Device, view: &mut CullView, depth: &wgpu::TextureView, size: [u32; 2] )
    {
        let mips = u32::BITS - size[ 0 ].max( size[ 1 ] ).max( 1 ).leading_zeros();
        let descriptor = wgpu::TextureDescriptor
        {
            label:           Some( "pyramid" ),
            size:            wgpu::Extent3d { width: size[ 0 ], height: size[ 1 ], depth_or_array_layers: 1 },
            mip_level_count: mips,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          PYRAMID_FORMAT,
            usage:           wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats:    &[],
        };
        let texture = device.create_texture( &descriptor );
        let level   = |mip: u32| texture.create_view( &wgpu::TextureViewDescriptor { base_mip_level: mip, mip_level_count: Some( 1 ), ..Default::default() } );
        let group   = |label: &str, layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| device.create_bind_group( &wgpu::BindGroupDescriptor { label: Some( label ), layout, entries } );

        let first = level( 0 );
        let base  = group( "pyramid base", &self.base_layout, &[ texture_entry( 0, depth ), texture_entry( 1, &first ) ] );

        let mut down = Vec::new();

        for mip in 1..mips
        {
            let ( target, source ) = ( level( mip ), level( mip - 1 ) );
            down.push( group( "pyramid down", &self.down_layout, &[ texture_entry( 1, &target ), texture_entry( 2, &source ) ] ) );
        }

        let whole       = texture.create_view( &wgpu::TextureViewDescriptor::default() );
        let buffers     = [ &view.uniform, &self.clusters, &view.states, &self.records, &view.indirect, &view.counters ];
        let mut entries = ( 0.. ).zip( buffers ).map( |( binding, buffer )| wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() } ).collect::<Vec<_>>();
        entries.push( texture_entry( 6, &whole ) );
        entries.push( wgpu::BindGroupEntry { binding: 7, resource: view.counts.as_entire_binding() } );
        let cull = group( "cull", &self.cull_layout, &entries );

        let sizes   = ( 0..mips ).map( |mip| size.map( |side| ( side >> mip ).max( 1 ) ) ).collect();
        let pyramid = Pyramid { sizes, base, down, cull };
        view.pyramid = Some( pyramid );
    }

    /// Triangle draws of the early pass: clusters visible last frame and still in the frustum.
    pub fn early( &self, view: &CullView, encoder: &mut wgpu::CommandEncoder )
    {
        let Some( pyramid ) = &view.pyramid else { return };
        view.clear( encoder );
        let mut pass = encoder.begin_compute_pass( &wgpu::ComputePassDescriptor { label: Some( "cull early" ), timestamp_writes: None } );
        pass.set_bind_group( 0, &pyramid.cull, &[] );
        pass.set_pipeline( &self.early_clusters );
        pass.dispatch_workgroups( self.cluster_count.div_ceil( WORKGROUP ), 1, 1 );
        pass.set_pipeline( &self.early_draws );
        pass.dispatch_workgroups( self.record_count.div_ceil( WORKGROUP ), 1, 1 );
    }

    /// The rest of the draws; with `occlusion`, against the pyramid of the early pass's depth.
    pub fn late( &self, view: &CullView, encoder: &mut wgpu::CommandEncoder, occlusion: bool )
    {
        let Some( pyramid ) = &view.pyramid else { return };
        view.clear( encoder );
        encoder.clear_buffer( &view.counters, 0, None );
        let mut pass = encoder.begin_compute_pass( &wgpu::ComputePassDescriptor { label: Some( "cull late" ), timestamp_writes: None } );

        if occlusion
        {
            let tiles = |[ width, height ]: [u32; 2]| ( width.div_ceil( TILE ), height.div_ceil( TILE ) );
            let ( x, y ) = tiles( pyramid.sizes.first().copied().unwrap_or( [ 1, 1 ] ) );
            pass.set_pipeline( &self.base );
            pass.set_bind_group( 0, &pyramid.base, &[] );
            pass.dispatch_workgroups( x, y, 1 );
            pass.set_pipeline( &self.down );
            for ( group, &size ) in pyramid.down.iter().zip( pyramid.sizes.iter().skip( 1 ) )
            {
                let ( x, y ) = tiles( size );
                pass.set_bind_group( 0, group, &[] );
                pass.dispatch_workgroups( x, y, 1 );
            }
        }

        pass.set_bind_group( 0, &pyramid.cull, &[] );
        pass.set_pipeline( &self.cull_clusters );
        pass.dispatch_workgroups( self.cluster_count.div_ceil( WORKGROUP ), 1, 1 );
        pass.set_pipeline( &self.late_draws );
        pass.dispatch_workgroups( self.record_count.div_ceil( WORKGROUP ), 1, 1 );
    }
}

impl CullView
{
    /// Uniforms of a frame: the view, the side of `plane` kept, and with `occlusion`
    /// the depth of last frame's visible clusters, drawn first, to cull against.
    pub fn prepare( &self, queue: &wgpu::Queue, view_proj: &[f32], plane: &[f32], occlusion: bool )
    {
        let Some( pyramid ) = &self.pyramid else { return };
        let [ width, height ] = pyramid.sizes.first().copied().unwrap_or( [ 1, 1 ] );

        let mut values = [ 0u32; 24 ];
        for ( value, float ) in values.iter_mut().zip( view_proj.iter().take( 16 ).chain( plane.iter().take( 4 ) ) )
        {
            *value = float.to_bits();
        }
        values[ 20 ] = width;
        values[ 21 ] = height;
        values[ 22 ] = to_u32( pyramid.sizes.len() );
        values[ 23 ] = if occlusion { OCCLUSION | TWO_PHASE } else { 0 };

        queue.write_buffer( &self.uniform, 0, bytemuck::cast_slice( &values ) );
    }

    /// Empties the draws before a cull packs them anew.
    fn clear( &self, encoder: &mut wgpu::CommandEncoder )
    {
        encoder.clear_buffer( &self.indirect, 0, None );
        encoder.clear_buffer( &self.counts, 0, None );
    }

    /// Copies the counters out unless the previous copy is still being read; true if it did.
    pub fn read_back( &self, encoder: &mut wgpu::CommandEncoder ) -> bool
    {
        let idle = self.readout.compare_exchange( IDLE, PENDING, Ordering::AcqRel, Ordering::Acquire ).is_ok();
        if idle
        {
            encoder.copy_buffer_to_buffer( &self.counters, 0, &self.readback, 0, COUNTERS_SIZE );
        }

        idle
    }

    /// After the submit that holds [`CullView::read_back`]'s copy.
    pub fn map( &self )
    {
        let readout = Arc::clone( &self.readout );
        self.readback.map_async(
            wgpu::MapMode::Read,
            ..,
            move |result|
            {
                readout.store( if result.is_ok() { READY } else { IDLE }, Ordering::Release );
            }
        );
    }

    /// Counters of the last frame read back; also whether a read is still on the way.
    pub fn visible( &mut self, device: &wgpu::Device ) -> ( Option<Visible>, bool )
    {
        if self.readout.load( Ordering::Acquire ) == PENDING
        {
            // Only fires callbacks of finished work, it does not wait.
            let _ = device.poll( wgpu::PollType::Poll );
        }

        if self.readout.load( Ordering::Acquire ) == READY
        {
            if let Ok( range ) = self.readback.slice( .. ).get_mapped_range()
            {
                let [ clusters, triangles ] = bytemuck::pod_read_unaligned::<[u32; 2]>( &range );
                self.visible = Some( Visible { clusters, triangles } );
            }
            self.readback.unmap();
            self.readout.store( IDLE, Ordering::Release );
        }

        ( self.visible, self.readout.load( Ordering::Acquire ) == PENDING )
    }
}

/// What the adapter lacks of the API the 3D view is built for, by name.
pub fn unsupported( adapter: &wgpu::Adapter, device: &wgpu::Device ) -> Vec<&'static str>
{
    let backend   = adapter.get_info().backend;
    let downlevel = adapter.get_downlevel_capabilities().flags;
    let checks =
    [
        ( matches!( backend, wgpu::Backend::Vulkan | wgpu::Backend::Dx12 | wgpu::Backend::Metal ), "Vulkan / DirectX 12 / Metal" ),
        ( downlevel.contains( wgpu::DownlevelFlags::COMPUTE_SHADERS ), "compute shaders" ),
        ( downlevel.contains( wgpu::DownlevelFlags::INDIRECT_EXECUTION ), "indirect draws" ),
        ( device.features().contains( wgpu::Features::MULTI_DRAW_INDIRECT_COUNT ), "multi draw indirect count" ),
    ];

    let missing = checks.into_iter().filter( |&( supported, _ )| !supported ).map( |( _, name )| name ).collect();

    missing
}

fn compute( device: &wgpu::Device, layout: &wgpu::BindGroupLayout, shader: &wgpu::ShaderModule, entry_point: &str ) -> wgpu::ComputePipeline
{
    let layout_descriptor = wgpu::PipelineLayoutDescriptor
    {
        label:              Some( entry_point ),
        bind_group_layouts: &[ Some( layout ) ],
        immediate_size:     0,
    };
    let pipeline_layout = device.create_pipeline_layout( &layout_descriptor );
    let descriptor = wgpu::ComputePipelineDescriptor
    {
        label:               Some( entry_point ),
        layout:              Some( &pipeline_layout ),
        module:              shader,
        entry_point:         Some( entry_point ),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache:               None,
    };

    device.create_compute_pipeline( &descriptor )
}

fn entry( binding: u32, ty: wgpu::BindingType ) -> wgpu::BindGroupLayoutEntry
{
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None }
}

fn buffer_type( ty: wgpu::BufferBindingType ) -> wgpu::BindingType
{
    wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }
}

fn texture_entry( binding: u32, view: &wgpu::TextureView ) -> wgpu::BindGroupEntry<'_>
{
    wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView( view ) }
}
