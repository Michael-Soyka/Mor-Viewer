use eframe::egui::TextureId;
use eframe::egui_wgpu::{ RenderState, wgpu };

/// Halvings before the gaussian: it runs at a quarter of the frame.
const HALVINGS: usize = 2;

/// Gaussian rounds at that size; each widens the blur.
const ROUNDS: usize = 2;

/// Blurred copy of a frame, for the plates floating over it.
pub struct Blur
{
    state:      RenderState,
    format:     wgpu::TextureFormat,
    layout:     wgpu::BindGroupLayout,
    sampler:    wgpu::Sampler,
    down:       wgpu::RenderPipeline,
    horizontal: wgpu::RenderPipeline,
    vertical:   wgpu::RenderPipeline,
    chain:      Option<Chain>,
    texture_id: Option<TextureId>,
}

/// Textures for one frame size.
struct Chain
{
    /// Halving passes: the group each reads and the view it draws into; the last view holds the result.
    halvings: Vec<(wgpu::BindGroup, wgpu::TextureView)>,

    /// Reads the result.
    result: wgpu::BindGroup,

    scratch:       wgpu::TextureView,
    scratch_group: wgpu::BindGroup,
}

impl Blur
{
    /// `format` is the frame's.
    pub fn new( state: &RenderState, format: wgpu::TextureFormat ) -> Self
    {
        let device = &state.device;
        let shader_source = wgpu::ShaderModuleDescriptor
        {
            label:  Some( "blur" ),
            source: wgpu::ShaderSource::Wgsl( include_str!( "shaders/blur.wgsl" ).into() ),
        };
        let shader = device.create_shader_module( shader_source );

        let entries =
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
        ];
        let layout = device.create_bind_group_layout( &wgpu::BindGroupLayoutDescriptor { label: Some( "blur" ), entries: &entries } );
        let layout_descriptor = wgpu::PipelineLayoutDescriptor
        {
            label:              Some( "blur" ),
            bind_group_layouts: &[ Some( &layout ) ],
            immediate_size:     0,
        };
        let pipeline_layout = device.create_pipeline_layout( &layout_descriptor );

        let pipeline = |fragment: &str|
        {
            let targets = [ Some( wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL } ) ];
            let vertex = wgpu::VertexState
            {
                module:              &shader,
                entry_point:         Some( "vs_main" ),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers:             &[],
            };
            let fragment_state = wgpu::FragmentState
            {
                module:              &shader,
                entry_point:         Some( fragment ),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets:             &targets,
            };
            let descriptor = wgpu::RenderPipelineDescriptor
            {
                label:          Some( fragment ),
                layout:         Some( &pipeline_layout ),
                vertex,
                primitive:      wgpu::PrimitiveState::default(),
                depth_stencil:  None,
                multisample:    wgpu::MultisampleState::default(),
                fragment:       Some( fragment_state ),
                multiview_mask: None,
                cache:          None,
            };
            device.create_render_pipeline( &descriptor )
        };

        let sampler_descriptor = wgpu::SamplerDescriptor
        {
            label:      Some( "blur" ),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        };

        let blur = Self
        {
            state:      state.clone(),
            format,
            sampler:    device.create_sampler( &sampler_descriptor ),
            down:       pipeline( "fs_down" ),
            horizontal: pipeline( "fs_horizontal" ),
            vertical:   pipeline( "fs_vertical" ),
            layout,
            chain:      None,
            texture_id: None,
        };

        blur
    }

    /// The blurred frame as an egui texture, once sized.
    pub fn id( &self ) -> Option<TextureId>
    {
        self.texture_id
    }

    /// Textures for frames of `size` read from `source`.
    pub fn resize( &mut self, source: &wgpu::TextureView, size: [u32; 2] )
    {
        let mut halvings = Vec::new();
        let mut group    = self.group( source );
        let mut size     = size;

        for _ in 0..HALVINGS
        {
            size = size.map( |side| ( side / 2 ).max( 1 ) );
            let view = self.texture( size );
            let next = self.group( &view );
            halvings.push( ( std::mem::replace( &mut group, next ), view ) );
        }

        let scratch       = self.texture( size );
        let scratch_group = self.group( &scratch );

        let Some( ( _, result ) ) = halvings.last() else { return };
        let device       = &self.state.device;
        let mut renderer = self.state.renderer.write();

        match self.texture_id
        {
            Some( id ) => renderer.update_egui_texture_from_wgpu_texture( device, result, wgpu::FilterMode::Linear, id ),
            None => self.texture_id = Some( renderer.register_native_texture( device, result, wgpu::FilterMode::Linear ) ),
        }
        self.chain = Some( Chain { halvings, result: group, scratch, scratch_group } );
    }

    /// Blurs the frame last drawn into the source.
    pub fn run( &self )
    {
        let Some( chain ) = &self.chain else { return };
        let Some( ( _, result ) ) = chain.halvings.last() else { return };
        let mut encoder = self.state.device.create_command_encoder( &wgpu::CommandEncoderDescriptor { label: Some( "blur" ) } );

        for ( group, view ) in &chain.halvings
        {
            pass( &mut encoder, &self.down, group, view );
        }
        for _ in 0..ROUNDS
        {
            pass( &mut encoder, &self.horizontal, &chain.result, &chain.scratch );
            pass( &mut encoder, &self.vertical, &chain.scratch_group, result );
        }

        self.state.queue.submit( [ encoder.finish() ] );
    }

    fn texture( &self, size: [u32; 2] ) -> wgpu::TextureView
    {
        let descriptor = wgpu::TextureDescriptor
        {
            label:           Some( "blur" ),
            size:            wgpu::Extent3d { width: size[ 0 ], height: size[ 1 ], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          self.format,
            usage:           wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats:    &[],
        };

        self.state.device.create_texture( &descriptor ).create_view( &wgpu::TextureViewDescriptor::default() )
    }

    fn group( &self, view: &wgpu::TextureView ) -> wgpu::BindGroup
    {
        let entries =
        [
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView( view ) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler( &self.sampler ) },
        ];

        self.state.device.create_bind_group( &wgpu::BindGroupDescriptor { label: Some( "blur" ), layout: &self.layout, entries: &entries } )
    }
}

impl Drop for Blur
{
    fn drop( &mut self )
    {
        if let Some( id ) = self.texture_id
        {
            self.state.renderer.write().free_texture( &id );
        }
    }
}

/// `pipeline` over the whole `target`, reading `group`.
fn pass( encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup, target: &wgpu::TextureView )
{
    let color = wgpu::RenderPassColorAttachment
    {
        view:           target,
        depth_slice:    None,
        resolve_target: None,
        ops:            wgpu::Operations { load: wgpu::LoadOp::Clear( wgpu::Color::TRANSPARENT ), store: wgpu::StoreOp::Store },
    };
    let descriptor = wgpu::RenderPassDescriptor
    {
        label:                    Some( "blur" ),
        color_attachments:        &[ Some( color ) ],
        depth_stencil_attachment: None,
        timestamp_writes:         None,
        occlusion_query_set:      None,
        multiview_mask:           None,
    };
    let mut render_pass = encoder.begin_render_pass( &descriptor );

    render_pass.set_pipeline( pipeline );
    render_pass.set_bind_group( 0, group, &[] );
    render_pass.draw( 0..3, 0..1 );
}
