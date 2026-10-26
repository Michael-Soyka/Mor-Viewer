use std::collections::{ HashMap, HashSet };
use std::time::Instant;

use eframe::egui::{ self, Color32, Key, Pos2, Rect, Sense, Stroke, TextureId, Vec2 };
use eframe::egui_wgpu::{ RenderState, wgpu };
use wgpu::util::DeviceExt;

use rust_i18n::t;
use crate::animation::Player;
use crate::blur::Blur;
use crate::i18n;
use crate::icons;
use crate::culling::{ self, CullView, Culling };
use crate::model::{ Model, Vertex, to_usize };
use crate::settings::ViewerSettings;
use crate::theme::{ self, Palette };
use crate::viewer::Sections;
use crate::widgets::{ self, to_f32 };

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Vertices per buffer: 40-byte vertices stay well under wgpu's 256 MiB default buffer limit.
const CHUNK_VERTICES: usize = 4_000_000;

/// Overlays are triangulated apart from the plain pass, so their depth is nudged toward the camera to win ties.
const OVERLAY_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState { constant: 2, slope_scale: 1.0, clamp: 0.0 };

/// Byte offset of the light color in a part's uniform, after the base color.
const LIGHT_OFFSET: wgpu::BufferAddress = 16;

/// Fog sliders reach this far, past the city's size; the end keeps this gap after the start.
const FOG_MAX:      f32 = 60_000.0;
const FOG_MIN_SPAN: f32 = 100.0;

/// Lightmap overlays add up on top of the plain pass.
const ADDITIVE: wgpu::BlendState = wgpu::BlendState
{
    color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
    alpha: wgpu::BlendComponent::REPLACE,
};

/// Lightmaps are 8-bit luminance.
const LIGHTMAP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

const SAMPLES:      u32                 = 4;
const FOV_Y:        f32                 = std::f32::consts::FRAC_PI_4;
const ORBIT_SPEED:  f32                 = 0.01;
const ZOOM_SPEED:   f32                 = 0.0015;
const PITCH_LIMIT:  f32                 = 1.55;
const LOOK_SPEED:   f32                 = 0.004;
const FLY_SPEED:    f32                 = 800.0;
const FLY_BOOST:    f32                 = 5.0;
const FLY_NEAR:     f32                 = 5.0;

/// Axis gizmo: plate, line length from its center, line width.
/// Fly controls hint: shown this long after entering fly mode, fading out at the end.
const HINT_SECONDS: f64 = 5.0;
const HINT_FADE:    f64 = 0.5;

/// Light markers: dot radius and how close the pointer has to be to pick one.
const LIGHT_MARKER: f32 = 4.0;
const LIGHT_PICK:   f32 = 7.0;

/// Frame time: frames further apart than this follow a pause; share of each new frame in the average.
const FRAME_GAP:       f32 = 0.25;
const FRAME_SMOOTHING: f32 = 0.1;

const PIVOT_PLATE:  f32 = 72.0;
const PIVOT_LENGTH: f32 = 26.0;
const PIVOT_STROKE: f32 = 2.0;

/// Game X, Y and Z in the mirrored space, with their colors.
const PIVOT_AXES: [([f32; 3], Color32); 3] =
[
    ( [ 1.0, 0.0, 0.0 ], Color32::from_rgb( 0x98, 0x4B, 0x4B ) ),
    ( [ 0.0, 1.0, 0.0 ], Color32::from_rgb( 0x51, 0x98, 0x4B ) ),
    ( [ 0.0, 0.0, -1.0 ], Color32::from_rgb( 0x56, 0x93, 0xE3 ) ),
];

/// Frame uniforms: view-projection, light, params, eye.
type FrameUniforms = [f32; 40];

/// Path and data of a texture file by the name a material refers to.
pub type FindTexture<'a> = &'a dyn Fn( &str ) -> Option<(String, Vec<u8>)>;

/// Decoded textures by lowercase name.
pub type Textures = HashMap<String, DecodedTexture>;

/// The game uses left-handed coordinates: flipping z turns them into right-handed ones.
const MIRROR: [[f32; 4]; 4] = [ [ 1.0, 0.0, 0.0, 0.0 ], [ 0.0, 1.0, 0.0, 0.0 ], [ 0.0, 0.0, -1.0, 0.0 ], [ 0.0, 0.0, 0.0, 1.0 ] ];

#[ derive( Clone, Copy, PartialEq, Eq ) ]
enum RenderMode
{
    Wireframe,
    Solid,
    Material,
    Shaded,
}

impl RenderMode
{
    const ALL: [Self; 4] = [ Self::Wireframe, Self::Solid, Self::Material, Self::Shaded ];

    fn icon( self ) -> egui::ImageSource<'static>
    {
        match self
        {
            Self::Wireframe => icons::WIREFRAME,
            Self::Solid => icons::SOLID,
            Self::Material => icons::MATERIAL,
            Self::Shaded => icons::SHADED,
        }
    }

    fn key( self ) -> &'static str
    {
        match self
        {
            Self::Wireframe => "model.wireframe",
            Self::Solid => "model.solid",
            Self::Material => "model.material",
            Self::Shaded => "model.shaded",
        }
    }

    fn index( self ) -> f32
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

/// Distance fog of the viewer; the game sets its own from scripts and the time of day.
#[ derive( Clone, Copy, PartialEq ) ]
struct Fog
{
    start: f32,
    end:   f32,
    color: Color32,
}

impl Fog
{
    /// A grey haze that hides the city edge, picked by eye.
    const DEFAULT: Self = Self { start: 3_000.0, end: 20_000.0, color: Color32::from_rgb( 0x8A, 0x8F, 0x94 ) };
}

/// Switches of the render menu.
#[ derive( Clone, Copy, PartialEq, Eq ) ]
struct RenderOptions
{
    /// Draw back faces too, as the game does for most surfaces.
    no_cull: bool,

    lightmaps: bool,
    fog:       bool,

