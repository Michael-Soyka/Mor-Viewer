use eframe::egui_wgpu::wgpu;
use wgpu::util::DeviceExt;

use crate::render::culling::{ CullView, Culling };
use crate::render::gpu::{ FRAME_FLOATS, FrameUniforms };
use crate::render::math::mul;
use crate::render::pipeline::pipeline;
use crate::render::target::Target;

/// Reflections are drawn at this fraction of the frame size.
const REFLECTION_DIVISOR: u32 = 2;

/// A reflection keeps this much below its water level, so shores meet their mirror image without a gap.
const CLIP_BIAS: f32 = 1.0;

/// Planar reflection of the scene in its water: the frame drawn once more from under the water level,
/// with the view mirrored in it and everything below it cut away.
/// One level is mirrored at a time, the one whose water is nearest under the eye; see [`Water::level`].
pub struct Water
{
    /// The water parts over the reflection.
    pub surface: wgpu::RenderPipeline,

    /// Front faces as the mirrored view sees them, wound the other way.
    pub mirrored: wgpu::RenderPipeline,

    pub layout:      wgpu::BindGroupLayout,
    pub sampler:     wgpu::Sampler,
    pub frame:       wgpu::Buffer,
    pub frame_group: wgpu::BindGroup,
    pub culling:     CullView,

    pub levels:     Vec<WaterLevel>,
    pub reflection: Option<Reflection>,
}

/// Water parts of one height and the boxes of the clusters they lie in.
pub struct WaterLevel
{
    pub height: f32,
    pub boxes:  Vec<([f32; 3], [f32; 3])>,
}

impl WaterLevel
{
    /// From `point` to the nearest of the boxes.
    fn distance( &self, point: [f32; 3] ) -> f32
    {
        let to_box = |&( min, max ): &( [f32; 3], [f32; 3] )|
        {
            let gap: [f32; 3] = std::array::from_fn( |axis| ( min[ axis ] - point[ axis ] ).max( point[ axis ] - max[ axis ] ).max( 0.0 ) );
            ( ( gap[ 0 ] * gap[ 0 ] ) + ( gap[ 1 ] * gap[ 1 ] ) + ( gap[ 2 ] * gap[ 2 ] ) ).sqrt()
        };

        self.boxes.iter().map( to_box ).fold( f32::MAX, f32::min )
    }
}

/// The reflection's frame and the bind group the water reads it through.
pub struct Reflection
{
    pub target: Target,
    pub group:  wgpu::BindGroup,
}

/// How a pass draws the water parts.
#[ derive( Clone, Copy ) ]
pub enum WaterPass<'a>
{
    /// As any other part, in the untextured modes.
    Plain,

    /// Over the reflection: the pipeline and the reflection's bind group.
    Mirror( &'a wgpu::RenderPipeline, &'a wgpu::BindGroup ),

    /// Left out, from the reflection itself.
    Hidden,
}

/// The frame mirrored in the water level at `level`: the view flipped under the plane, the eye with it,
/// and everything below the plane cut away.
pub fn reflected( uniforms: &FrameUniforms, level: f32 ) -> FrameUniforms
{
    let view_proj: [[f32; 4]; 4] = std::array::from_fn( |column| std::array::from_fn( |row| uniforms[ ( column * 4 ) + row ] ) );
    let flip = [ [ 1.0, 0.0, 0.0, 0.0 ], [ 0.0, -1.0, 0.0, 0.0 ], [ 0.0, 0.0, 1.0, 0.0 ], [ 0.0, 2.0 * level, 0.0, 1.0 ] ];

    let mut mirrored = *uniforms;
    mirrored[ ..16 ].copy_from_slice( mul( view_proj, flip ).as_flattened() );
    mirrored[ 25 ] = ( 2.0 * level ) - uniforms[ 25 ];
    mirrored[ 40..44 ].copy_from_slice( &[ 0.0, 1.0, 0.0, CLIP_BIAS - level ] );
    mirrored[ 45 ] = 0.0;

    mirrored
}

