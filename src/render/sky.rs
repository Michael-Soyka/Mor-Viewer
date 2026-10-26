use eframe::egui_wgpu::{ RenderState, wgpu };

use crate::render::pipeline::uniform_entry;
use crate::render::target::{ COLOR_FORMAT, DEPTH_FORMAT, SAMPLES };
use crate::render::weather::SkyImages;

/// Corners of the sky cube's triangles, see `vs_sky` in `sky.wgsl`.
pub const SKY_VERTICES: u32 = 36;

/// The sky cube and the faces it reads.
pub struct GpuSky
{
    pub pipeline: wgpu::RenderPipeline,
    pub group:    wgpu::BindGroup,
}

impl GpuSky
{
    pub fn new( state: &RenderState, images: &SkyImages ) -> Self
    {
        let device = &state.device;
        let shader_source = wgpu::ShaderModuleDescriptor
        {
            label:  Some( "sky" ),
            source: wgpu::ShaderSource::Wgsl( include_str!( "shaders/sky.wgsl" ).into() ),
        };
        let shader = device.create_shader_module( shader_source );

        let frame_entries = [ uniform_entry( 0, wgpu::ShaderStages::VERTEX_FRAGMENT ) ];
        let frame_layout  = device.create_bind_group_layout( &wgpu::BindGroupLayoutDescriptor { label: Some( "sky frame" ), entries: &frame_entries } );
        let face_entries =
        [
            wgpu::BindGroupLayoutEntry
            {
                binding:    0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty:         wgpu::BindingType::Texture
                {
                    sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
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
        ];
        let face_layout = device.create_bind_group_layout( &wgpu::BindGroupLayoutDescriptor { label: Some( "sky faces" ), entries: &face_entries } );
        let layout_descriptor = wgpu::PipelineLayoutDescriptor
        {
            label:              Some( "sky" ),
            bind_group_layouts: &[ Some( &frame_layout ), Some( &face_layout ) ],
            immediate_size:     0,
        };
        let layout = device.create_pipeline_layout( &layout_descriptor );

        let targets = [ Some( wgpu::ColorTargetState { format: COLOR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL } ) ];
        let vertex = wgpu::VertexState
        {
            module:              &shader,
            entry_point:         Some( "vs_sky" ),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers:             &[],
        };
        let fragment = wgpu::FragmentState
        {
            module:              &shader,
            entry_point:         Some( "fs_sky" ),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets:             &targets,
        };
        // At the far depth and leaving it as cleared: everything after draws over the sky.
        let depth_stencil = wgpu::DepthStencilState
        {
            format:              DEPTH_FORMAT,
            depth_write_enabled: Some( false ),
            depth_compare:       Some( wgpu::CompareFunction::Always ),
            stencil:             wgpu::StencilState::default(),
            bias:                wgpu::DepthBiasState::default(),
        };
        let descriptor = wgpu::RenderPipelineDescriptor
        {
            label:          Some( "sky" ),
            layout:         Some( &layout ),
            vertex,
            primitive:      wgpu::PrimitiveState::default(),
            depth_stencil:  Some( depth_stencil ),
            multisample:    wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
            fragment:       Some( fragment ),
            multiview_mask: None,
            cache:          None,
        };
        let pipeline = device.create_render_pipeline( &descriptor );

        let texture_descriptor = wgpu::TextureDescriptor
        {
            label:           Some( "sky faces" ),
            size:            wgpu::Extent3d { width: images.side, height: images.side, depth_or_array_layers: images.layers },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          wgpu::TextureFormat::Rgba8Unorm,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        };
        let texture     = device.create_texture( &texture_descriptor );
        let destination = wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All };
        let data_layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some( images.side * 4 ), rows_per_image: Some( images.side ) };
        state.queue.write_texture( destination, &images.rgba, data_layout, texture_descriptor.size );

        let view    = texture.create_view( &wgpu::TextureViewDescriptor { dimension: Some( wgpu::TextureViewDimension::D2Array ), ..Default::default() } );
        let sampler = device.create_sampler( &wgpu::SamplerDescriptor { label: Some( "sky" ), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() } );
        let entries =
        [
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView( &view ) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler( &sampler ) },
        ];
        let group = device.create_bind_group( &wgpu::BindGroupDescriptor { label: Some( "sky faces" ), layout: &face_layout, entries: &entries } );

        let sky = Self { pipeline, group };

        sky
    }
}