    /// Leave out clusters hidden behind others, see [`Culling`].
    occlusion: bool,

}

impl RenderOptions
{
    /// A single object; scenes turn lightmaps on, see [`ModelViewer::with_lightmaps`].
    const DEFAULT: Self = Self { no_cull: true, lightmaps: false, fog: false };
}

/// Pipeline a frame is drawn with.
#[ derive( Clone, Copy ) ]
enum Pass
{
    Wireframe,
    TwoSided,
    FrontFaces,
}

/// Directional sun fixed in the world.
#[ derive( Clone, Copy ) ]
struct Light
{
    enabled: bool,

    /// Degrees around the vertical axis.
    azimuth: f32,

    /// Degrees above the horizon.
    elevation: f32,
}

impl Light
{
    const DEFAULT: Self = Self { enabled: true, azimuth: 45.0, elevation: 50.0 };

    /// Towards the sun, in the mirrored space of `Camera`.
    fn direction( self ) -> [f32; 3]
    {
        let ( sin_azimuth, cos_azimuth ) = self.azimuth.to_radians().sin_cos();

        let ( sin_elevation, cos_elevation ) = self.elevation.to_radians().sin_cos();

        [ cos_elevation * sin_azimuth, sin_elevation, cos_elevation * cos_azimuth ]
    }
}

/// Positions are in the mirrored space, see `MIRROR`.
#[ derive( Clone ) ]
pub enum Camera
{
    Orbit( Orbit ),
    Fly( Fly ),
}

impl Camera
{
    /// Looking at the whole model from outside.
    pub fn framing( model: &Model ) -> Self
    {
        let [ x, y, z ] = model.center;
        let distance = model.radius / ( FOV_Y / 2.0 ).sin();
        let orbit    = Orbit { target: [ x, y, -z ], yaw: 0.6, pitch: 0.35, distance, radius: model.radius };

        Self::Orbit( orbit )
    }

    /// Standing at a game position, looking along a game direction.
    pub fn standing( position: [f32; 3], direction: [f32; 3] ) -> Self
    {
        let [ x, y, z ]    = position;
        let [ dx, dy, dz ] = normalize( [ direction[ 0 ], direction[ 1 ], -direction[ 2 ] ] );
        let fly            = Fly { position: [ x, y, -z ], yaw: ( -dx ).atan2( -dz ), pitch: dy.asin(), speed: FLY_SPEED };

        Self::Fly( fly )
    }

    fn eye( &self ) -> [f32; 3]
    {
        match self
        {
            Self::Orbit( orbit ) => orbit.eye(),
            Self::Fly( fly ) => fly.position,
        }
    }

    /// Game coordinates of the free camera.
    fn position( &self ) -> Option<[f32; 3]>
    {
        let Self::Fly( fly ) = self else { return None };
        let [ x, y, z ] = fly.position;

        Some( [ x, y, -z ] )
    }

    /// Right, up and forward of the view.
    fn basis( &self ) -> ([f32; 3], [f32; 3], [f32; 3])
    {
        let forward = match self
        {
            Self::Orbit( orbit ) => normalize( sub( orbit.target, orbit.eye() ) ),
            Self::Fly( fly ) => fly.forward(),
        };
        let side = normalize( cross( forward, [ 0.0, 1.0, 0.0 ] ) );
        let up   = cross( side, forward );

        ( side, up, forward )
    }

    /// `far` clips what lies past it; `None` keeps the far plane at infinity.
    fn view_proj( &self, aspect: f32, far: Option<f32> ) -> [[f32; 4]; 4]
    {
        let eye = self.eye();
        let ( target, near ) = match self
        {
            Self::Orbit( orbit ) => ( orbit.target, ( orbit.distance - ( orbit.radius * 2.0 ) ).max( orbit.radius * 0.01 ) ),
            Self::Fly( fly ) => ( add( fly.position, fly.forward() ), FLY_NEAR ),
        };
        let view = look_at( eye, target, [ 0.0, 1.0, 0.0 ] );

        mul( perspective( FOV_Y, aspect, near, far ), mul( view, MIRROR ) )
    }
}

#[ derive( Clone ) ]
pub struct Orbit
{
    target:   [f32; 3],
    yaw:      f32,
    pitch:    f32,
    distance: f32,

    /// Model radius: bounds the zoom and the depth range.
    radius: f32,
}

impl Orbit
{
    fn eye( &self ) -> [f32; 3]
    {
        let ( sin_yaw, cos_yaw ) = self.yaw.sin_cos();
        let ( sin_pitch, cos_pitch ) = self.pitch.sin_cos();
        let offset = [ cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw ];

        [ 0, 1, 2 ].map( |axis| self.target[ axis ] + ( offset[ axis ] * self.distance ) )
    }

    fn pan( &mut self, delta: Vec2, height: f32 )
    {
        let eye     = self.eye();
        let forward = normalize( sub( self.target, eye ) );
        let right   = normalize( cross( forward, [ 0.0, 1.0, 0.0 ] ) );
        let up      = cross( right, forward );
        let scale   = ( ( 2.0 * self.distance ) * ( FOV_Y / 2.0 ).tan() ) / height.max( 1.0 );

        for axis in 0..3
        {
            self.target[ axis ] += ( ( -delta.x * right[ axis ] ) + ( delta.y * up[ axis ] ) ) * scale;
        }
    }

    /// Returns whether the view changed.
    fn on_input( &mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect ) -> bool
    {
        let mut changed = false;

        if delta != Vec2::ZERO
        {
            if response.dragged_by( egui::PointerButton::Primary )
            {
                self.yaw -= delta.x * ORBIT_SPEED;
                self.pitch = ( self.pitch + ( delta.y * ORBIT_SPEED ) ).clamp( -PITCH_LIMIT, PITCH_LIMIT );
            }
            else
            {
                self.pan( delta, rect.height() );
            }
            changed = true;
        }

        let scroll = ui.input( |input| input.smooth_scroll_delta.y );
        if response.hovered() && scroll != 0.0
        {
            self.distance = ( self.distance * ( -scroll * ZOOM_SPEED ).exp() ).max( self.radius * 0.05 );
            changed = true;
        }

        changed
    }
}

/// Free camera: drag to look, WASD to move, Q and E down and up, Shift faster, the wheel sets the speed.
#[ derive( Clone ) ]
pub struct Fly
{
    position: [f32; 3],

    /// Zero looks along -z, positive turns left.
    yaw: f32,

    pitch: f32,

