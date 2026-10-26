use eframe::egui::{ self, Color32 };

use crate::render::gpu::{ FRAME_FLOATS, FrameUniforms };
use crate::render::weather::Weather;
use crate::ui::icons;
use crate::ui::widgets::to_f32;
use crate::viewer::model::camera::Camera;

#[ derive( Clone, Copy, PartialEq, Eq ) ]
pub enum RenderMode
{
    Wireframe,
    Solid,
    Material,
    Shaded,
}

impl RenderMode
{
    pub const ALL: [Self; 4] = [ Self::Wireframe, Self::Solid, Self::Material, Self::Shaded ];

    pub fn icon( self ) -> egui::ImageSource<'static>
    {
        match self
        {
            Self::Wireframe => icons::WIREFRAME,
            Self::Solid => icons::SOLID,
            Self::Material => icons::MATERIAL,
            Self::Shaded => icons::SHADED,
        }
    }

    pub fn key( self ) -> &'static str
    {
        match self
        {
            Self::Wireframe => "model.wireframe",
            Self::Solid => "model.solid",
            Self::Material => "model.material",
            Self::Shaded => "model.shaded",
        }
    }

    pub fn index( self ) -> f32
    {
        match self
        {
            Self::Wireframe => 0.0,
            Self::Solid => 1.0,
            Self::Material => 2.0,
            Self::Shaded => 3.0,
        }
    }
}

/// Hour, 0 to 24, and weather of an outdoor level; see [`weather::at`].
#[ derive( Clone, Copy, PartialEq ) ]
pub struct TimeOfDay
{
    pub hour:    f32,
    pub weather: Weather,

    /// The street lights burn at night only, as the world script switches them; see [`ModelViewer::with_street_lights`].
    pub street_lights: bool,
}

/// Distance fog of the viewer; the game sets its own from scripts and the time of day.
#[ derive( Clone, Copy, PartialEq ) ]
pub struct Fog
{
    pub start: f32,
    pub end:   f32,
    pub color: Color32,
}

impl Fog
{
    /// A grey haze that hides the city edge, picked by eye.
    pub const DEFAULT: Self = Self { start: 3_000.0, end: 20_000.0, color: Color32::from_rgb( 0x8A, 0x8F, 0x94 ) };
}

/// Switches of the render menu.
#[ derive( Clone, Copy, PartialEq, Eq ) ]
pub struct RenderOptions
{
    /// Draw back faces too, as the game does for most surfaces.
    pub no_cull: bool,

    pub lightmaps: bool,
    pub fog:       bool,

    /// Leave out clusters hidden behind others, see [`Culling`].
    pub occlusion: bool,

    /// Mirror the scene in the water; without, it shows the background, and the frame is drawn once.
    pub reflections: bool,

    /// Let the hour pick the sky, light and fog of an outdoor level; see [`TimeOfDay`].
    pub time_of_day: bool,
}

impl RenderOptions
{
    /// A single object; scenes turn lightmaps and occlusion on, see [`ModelViewer::with_lightmaps`].
    pub const DEFAULT: Self = Self { no_cull: true, lightmaps: false, fog: false, occlusion: false, reflections: false, time_of_day: false };
}

/// Directional sun fixed in the world.
#[ derive( Clone, Copy ) ]
pub struct Light
{
    pub enabled: bool,

    /// Degrees around the vertical axis.
    pub azimuth: f32,

    /// Degrees above the horizon.
    pub elevation: f32,
}

impl Light
{
    pub const DEFAULT: Self = Self { enabled: true, azimuth: 45.0, elevation: 50.0 };

    /// Towards the sun, in the mirrored space of `Camera`.
    fn direction( self ) -> [f32; 3]
    {
        let ( sin_azimuth, cos_azimuth ) = self.azimuth.to_radians().sin_cos();

        let ( sin_elevation, cos_elevation ) = self.elevation.to_radians().sin_cos();

        [ cos_elevation * sin_azimuth, sin_elevation, cos_elevation * cos_azimuth ]
    }
}

pub fn frame_uniforms( camera: &Camera, mode: RenderMode, light: Light, options: RenderOptions, ambient: Option<[f32; 3]>, fog: Option<Fog>, size: [u32; 2] ) -> FrameUniforms
{
    let [ width, height ] = size.map( |side| to_f32( usize::try_from( side ).unwrap_or( 1 ) ).max( 1.0 ) );
    // The fog hides everything past its end, so the far plane clips there, as in the game.
    let view_proj = camera.view_proj( width / height, fog.map( |fog| fog.end ) );

    let [ sun_x, sun_y, sun_z ] = light.direction();

    let [ eye_x, eye_y, eye_z ] = camera.eye();

    let mut uniforms: FrameUniforms = [ 0.0; FRAME_FLOATS ];
    uniforms[ ..16 ].copy_from_slice( view_proj.as_flattened() );
    // Back to the model's own handedness, as the normals and positions are.
    uniforms[ 16..19 ].copy_from_slice( &[ sun_x, sun_y, -sun_z ] );
    uniforms[ 20 ] = mode.index();
    uniforms[ 21 ] = if light.enabled { 1.0 } else { 0.0 };
    uniforms[ 22 ] = if options.lightmaps { 1.0 } else { 0.0 };
    uniforms[ 24..27 ].copy_from_slice( &[ eye_x, eye_y, -eye_z ] );
    if let Some( ambient ) = ambient
    {
        uniforms[ 28..31 ].copy_from_slice( &ambient );
        uniforms[ 31 ] = 1.0;
    }
    if let Some( fog ) = fog
    {
        let [ red, green, blue, _ ] = fog.color.to_normalized_gamma_f32();
        uniforms[ 32..36 ].copy_from_slice( &[ red, green, blue, 1.0 ] );
        uniforms[ 36..38 ].copy_from_slice( &[ fog.start, fog.end ] );
    }
    // No clip plane: everything is on its kept side.
    uniforms[ 43 ] = 1.0;
    uniforms[ 46..48 ].copy_from_slice( &[ 1.0 / width, 1.0 / height ] );

    uniforms
}
