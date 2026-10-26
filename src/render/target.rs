use eframe::egui_wgpu::wgpu;

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const SAMPLES:      u32                 = 4;

pub struct Target
{
    pub size:    [u32; 2],
    pub msaa:    wgpu::TextureView,
    pub resolve: wgpu::TextureView,
    pub depth:   wgpu::TextureView,
}

impl Target
{
    /// Multisampled color and depth of `size`, and the color resolved for reading.
    pub fn new( device: &wgpu::Device, size: [u32; 2] ) -> Self
    {
        let texture = |label: &str, format: wgpu::TextureFormat, samples: u32, usage: wgpu::TextureUsages|
        {
            let descriptor = wgpu::TextureDescriptor
            {
                label:           Some( label ),
                size:            wgpu::Extent3d { width: size[ 0 ], height: size[ 1 ], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count:    samples,
                dimension:       wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats:    &[],
            };
            device.create_texture( &descriptor ).create_view( &wgpu::TextureViewDescriptor::default() )
        };
        let target = Self
        {
            size,
            msaa:    texture( "model msaa", COLOR_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT ),
            resolve: texture( "model color", COLOR_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING ),
            depth:   texture( "model depth", DEPTH_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING ),
        };

        target
    }
}

/// A pass over the frame: `clear` starts it anew, otherwise it goes on from the previous pass; the `last` one resolves.
pub fn begin_pass<'a>( encoder: &'a mut wgpu::CommandEncoder, target: &Target, clear: Option<wgpu::Color>, last: bool ) -> wgpu::RenderPass<'a>
{
    let store = if last { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store };
    let color = wgpu::RenderPassColorAttachment
    {
        view:           &target.msaa,
        depth_slice:    None,
        resolve_target: last.then_some( &target.resolve ),
        ops:            wgpu::Operations { load: clear.map_or( wgpu::LoadOp::Load, wgpu::LoadOp::Clear ), store },
    };
    let depth = wgpu::RenderPassDepthStencilAttachment
    {
        view:        &target.depth,
        depth_ops:   Some( wgpu::Operations { load: if clear.is_some() { wgpu::LoadOp::Clear( 0.0 ) } else { wgpu::LoadOp::Load }, store } ),
        stencil_ops: None,
    };
    let descriptor = wgpu::RenderPassDescriptor
    {
        label:                    Some( "model" ),
        color_attachments:        &[ Some( color ) ],
        depth_stencil_attachment: Some( depth ),
        timestamp_writes:         None,
        occlusion_query_set:      None,
        multiview_mask:           None,
    };

    encoder.begin_render_pass( &descriptor )
}