    /// World units per second.
    speed: f32,
}

impl Fly
{
    fn forward( &self ) -> [f32; 3]
    {
        let ( sin_yaw, cos_yaw ) = self.yaw.sin_cos();
        let ( sin_pitch, cos_pitch ) = self.pitch.sin_cos();

        [ -sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch ]
    }

    fn right( &self ) -> [f32; 3]
    {
        let ( sin_yaw, cos_yaw ) = self.yaw.sin_cos();

        [ cos_yaw, 0.0, -sin_yaw ]
    }

    /// Returns whether the view changed.
    fn on_input( &mut self, ui: &egui::Ui, response: &egui::Response ) -> bool
    {
        let mut changed = false;

        if delta != Vec2::ZERO
        {
            self.yaw -= delta.x * LOOK_SPEED;
            self.pitch = ( self.pitch - ( delta.y * LOOK_SPEED ) ).clamp( -PITCH_LIMIT, PITCH_LIMIT );
            changed = true;
        }

        if !response.hovered() || ui.ctx().egui_wants_keyboard_input()
        {
            return changed;
        }

        let scroll = ui.input( |input| input.smooth_scroll_delta.y );
        if scroll != 0.0
        {
            self.speed = ( self.speed * ( scroll * ZOOM_SPEED ).exp() ).clamp( 10.0, 100_000.0 );
        }

        let ( forward, right ) = ( self.forward(), self.right() );

        let moves =
        [
            ( Key::W, forward ),
            ( Key::S, forward.map( |value| -value ) ),
            ( Key::D, right ),
            ( Key::A, right.map( |value| -value ) ),
            ( Key::E, [ 0.0, 1.0, 0.0 ] ),
            ( Key::Q, [ 0.0, -1.0, 0.0 ] ),
        ];
        let ( step, boost, dt ) = ui.input(
            |input|
            {
                let step = moves.iter().filter( |( key, _ )| input.key_down( *key ) ).fold( [ 0.0; 3 ], |sum, ( _, direction )| add( sum, *direction ) );
                let boost = if input.modifiers.shift { FLY_BOOST } else { 1.0 };
                ( step, boost, input.stable_dt.min( 0.1 ) )
            }
        );
        if step != [ 0.0; 3 ]
        {
            let distance = ( self.speed * boost ) * dt;
            self.position = add( self.position, normalize( step ).map( |value| value * distance ) );
            ui.ctx().request_repaint();
            changed = true;
        }

        changed
    }
}

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
struct PathView
{
    cells: bool,
    links: bool,
}

struct GpuPaths
{
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    indices:  wgpu::Buffer,
    cells:    std::ops::Range<u32>,
    links:    std::ops::Range<u32>,
}

/// A light source of a scene, drawn as a marker over the view.
pub struct SceneLight
{
    /// Actor the light belongs to; lights of a group share it.
    pub name:     String,
    pub id:       u32,
    pub position: [f32; 3],
    pub color:    Color32,
    pub radius:   f32,
}

/// Time between drawn frames, smoothed; frames drawn far apart, after the view stood still, are left out.
#[ derive( Default ) ]
struct FrameTime
{
    last:    Option<Instant>,
    seconds: Option<f32>,
}

impl FrameTime
{
    fn on_frame( &mut self )
    {
        let now = Instant::now();

        if let Some( gap ) = self.last.map( |last| now.duration_since( last ).as_secs_f32() ).filter( |&gap| gap < FRAME_GAP )
        {
            let seconds = self.seconds.map_or( gap, |seconds| seconds + ( ( gap - seconds ) * FRAME_SMOOTHING ) );
            self.seconds = Some( seconds );
        }
        self.last = Some( now );
    }
}

/// Totals for the footer; the model itself is dropped once uploaded.
struct Stats
{
    kind:      &'static str,
    surfaces:  usize,
    vertices:  usize,
    triangles: usize,
}

pub struct DecodedTexture
{
    width:  u32,
    height: u32,
    mips:   Vec<Vec<u8>>,
}

struct Target
{
    size:    [u32; 2],
    msaa:    wgpu::TextureView,
    resolve: wgpu::TextureView,
    depth:   wgpu::TextureView,
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

/// Planar reflection of the scene in its water: the frame drawn once more from under the water level,
/// with the view mirrored in it and everything below it cut away.
/// One level is mirrored at a time, the one whose water is nearest under the eye; see [`Water::level`].
struct Water
{
    /// The water parts over the reflection.
    surface: wgpu::RenderPipeline,

/// The reflection's frame and the bind group the water reads it through.
struct Reflection
{
    target: Target,
    group:  wgpu::BindGroup,
}

/// How a pass draws the water parts.
#[ derive( Clone, Copy ) ]
enum WaterPass<'a>
{
    /// As any other part, in the untextured modes.
    Plain,

    /// Over the reflection: the pipeline and the reflection's bind group.
    Mirror( &'a wgpu::RenderPipeline, &'a wgpu::BindGroup ),

    /// Left out, from the reflection itself.
    Hidden,
}

/// What every camera of a frame draws alike: the pipeline, the lightmap overlays, whether two-phase, the background.
#[ derive( Clone, Copy ) ]
struct Look<'a>
{
    pass:      Pass,
    lights:    Option<&'a HashSet<u32>>,
    two_phase: bool,
    clear:     wgpu::Color,

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

struct Gpu
{
    state:       RenderState,
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
    culling:     Culling,

    /// Culling of the camera; the reflection has its own, see [`Water`].
    camera: CullView,

    water:       Option<Water>,
    lightmapped: bool,
    paths:       Option<GpuPaths>,
    target:      Option<Target>,
    texture_id:  Option<TextureId>,

}

pub struct ModelViewer
{
    gpu: Gpu,

    /// The frame under the floating plates.
    blur: Blur,

    /// A character's animations.
    player: Option<Player>,

    stats:   Stats,
    camera:  Camera,
    mode:    RenderMode,
    options: RenderOptions,

    /// What the render menu resets to; a changed menu is highlighted.
    defaults: RenderOptions,

    light:   Light,
    missing: Vec<String>,

    /// API the device lacks, see [`culling::unsupported`].
    unsupported: Vec<&'static str>,

    dirty: bool,

