use eframe::egui_wgpu::wgpu;

use crate::render::model::Vertex;
use crate::render::target::{ COLOR_FORMAT, DEPTH_FORMAT, SAMPLES };

/// Overlays are triangulated apart from the plain pass, so their depth is nudged toward the camera to win ties.
const OVERLAY_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState { constant: 2, slope_scale: 1.0, clamp: 0.0 };

/// A blended pipeline draws over what is there: it keeps the depth and passes on equal depth.
/// Front faces wind clockwise, as in the game's Direct3D, and stay so through the mirrored view.
pub fn pipeline( device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, fragment: &str, topology: wgpu::PrimitiveTopology, cull_mode: Option<wgpu::Face>, blend: Option<wgpu::BlendState> ) -> wgpu::RenderPipeline
{
    let attributes = wgpu::vertex_attr_array![ 0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x2 ];
    let stride     = wgpu::BufferAddress::try_from( size_of::<Vertex>() ).unwrap_or( 0 );
    let buffers    = [ Some( wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attributes } ) ];
    let targets    = [ Some( wgpu::ColorTargetState { format: COLOR_FORMAT, blend, write_mask: wgpu::ColorWrites::ALL } ) ];

    let vertex = wgpu::VertexState
    {
        module:              shader,
        entry_point:         Some( "vs_main" ),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        buffers:             &buffers,
    };
    let depth_stencil = wgpu::DepthStencilState
    {
        format:              DEPTH_FORMAT,
        depth_write_enabled: Some( blend.is_none() ),
        depth_compare:       Some( wgpu::CompareFunction::GreaterEqual ),
        stencil:             wgpu::StencilState::default(),
        bias:                if blend.is_some() { OVERLAY_BIAS } else { wgpu::DepthBiasState::default() },
    };
    let fragment_state = wgpu::FragmentState
    {
        module:              shader,
        entry_point:         Some( fragment ),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        targets:             &targets,
    };
    let descriptor = wgpu::RenderPipelineDescriptor
    {
        label:          Some( fragment ),
        layout:         Some( layout ),
        vertex,
        primitive:      wgpu::PrimitiveState { topology, front_face: wgpu::FrontFace::Cw, cull_mode, ..Default::default() },
        depth_stencil:  Some( depth_stencil ),
        multisample:    wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
        fragment:       Some( fragment_state ),
        multiview_mask: None,
        cache:          None,
    };

    device.create_render_pipeline( &descriptor )
}

pub fn uniform_entry( binding: u32, visibility: wgpu::ShaderStages ) -> wgpu::BindGroupLayoutEntry
{
    let entry = wgpu::BindGroupLayoutEntry
    {
        binding,
        visibility,
        ty:    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    };

    entry
}
