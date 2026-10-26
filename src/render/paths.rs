use eframe::egui_wgpu::wgpu;
use wgpu::util::DeviceExt;

use crate::render::gpu::non_empty;
use crate::render::model::{ Vertex, to_u32 };
use crate::render::pipeline::{ pipeline, uniform_entry };

/// Navigation lines of a scene; the color rides in the normal slot of each vertex.
pub struct PathLines
{
    pub vertices: Vec<Vertex>,

    /// Line list over `vertices`: the cell outlines.
    pub cells: Vec<u32>,

    /// Line list over `vertices`: the graph links.
    pub links: Vec<u32>,
}

/// Which path lines to draw.
#[ derive( Clone, Copy, PartialEq ) ]
pub struct PathView
{
    pub cells: bool,
    pub links: bool,
}

pub struct GpuPaths
{
    pub pipeline: wgpu::RenderPipeline,
    pub vertices: wgpu::Buffer,
    pub indices:  wgpu::Buffer,
    pub cells:    std::ops::Range<u32>,
    pub links:    std::ops::Range<u32>,
}

impl GpuPaths
{
    pub fn new( device: &wgpu::Device, lines: &PathLines ) -> Self
    {
        let shader_source = wgpu::ShaderModuleDescriptor
        {
            label:  Some( "paths" ),
            source: wgpu::ShaderSource::Wgsl( include_str!( "shaders/model.wgsl" ).into() ),
        };
        let shader = device.create_shader_module( shader_source );

        let frame_entries = [ uniform_entry( 0, wgpu::ShaderStages::VERTEX_FRAGMENT ) ];
        let frame_layout  = device.create_bind_group_layout( &wgpu::BindGroupLayoutDescriptor { label: Some( "paths frame" ), entries: &frame_entries } );
        let layout_descriptor = wgpu::PipelineLayoutDescriptor
        {
            label:              Some( "paths" ),
            bind_group_layouts: &[ Some( &frame_layout ) ],
            immediate_size:     0,
        };
        let layout   = device.create_pipeline_layout( &layout_descriptor );
        let pipeline = pipeline( device, &layout, &shader, "fs_path", wgpu::PrimitiveTopology::LineList, None, None );

        let indices: Vec<u32> = lines.cells.iter().chain( &lines.links ).copied().collect();
        let cells_end         = to_u32( lines.cells.len() );
        let links_end         = to_u32( indices.len() );

        let buffer = |label: &str, contents: &[u8], usage: wgpu::BufferUsages|
        {
            device.create_buffer_init( &wgpu::util::BufferInitDescriptor { label: Some( label ), contents, usage } )
        };
        let paths = Self
        {
            pipeline,
            vertices: buffer( "path vertices", bytemuck::cast_slice( &lines.vertices ), wgpu::BufferUsages::VERTEX ),
            indices:  buffer( "path indices", bytemuck::cast_slice( &non_empty( &indices ) ), wgpu::BufferUsages::INDEX ),
            cells:    0..cells_end,
            links:    cells_end..links_end,
        };

        paths
    }
}