    /// Where a double click puts the camera back.
    home: Camera,

    /// When the fly controls hint goes away; set on the first fly frame.
    hint_until: Option<f64>,

    lights:      Vec<SceneLight>,
    show_lights: bool,
    paths:       PathView,

    /// Lights whose lightmaps are left out, by actor id; a click on a marker switches one.
    switched_off: HashSet<u32>,

    /// Lighting of a scene without sun, in place of it; see [`ModelViewer::with_ambient`].
    ambient: Option<[f32; 3]>,

    fog: Fog,
}

impl ModelViewer
{
    pub fn new( state: &RenderState, model: &Model, find_texture: FindTexture ) -> Self
    {
        let ( textures, missing ) = decode_textures( model, find_texture, None );

        Self::from_decoded( state, model, &textures, missing, Camera::framing( model ) )
    }

    pub fn from_decoded( state: &RenderState, model: &Model, textures: &Textures, missing: Vec<String>, camera: Camera ) -> Self
    {
        let gpu         = Gpu::new( state, model, textures );
        let blur        = Blur::new( state, COLOR_FORMAT );
        let player      = model.skeleton.clone().map( |skeleton| Player::new( skeleton, model.vertices.clone() ) );
        let unsupported = culling::unsupported( &state.adapter, &state.device );
        let stats       = Stats { kind: model.kind, surfaces: model.surfaces, vertices: model.vertices.len(), triangles: model.triangle_count() };

        Self { gpu, stats, home: camera.clone(), camera, mode: RenderMode::Shaded, options: RenderOptions::DEFAULT, defaults: RenderOptions::DEFAULT, light: Light::DEFAULT, missing, dirty: true, hint_until: None, lights: Vec::new(), show_lights: true, paths: PathView { cells: false, links: false }, switched_off: HashSet::new(), ambient: None, fog: Fog::DEFAULT }
    }

    /// Light markers, and the lights' colors for their lightmaps.
    pub fn with_lights( mut self, lights: Vec<SceneLight> ) -> Self
    {
        let colors: HashMap<u32, Color32> = lights.iter().map( |light| ( light.id, light.color ) ).collect();
        self.gpu.set_light_colors( &colors );
        self.lights = lights;
        self.dirty = true;

        self
    }

    pub fn with_lightmaps( mut self ) -> Self
    {
        self.options.lightmaps = true;
        self.defaults = self.options;
        self.dirty = true;

        self
    }

    /// For a whole level, where most of it is out of sight.
    pub fn with_occlusion( mut self ) -> Self
    {
        self.options.occlusion = true;
        self.defaults = self.options;
        self.dirty = true;

        self
    }

    /// The scene's `<Ambient>`, 0 to 255 per channel, as the base lighting instead of the sun.
    /// Doubled like the lightmaps, as the game's final `MODULATE2X` stage does.
    pub fn with_ambient( mut self, ambient: Option<[f32; 3]> ) -> Self
    {
        self.ambient = ambient.map( |rgb| rgb.map( |channel| ( channel / 255.0 ) * 2.0 ) );
        self.level   = true;
        // Outdoor scenes fade into haze; interiors are closed rooms.
        self.options.fog = ambient.is_none();
        self.defaults = self.options;
        self.dirty = true;

        self
    }

    pub fn with_paths( mut self, lines: Option<&PathLines> ) -> Self
    {
        if let Some( lines ) = lines
        {
            self.gpu.set_paths( lines );
        }

        self
    }