impl Water
{
    /// `layouts` are the frame's and the parts'; `reflection_entries` lay out a texture and its sampler.
    pub fn new( device: &wgpu::Device, shader: &wgpu::ShaderModule, layouts: [&wgpu::BindGroupLayout; 2], reflection_entries: &[wgpu::BindGroupLayoutEntry], culling: &Culling, levels: Vec<WaterLevel> ) -> Self
    {
        let [ frame_layout, part_layout ] = layouts;
        let layout = device.create_bind_group_layout( &wgpu::BindGroupLayoutDescriptor { label: Some( "reflection" ), entries: reflection_entries } );

        let pipeline_layout = |label: &str, groups: &[Option<&wgpu::BindGroupLayout>]|
        {
            let descriptor = wgpu::PipelineLayoutDescriptor { label: Some( label ), bind_group_layouts: groups, immediate_size: 0 };
            device.create_pipeline_layout( &descriptor )
        };
        let surface_layout = pipeline_layout( "water", &[ Some( frame_layout ), Some( part_layout ), Some( &layout ) ] );
        let model_layout   = pipeline_layout( "reflection", &[ Some( frame_layout ), Some( part_layout ) ] );
        let surface        = pipeline( device, &surface_layout, shader, "fs_water", wgpu::PrimitiveTopology::TriangleList, None, Some( wgpu::BlendState::ALPHA_BLENDING ) );
        let mirrored       = pipeline( device, &model_layout, shader, "fs_main", wgpu::PrimitiveTopology::TriangleList, Some( wgpu::Face::Front ), None );

        let sampler_descriptor = wgpu::SamplerDescriptor
        {
            label:      Some( "reflection" ),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        };
        let sampler = device.create_sampler( &sampler_descriptor );

        let frame_descriptor = wgpu::util::BufferInitDescriptor
        {
            label:    Some( "reflection frame" ),
            contents: bytemuck::cast_slice( &[ 0.0f32; FRAME_FLOATS ] ),
            usage:    wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        };
        let frame         = device.create_buffer_init( &frame_descriptor );
        let frame_binding = [ wgpu::BindGroupEntry { binding: 0, resource: frame.as_entire_binding() } ];
        let frame_group   = device.create_bind_group( &wgpu::BindGroupDescriptor { label: Some( "reflection frame" ), layout: frame_layout, entries: &frame_binding } );

        let water = Self { surface, mirrored, layout, sampler, frame, frame_group, culling: culling.view( device ), levels, reflection: None };

        water
    }

    /// What the water reads: the reflection in `resolve`.
    fn group( &self, device: &wgpu::Device, resolve: &wgpu::TextureView ) -> wgpu::BindGroup
    {
        let entries =
        [
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView( resolve ) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler( &self.sampler ) },
        ];

        device.create_bind_group( &wgpu::BindGroupDescriptor { label: Some( "reflection" ), layout: &self.layout, entries: &entries } )
    }

    /// The level mirrored for an eye at `eye`: of those under it, as water above the eye shows its underside,
    /// the one whose water comes nearest; a far pond does not take the reflection from the river below.
    pub fn level( &self, eye: [f32; 3] ) -> Option<f32>
    {
        let under = self.levels.iter().filter( |level| level.height < eye[ 1 ] );
        let level = under.min_by( |a, b| a.distance( eye ).total_cmp( &b.distance( eye ) ) )?;

        Some( level.height )
    }

    /// A reflection for frames of `size`, smaller by [`REFLECTION_DIVISOR`].
    pub fn resize( &mut self, device: &wgpu::Device, culling: &Culling, size: [u32; 2] )
    {
        let size   = size.map( |side| ( side / REFLECTION_DIVISOR ).max( 1 ) );
        let target = Target::new( device, size );
        culling.resize( device, &mut self.culling, &target.depth, size );

        let group      = self.group( device, &target.resolve );
        let reflection = Reflection { target, group };
        self.reflection = Some( reflection );
    }
}