    /// `shown` picks the overlays: frame time, statistics, coordinates and the axis gizmo.
    pub fn show( &mut self, sections: &mut Sections, palette: &Palette, shown: &ViewerSettings )
    {
        // Test: the plates stay dark over the blurred view in either theme.
        let plate   = &theme::DARK;
        let changed = self.options != self.defaults;
        let render  = widgets::icon_toggle( &mut sections.action_left, plate, icons::RENDER, changed ).on_hover_text( t!( "model.render" ) );
        egui::Popup::menu( &render ).close_behavior( egui::PopupCloseBehavior::CloseOnClickOutside ).show(
            |ui|
            {
                self.dirty |= ui.checkbox( &mut self.options.no_cull, t!( "model.no_cull" ) ).changed();
                if self.gpu.culling.cluster_count() > 1
                {
                    self.dirty |= ui.checkbox( &mut self.options.occlusion, t!( "model.occlusion" ) ).changed();
                }
                if self.gpu.lightmapped
                {
                    self.dirty |= ui.checkbox( &mut self.options.lightmaps, t!( "model.lightmaps" ) ).changed();
                }
                if !self.lights.is_empty()
                {
                    ui.checkbox( &mut self.show_lights, t!( "model.light_sources" ) );
                }
                if self.gpu.paths.is_some()
                {
                    self.dirty |= ui.checkbox( &mut self.paths.cells, t!( "model.path_cells" ) ).changed();
                    self.dirty |= ui.checkbox( &mut self.paths.links, t!( "model.path_links" ) ).changed();
                }
                self.dirty |= ui.checkbox( &mut self.options.fog, t!( "model.fog" ) ).changed();
                if self.options.fog
                {
                    self.dirty |= fog_settings( ui, palette, &mut self.fog );
                }
            }
        );

        // Only the shaded mode is lit.
        if self.mode == RenderMode::Shaded
        {
            let light = widgets::icon_toggle( &mut sections.action_left, plate, icons::LIGHT, self.light.enabled ).on_hover_text( t!( "model.lighting" ) );
            egui::Popup::menu( &light ).close_behavior( egui::PopupCloseBehavior::CloseOnClickOutside ).show(
                |ui|
                {
                    self.dirty |= ui.checkbox( &mut self.light.enabled, t!( "model.lighting" ) ).changed();
                    egui::Grid::new( "light" ).show(
                        ui,
                        |ui|
                        {
                            self.dirty |= angle( ui, palette, &t!( "model.azimuth" ), &mut self.light.azimuth, ( 0.0, 360.0 ) );
                            self.dirty |= angle( ui, palette, &t!( "model.elevation" ), &mut self.light.elevation, ( -90.0, 90.0 ) );
                        }
                    );
                }
            );
        }
        for mode in RenderMode::ALL.into_iter().rev()
        {
            let response = widgets::icon_toggle( &mut sections.action_right, plate, mode.icon(), self.mode == mode ).on_hover_text( t!( mode.key() ) );
            if response.clicked() && self.mode != mode
            {
                self.mode = mode;
                self.dirty = true;
            }
        }

        if let Some( player ) = &mut self.player
        {
            if let Some( timeline ) = &mut sections.timeline
            {
                player.show( timeline, plate );
            }
            if player.playing()
            {
                player.advance( sections.view.input( |input| input.stable_dt ) );
                sections.view.ctx().request_repaint();
            }
        }

        let view = &mut sections.view;
        let ( rect, response ) = view.allocate_exact_size( view.available_size(), Sense::click_and_drag() );
        let light = if self.show_lights { light_under( rect, &self.camera, &self.lights, response.hover_pos() ) } else { None };
        if let Some( id ) = light.filter( |_| response.clicked() ).and_then( |index| self.lights.get( index ) ).map( |light| light.id )
        {
            self.on_switch_light( id );
        }
        self.on_input( view, &response, rect, light.is_some() );
        let pixels_per_point = view.pixels_per_point();
        let size             = [ physical( rect.width(), pixels_per_point ), physical( rect.height(), pixels_per_point ) ];
        let background = if self.options.fog { self.fog.color } else { palette.view };
        if let Some( id ) = self.render( size, background )
        {
            let uv = Rect::from_min_max( Pos2::ZERO, Pos2::new( 1.0, 1.0 ) );
            view.painter().image( id, rect, uv, Color32::WHITE );
        }
        if self.show_lights
        {
            paint_lights( view, palette, rect, &self.camera, &self.lights, &self.switched_off, light );
        }

        // Read even when hidden, so the readback keeps going.
        let ( visible, pending ) = self.gpu.camera.visible( &self.gpu.state.device );
        let clusters = self.gpu.culling.cluster_count();

        if pending
        {
            sections.view.ctx().request_repaint();
        }

        let mut segments = Vec::new();

        if shown.show_stats
        {
            segments.extend(
                [
                    t!( self.stats.kind ).into_owned(),
                    i18n::count( "model.surfaces", self.stats.surfaces ),
                    i18n::count( "model.vertices", self.stats.vertices ),
                    i18n::count( "model.triangles", self.stats.triangles ),
                ]
            );
        }
        if let Some( visible ) = visible.filter( |_| shown.show_stats && clusters > 1 )
        {
            segments.push( t!( "model.visible_clusters", visible = visible.clusters, total = clusters ).into_owned() );
            segments.push( t!( "model.drawn_triangles", count = visible.triangles ).into_owned() );
        }

        if !self.unsupported.is_empty()
        {
            segments.push( t!( "model.unsupported", list = self.unsupported.join( ", " ) ).into_owned() );
        }
        if !self.missing.is_empty()
        {
            segments.push( i18n::count( "model.missing", self.missing.len() ) );
        }
        sections.footer_segments( plate, &segments );
        if !self.missing.is_empty()
        {
            sections.footer_left.label( egui::RichText::new( "?" ).color( plate.menu_text ) ).on_hover_text( self.missing.join( "\n" ) );
        }
        if let Some( [ x, y, z ] ) = self.camera.position().filter( |_| shown.show_coordinates )
        {
            let position = [ format!( "x {x:.0}" ), format!( "y {y:.0}" ), format!( "z {z:.0}" ) ];
            sections.footer_right_segments( plate, &position );
        }
        if let Camera::Fly( _ ) = self.camera
        {
            let now   = sections.view.input( |input| input.time );
            let until = *self.hint_until.get_or_insert( now + HINT_SECONDS );
            let left  = until - now;
            if left > 0.0
            {
                let opacity = ( left / HINT_FADE ).min( 1.0 ) as f32;
                sections.hint( plate, &t!( "model.fly_controls" ), opacity );
                sections.view.ctx().request_repaint();
            }
        }

        let corner = sections.corner( PIVOT_PLATE );
        paint_pivot( sections.view.painter(), corner.center(), &self.camera );
    }

    fn on_switch_light( &mut self, id: u32 )
    {
        if !self.switched_off.remove( &id )
        {
            self.switched_off.insert( id );
        }
        self.dirty = true;
    }

    /// A left double click puts the camera back, unless it lands on a light marker.
    fn on_input( &mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect, over_light: bool )
    {
        if response.double_clicked_by( egui::PointerButton::Primary ) && !over_light
        {
            self.camera = self.home.clone();
            self.dirty = true;
            return;
        }

        self.dirty |= match &mut self.camera
        {
            Camera::Orbit( orbit ) => orbit.on_input( ui, response, rect ),
            Camera::Fly( fly ) => fly.on_input( ui, response ),
        };
    }

    fn render( &mut self, size: [u32; 2], background: Color32 ) -> Option<TextureId>
    {
        if size[ 0 ] == 0 || size[ 1 ] == 0
        {
            return None;
        }
        let resized = self.gpu.target.as_ref().is_none_or( |target| target.size != size );
        if resized
        {
            self.gpu.resize( size );
            if let Some( target ) = &self.gpu.target
            {
                self.blur.resize( &target.resolve, size );
            }
            self.dirty = true;
        }
        if let Some( vertices ) = self.player.as_mut().and_then( Player::pose )
        {
            self.gpu.set_vertices( vertices );
            self.dirty = true;
        }
        if std::mem::take( &mut self.dirty )
        {
            let fog      = self.options.fog.then_some( self.fog );
            let uniforms = frame_uniforms( &self.camera, self.mode, self.light, self.options, self.ambient, fog, size );
            let pass = match ( self.mode, self.options.no_cull )
            {
                ( RenderMode::Wireframe, _ ) => Pass::Wireframe,
                ( _, true ) => Pass::TwoSided,
                ( _, false ) => Pass::FrontFaces,
            };
            let textured = matches!( self.mode, RenderMode::Material | RenderMode::Shaded );
            let lights   = ( self.options.lightmaps && textured ).then_some( &self.switched_off );
            self.gpu.draw( &uniforms, pass, self.paths, lights, background );
        }

        self.gpu.texture_id
    }
}

/// A model rendered once for the thumbnail grid; only the resulting texture is kept.
pub struct ModelThumbnail
{
    state: RenderState,
    id:    TextureId,
}

impl ModelThumbnail
{
    pub fn id( &self ) -> TextureId
    {
        self.id
    }
}

impl Drop for ModelThumbnail
{
    fn drop( &mut self )
    {
        self.state.renderer.write().free_texture( &self.id );
    }
}

pub fn thumbnail( state: &RenderState, data: &[u8], find_texture: FindTexture, size: u32 ) -> Option<ModelThumbnail>
{
    let model = Model::parse( data ).ok()?;

    let ( textures, _ ) = decode_textures( &model, find_texture, Some( size ) );

    let mut gpu = Gpu::new( state, &model, &textures );
    gpu.resize( [ size, size ] );
    let uniforms = frame_uniforms( &Camera::framing( &model ), RenderMode::Shaded, Light::DEFAULT, RenderOptions::DEFAULT, None, None, [ size, size ] );
    gpu.draw( &uniforms, Pass::TwoSided, PathView { cells: false, links: false }, None, false, Color32::TRANSPARENT, false, false );

    // Taken out so dropping the buffers keeps the registered texture alive.
    let id        = gpu.texture_id.take()?;
    let thumbnail = ModelThumbnail { state: state.clone(), id };

    Some( thumbnail )
}

/// Every texture the model's parts name, and the names not found; `limit` skips mips wider than it.
pub fn decode_textures( model: &Model, find_texture: FindTexture, limit: Option<u32> ) -> (Textures, Vec<String>)
{
    let mut textures = Textures::new();
    let mut missing  = Vec::new();

    for name in model.parts.iter().filter_map( |part| part.texture.as_deref() )
    {
        let key = name.to_lowercase();
        if textures.contains_key( &key ) || missing.contains( &key )
        {
            continue;
        }
        match decode_texture( name, find_texture, limit )
        {
            Some( texture ) =>
            {
                textures.insert( key, texture );
            }
            None => missing.push( key ),
        }
    }

    ( textures, missing )
}

/// The light whose marker is under the pointer; the topmost, as the markers are drawn in order.
fn light_under( rect: Rect, camera: &Camera, lights: &[SceneLight], pointer: Option<Pos2> ) -> Option<usize>
{
    let pointer = pointer?;
    let matrix  = camera.view_proj( rect.width() / rect.height(), None );

    lights.iter().rposition( |light| project( matrix, light.position, rect ).is_some_and( |( position, _ )| pointer.distance( position ) <= LIGHT_PICK ) )
}

/// Light markers over the view; the hovered one also shows its range and details.
fn paint_lights( ui: &egui::Ui, palette: &Palette, rect: Rect, camera: &Camera, lights: &[SceneLight], switched_off: &HashSet<u32>, hovered: Option<usize> )
{
    let matrix  = camera.view_proj( rect.width() / rect.height(), None );
    let painter = ui.painter_at( rect );

    for light in lights
    {
        let Some( ( position, _ ) ) = project( matrix, light.position, rect ) else { continue };
        // A switched off light keeps only its outline.
        let fill = if switched_off.contains( &light.id ) { Color32::TRANSPARENT } else { light.color };
        painter.circle( position, LIGHT_MARKER, fill, Stroke::new( 1.0, if fill == Color32::TRANSPARENT { light.color } else { palette.view } ) );
    }

    let Some( light ) = hovered.and_then( |index| lights.get( index ) ) else { return };
    let Some( ( position, depth ) ) = project( matrix, light.position, rect ) else { return };
    let focal = 1.0 / ( FOV_Y / 2.0 ).tan();
    let range = ( ( light.radius * focal ) * ( rect.height() / 2.0 ) ) / depth;
    painter.circle_stroke( position, range, Stroke::new( 1.0, light.color.gamma_multiply( 0.6 ) ) );

    let [ red, green, blue, _ ] = light.color.to_array();
    let [ x, y, z ]             = light.position;
    let color    = format!( "#{red:02X}{green:02X}{blue:02X}" );
    let radius   = format!( "{:.0}", light.radius );
    let location = format!( "{x:.0}, {y:.0}, {z:.0}" );
    let text     = t!( "model.light_tip", name = light.name, color = color, radius = radius, position = location );
    let galley = painter.layout_no_wrap( text.into_owned(), egui::FontId::proportional( crate::theme::TEXT_SIZE ), palette.text );
    let label  = Rect::from_min_size( position + Vec2::new( LIGHT_PICK, LIGHT_PICK ), galley.size() );
    painter.rect( label.expand( 4.0 ), egui::CornerRadius::same( crate::theme::CONTROL_RADIUS ), palette.window, Stroke::new( 1.0, palette.border ), egui::StrokeKind::Outside );
    painter.galley( label.min, galley, palette.text );
}

/// Screen position and view depth of a game point, if it is in front of the camera and inside `rect`.
fn project( matrix: [[f32; 4]; 4], point: [f32; 3], rect: Rect ) -> Option<(Pos2, f32)>
{
    let clip  = [ 0, 1, 2, 3 ].map( |row| ( matrix[ 0 ][ row ] * point[ 0 ] ) + ( matrix[ 1 ][ row ] * point[ 1 ] ) + ( matrix[ 2 ][ row ] * point[ 2 ] ) + matrix[ 3 ][ row ] );
    let depth = clip[ 3 ];
    if depth <= 0.0
    {
        return None;
    }

    let x        = rect.center().x + ( ( clip[ 0 ] / depth ) * ( rect.width() / 2.0 ) );
    let y        = rect.center().y - ( ( clip[ 1 ] / depth ) * ( rect.height() / 2.0 ) );
    let position = Pos2::new( x, y );

    rect.contains( position ).then_some( ( position, depth ) )
}

/// Game axes as the camera sees them, the nearest drawn last.
fn paint_pivot( painter: &egui::Painter, center: Pos2, camera: &Camera )
{
    let ( side, up, forward ) = camera.basis();

    let mut axes = PIVOT_AXES.map( |( axis, color )| ( Vec2::new( dot( side, axis ), -dot( up, axis ) ), -dot( forward, axis ), color ) );
    axes.sort_by( |a, b| a.1.total_cmp( &b.1 ) );

    for ( direction, _, color ) in axes
    {
        painter.line_segment( [ center, center + ( direction * PIVOT_LENGTH ) ], Stroke::new( PIVOT_STROKE, color ) );
    }
}

/// Grid row: label, slider, value in whole degrees.
/// Fog start, end and color; the end stays past the start.
fn fog_settings( ui: &mut egui::Ui, palette: &Palette, fog: &mut Fog ) -> bool
{
    let before = *fog;

    egui::Grid::new( "fog" ).show(
        ui,
        |ui|
        {
            distance( ui, palette, &t!( "model.fog_start" ), &mut fog.start );
            distance( ui, palette, &t!( "model.fog_end" ), &mut fog.end );
            ui.label( t!( "model.fog_color" ) );
            egui::color_picker::color_edit_button_srgba( ui, &mut fog.color, egui::color_picker::Alpha::Opaque );
            ui.end_row();
        }
    );
    fog.end = fog.end.max( fog.start + FOG_MIN_SPAN );

    *fog != before
}

/// Grid row: label, slider, value in world units.
fn distance( ui: &mut egui::Ui, palette: &Palette, label: &str, value: &mut f32 )
{
    ui.label( label );
    if let Some( distance ) = widgets::slider( ui, palette, *value, ( 0.0, FOG_MAX ) )
    {
        *value = ( distance / 100.0 ).round() * 100.0;
    }
    ui.label( format!( "{value:.0}" ) );
    ui.end_row();
}

fn angle( ui: &mut egui::Ui, palette: &Palette, label: &str, value: &mut f32, range: (f32, f32) ) -> bool
{
    ui.label( label );
    let result = widgets::slider( ui, palette, *value, range );
    if let Some( angle ) = result
    {
        *value = angle.round();
    }
    ui.label( format!( "{value:.0}°" ) );
    ui.end_row();

    result.is_some()
}

fn frame_uniforms( camera: &Camera, mode: RenderMode, light: Light, options: RenderOptions, ambient: Option<[f32; 3]>, fog: Option<Fog>, size: [u32; 2] ) -> FrameUniforms
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

/// The frame mirrored in the water level at `level`: the view flipped under the plane, the eye with it,
/// and everything below the plane cut away.
fn reflected( uniforms: &FrameUniforms, level: f32 ) -> FrameUniforms
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
            let start = crate::model::to_u32( ranges[ chunk ].start );
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
    fn new( state: &RenderState, model: &Model, textures: &Textures ) -> Self
    {
        let device = &state.device;
        let shader_source = wgpu::ShaderModuleDescriptor
        {
            label:  Some( "model" ),
            source: wgpu::ShaderSource::Wgsl( include_str!( "model.wgsl" ).into() ),
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
            let values   = [ part.color, [ 1.0; 4 ] ];
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
        };

        gpu
    }

    /// New vertices for a model of one chunk, as a character is.
    fn set_vertices( &self, vertices: &[Vertex] )
    {
        let Some( buffer ) = self.vertices.first() else { return };
        self.state.queue.write_buffer( buffer, 0, bytemuck::cast_slice( vertices ) );
    }

    fn set_light_colors( &self, colors: &HashMap<u32, Color32> )
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
    fn set_paths( &mut self, lines: &PathLines )
    {
        let device = &self.state.device;
        let shader_source = wgpu::ShaderModuleDescriptor
        {
            label:  Some( "paths" ),
            source: wgpu::ShaderSource::Wgsl( include_str!( "model.wgsl" ).into() ),
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
        let cells_end         = crate::model::to_u32( lines.cells.len() );
        let links_end         = crate::model::to_u32( indices.len() );

        let buffer = |label: &str, contents: &[u8], usage: wgpu::BufferUsages|
        {
            device.create_buffer_init( &wgpu::util::BufferInitDescriptor { label: Some( label ), contents, usage } )
        };
        let paths = GpuPaths
        {
            pipeline,
            vertices: buffer( "path vertices", bytemuck::cast_slice( &lines.vertices ), wgpu::BufferUsages::VERTEX ),
            indices:  buffer( "path indices", bytemuck::cast_slice( &non_empty( &indices ) ), wgpu::BufferUsages::INDEX ),
            cells:    0..cells_end,
            links:    cells_end..links_end,
        };
        self.paths = Some( paths );
    }

    fn resize( &mut self, size: [u32; 2] )
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

    /// A reflection for frames of `size`, smaller by [`REFLECTION_DIVISOR`].
    fn resize( &mut self, device: &wgpu::Device, culling: &Culling, size: [u32; 2] )
    {
        let size   = size.map( |side| ( side / REFLECTION_DIVISOR ).max( 1 ) );
        let target = Target::new( device, size );
        culling.resize( device, &mut self.culling, &target.depth, size );

        let group      = self.group( device, &target.resolve );
        let reflection = Reflection { target, group };
        self.reflection = Some( reflection );
    }
}

impl Target
{
    /// Multisampled color and depth of `size`, and the color resolved for reading.
    fn new( device: &wgpu::Device, size: [u32; 2] ) -> Self
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

/// Front faces wind clockwise, as in the game's Direct3D, and stay so through the mirrored view.
/// A pass over the frame: `clear` starts it anew, otherwise it goes on from the previous pass; the `last` one resolves.
fn begin_pass<'a>( encoder: &'a mut wgpu::CommandEncoder, target: &Target, clear: Option<wgpu::Color>, last: bool ) -> wgpu::RenderPass<'a>
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

/// A blended pipeline draws over what is there: it keeps the depth and passes on equal depth.
fn pipeline( device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, fragment: &str, topology: wgpu::PrimitiveTopology, cull_mode: Option<wgpu::Face>, blend: Option<wgpu::BlendState> ) -> wgpu::RenderPipeline
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

fn uniform_entry( binding: u32, visibility: wgpu::ShaderStages ) -> wgpu::BindGroupLayoutEntry
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

fn decode_texture( name: &str, find_texture: FindTexture, limit: Option<u32> ) -> Option<DecodedTexture>
{
    let ( path, data ) = find_texture( name )?;
    if crate::preview::image_format( &path ).is_some()
    {
        let image   = crate::preview::decode_image( &path, &data ).ok()?;
        let decoded = DecodedTexture { width: image.width, height: image.height, mips: vec![ image.rgba ] };
        return Some( decoded );
    }

    let texture = mortex::Texture::parse( &data ).ok()?;
    let last    = texture.mip_count().saturating_sub( 1 );
    let first   = limit.map_or( 0, |limit| ( 0..last ).find( |&mip| ( texture.width() >> mip ) <= limit ).unwrap_or( last ) );
    let mips: Vec<Vec<u8>> = ( first..texture.mip_count() ).map_while( |mip| texture.decode( 0, mip ).ok().map( |image| image.rgba ) ).collect();
    if mips.is_empty()
    {
        return None;
    }
    let ( width, height ) = ( ( texture.width() >> first ).max( 1 ), ( texture.height() >> first ).max( 1 ) );

    Some( DecodedTexture { width, height, mips } )
}

fn upload_texture( state: &RenderState, decoded: &DecodedTexture ) -> wgpu::TextureView
{
    upload( state, decoded.width, decoded.height, &decoded.mips, COLOR_FORMAT )
}

/// `mips` from the full size down, tightly packed rows of `format` texels.
fn upload( state: &RenderState, width: u32, height: u32, mips: &[Vec<u8>], format: wgpu::TextureFormat ) -> wgpu::TextureView
{
    let texel = format.block_copy_size( None ).unwrap_or( 4 );

    let mip_count = u32::try_from( mips.len() ).unwrap_or( 1 );
    let descriptor = wgpu::TextureDescriptor
    {
        label:           Some( "model albedo" ),
        size:            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: mip_count,
        sample_count:    1,
        dimension:       wgpu::TextureDimension::D2,
        format,
        usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats:    &[],
    };
    let texture = state.device.create_texture( &descriptor );

    for ( mip, rgba ) in ( 0..mip_count ).zip( mips )
    {
        let ( mip_width, mip_height ) = ( ( width >> mip ).max( 1 ), ( height >> mip ).max( 1 ) );
        let destination = wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: mip, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All };
        let layout      = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some( mip_width * texel ), rows_per_image: Some( mip_height ) };
        let extent      = wgpu::Extent3d { width: mip_width, height: mip_height, depth_or_array_layers: 1 };
        state.queue.write_texture( destination, rgba, layout, extent );
    }

    texture.create_view( &wgpu::TextureViewDescriptor::default() )
}

/// wgpu rejects zero-sized buffers.
fn non_empty( indices: &[u32] ) -> Vec<u32>
{
    if indices.is_empty() { vec![ 0 ] } else { indices.to_vec() }
}

fn physical( points: f32, pixels_per_point: f32 ) -> u32
{
    let pixels = ( points * pixels_per_point ).round().max( 0.0 );

    widgets::from_f32( pixels ).try_into().unwrap_or( u32::MAX )
}

/// Reversed depth: 1 at `near`, falling to 0 at `far`, or far away without one, so precision holds across a whole city.
fn perspective( fov_y: f32, aspect: f32, near: f32, far: Option<f32> ) -> [[f32; 4]; 4]
{
    let focal = 1.0 / ( fov_y / 2.0 ).tan();

    // Depth near * ( far - d ) / ( d * ( far - near ) ) for the distance d along the view.
    let ( scale, offset ) = match far
    {
        Some( far ) =>
        {
            let far = far.max( near * 2.0 );
            ( near / ( far - near ), ( near * far ) / ( far - near ) )
        }
        None => ( 0.0, near ),
    };

    [ [ focal / aspect, 0.0, 0.0, 0.0 ], [ 0.0, focal, 0.0, 0.0 ], [ 0.0, 0.0, scale, -1.0 ], [ 0.0, 0.0, offset, 0.0 ] ]
}

fn look_at( eye: [f32; 3], target: [f32; 3], up: [f32; 3] ) -> [[f32; 4]; 4]
{
    let forward = normalize( sub( target, eye ) );
    let side    = normalize( cross( forward, up ) );
    let up      = cross( side, forward );

    let matrix =
    [
        [ side[ 0 ], up[ 0 ], -forward[ 0 ], 0.0 ],
        [ side[ 1 ], up[ 1 ], -forward[ 1 ], 0.0 ],
        [ side[ 2 ], up[ 2 ], -forward[ 2 ], 0.0 ],
        [ -dot( side, eye ), -dot( up, eye ), dot( forward, eye ), 1.0 ],
    ];

    matrix
}

/// Column-major product `a * b`.
fn mul( a: [[f32; 4]; 4], b: [[f32; 4]; 4] ) -> [[f32; 4]; 4]
{
    b.map( |column| [ 0, 1, 2, 3 ].map( |row| ( 0..4 ).map( |k| a[ k ][ row ] * column[ k ] ).sum() ) )
}

fn add( a: [f32; 3], b: [f32; 3] ) -> [f32; 3]
{
    [ a[ 0 ] + b[ 0 ], a[ 1 ] + b[ 1 ], a[ 2 ] + b[ 2 ] ]
}

fn sub( a: [f32; 3], b: [f32; 3] ) -> [f32; 3]
{
    [ a[ 0 ] - b[ 0 ], a[ 1 ] - b[ 1 ], a[ 2 ] - b[ 2 ] ]
}

fn dot( a: [f32; 3], b: [f32; 3] ) -> f32
{
    ( a[ 0 ] * b[ 0 ] ) + ( a[ 1 ] * b[ 1 ] ) + ( a[ 2 ] * b[ 2 ] )
}

fn cross( a: [f32; 3], b: [f32; 3] ) -> [f32; 3]
{
    [ ( a[ 1 ] * b[ 2 ] ) - ( a[ 2 ] * b[ 1 ] ), ( a[ 2 ] * b[ 0 ] ) - ( a[ 0 ] * b[ 2 ] ), ( a[ 0 ] * b[ 1 ] ) - ( a[ 1 ] * b[ 0 ] ) ]
}

fn normalize( v: [f32; 3] ) -> [f32; 3]
{
    let length = dot( v, v ).sqrt();

    if length > 0.0 { v.map( |value| value / length ) } else { v }
}
