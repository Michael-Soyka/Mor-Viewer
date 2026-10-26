pub mod camera;
mod controls;
mod frame;
mod overlay;

use std::collections::{ HashMap, HashSet };
use std::time::Instant;

use eframe::egui::{ self, Color32, Pos2, Rect, Sense, TextureId };
use eframe::egui_wgpu::RenderState;

use rust_i18n::t;
use crate::i18n;
use crate::render::animation::Player;
use crate::render::blur::Blur;
use crate::render::culling;
use crate::render::gpu::{ Gpu, Pass };
use crate::render::model::Model;
use crate::render::paths::{ PathLines, PathView };
use crate::render::scene::SceneLight;
use crate::render::target::COLOR_FORMAT;
use crate::render::texture::{ FindTexture, Textures, decode_textures };
use crate::render::weather::{ self, SkyImages, Weather };
use crate::settings::ViewerSettings;
use crate::ui::icons;
use crate::ui::theme::{ self, Palette };
use crate::ui::widgets;
use crate::viewer::Sections;

use camera::{ Camera, CursorCapture };
use controls::{ angle, fog_settings, time_settings };
use frame::{ Fog, Light, RenderMode, RenderOptions, TimeOfDay, frame_uniforms };
use overlay::{ PIVOT_PLATE, light_under, paint_lights, paint_pivot };

/// Fly controls hint: shown this long after entering fly mode, fading out at the end.
const HINT_SECONDS: f64 = 5.0;
const HINT_FADE:    f64 = 0.5;

/// Frame time: frames further apart than this follow a pause; share of each new frame in the average.
const FRAME_GAP:       f32 = 0.25;
const FRAME_SMOOTHING: f32 = 0.1;

/// Hour a level with the game's sky opens at.
const START_HOUR: f32 = 13.0;

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

    /// The street lights the world script switches with the hour, by actor id.
    street_lights: HashSet<u32>,

    /// Lighting of a scene without sun, in place of it; see [`ModelViewer::with_ambient`].
    ambient: Option<[f32; 3]>,

    /// A whole level; only it offers the fog, a single object has nothing far to hide.
    level: bool,

    /// Holds the cursor while a level is looked around.
    capture: CursorCapture,

    fog:        Fog,
    frame_time: FrameTime,

    /// The hour and weather of an outdoor level with the game's sky; they pick its sky, light and fog.
    time: Option<TimeOfDay>,
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

        Self { gpu, blur, player, stats, home: camera.clone(), camera, mode: RenderMode::Shaded, options: RenderOptions::DEFAULT, defaults: RenderOptions::DEFAULT, light: Light::DEFAULT, missing, dirty: true, hint_until: None, lights: Vec::new(), show_lights: true, paths: PathView { cells: false, links: false }, switched_off: HashSet::new(), street_lights: HashSet::new(), ambient: None, level: false, capture: CursorCapture::default(), fog: Fog::DEFAULT, frame_time: FrameTime::default(), time: None, unsupported }
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

    /// The game's sky over an outdoor level, at [`START_HOUR`] on a cloudy day.
    pub fn with_sky( mut self, images: Option<&SkyImages> ) -> Self
    {
        if let Some( images ) = images
        {
            self.gpu.set_sky( images );
            self.time = Some( TimeOfDay { hour: START_HOUR, weather: Weather::Cloudy, street_lights: true } );
            self.options.time_of_day = true;
            self.defaults = self.options;
            self.on_time_change();
        }

        self
    }

    /// `street` are the lights the world script switches with the hour; `off` start switched off, as the game has them.
    pub fn with_street_lights( mut self, street: HashSet<u32>, off: &HashSet<u32> ) -> Self
    {
        self.street_lights = street;
        self.switched_off.extend( off );
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
                if self.gpu.water.as_ref().is_some_and( |water| !water.levels.is_empty() )
                {
                    self.dirty |= ui.checkbox( &mut self.options.reflections, t!( "model.reflections" ) ).changed();
                }
                let mut changed = false;
                if let Some( time ) = &mut self.time
                {
                    changed |= ui.checkbox( &mut self.options.time_of_day, t!( "model.time_of_day" ) ).changed();
                    if self.options.time_of_day
                    {
                        changed |= time_settings( ui, palette, time );
                    }
                }
                if changed
                {
                    self.on_time_change();
                }
                if self.level
                {
                    self.dirty |= ui.checkbox( &mut self.options.fog, t!( "model.fog" ) ).changed();
                }
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
            paint_lights( view, palette, rect, &self.camera, &self.lights, &self.lights_out(), light );
        }
        if let Some( blurred ) = self.blur.id()
        {
            sections.set_backdrop( blurred, rect );
        }
        if let Some( seconds ) = self.frame_time.seconds.filter( |_| shown.show_fps )
        {
            let text = t!( "model.frame_time", fps = format!( "{:.0}", 1.0 / seconds ), ms = format!( "{:.1}", seconds * 1000.0 ) );
            sections.top_text( plate, &text );
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

        if shown.show_orientation
        {
            let corner = sections.corner( PIVOT_PLATE );
            paint_pivot( sections.view.painter(), corner.center(), &self.camera );
        }
    }

    /// The fog follows the hour, as the game's weather script sets it; without the time of day it is the viewer's own.
    fn on_time_change( &mut self )
    {
        self.dirty = true;
        let Some( time ) = self.time.filter( |_| self.options.time_of_day ) else
        {
            self.fog = Fog::DEFAULT;
            return;
        };
        let sky = weather::at( time.hour, time.weather );
        let [ red, green, blue ] = sky.fog.map( |channel| to_byte( channel * 255.0 ) );
        self.fog = Fog { start: weather::FOG_START, end: weather::FOG_END, color: Color32::from_rgb( red, green, blue ) };
    }

    /// Lights left out of the frame: those switched off by hand, and the street lights by day while they follow the hour.
    fn lights_out( &self ) -> HashSet<u32>
    {
        let by_day = self.time.is_some_and( |time| self.options.time_of_day && time.street_lights && !weather::street_lights_on( time.hour ) );
        let out    = if by_day { self.switched_off.union( &self.street_lights ).copied().collect() } else { self.switched_off.clone() };

        out
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

        let delta = if self.level { self.capture.on_drag( ui, response ) } else { response.drag_delta() };
        self.dirty |= match &mut self.camera
        {
            Camera::Orbit( orbit ) => orbit.on_input( ui, response, rect, delta ),
            Camera::Fly( fly ) => fly.on_input( ui, response, delta ),
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
            let fog          = self.options.fog.then_some( self.fog );
            let mut uniforms = frame_uniforms( &self.camera, self.mode, self.light, self.options, self.ambient, fog, size );

            if let Some( time ) = self.time.filter( |_| self.options.time_of_day )
            {
                let sky = weather::at( time.hour, time.weather );
                uniforms[ 28..31 ].copy_from_slice( &sky.ambient );
                uniforms[ 52..55 ].copy_from_slice( &sky.sun );
                uniforms[ 55 ] = 1.0;
                uniforms[ 56..60 ].copy_from_slice( &[ sky.from, sky.to, sky.blend, 1.0 ] );
            }
            let pass = match ( self.mode, self.options.no_cull )
            {
                ( RenderMode::Wireframe, _ ) => Pass::Wireframe,
                ( _, true ) => Pass::TwoSided,
                ( _, false ) => Pass::FrontFaces,
            };
            let textured = matches!( self.mode, RenderMode::Material | RenderMode::Shaded );
            let out      = self.lights_out();
            let lights   = ( self.options.lightmaps && textured ).then_some( &out );
            self.gpu.draw( &uniforms, pass, self.paths, lights, self.options.occlusion, background, textured, self.options.reflections );
            self.blur.run();
            self.frame_time.on_frame();
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

/// Rounded and held to 0..255; Rust has no `TryFrom<f32>` for integers.
fn to_byte( value: f32 ) -> u8
{
    value.round().clamp( 0.0, 255.0 ) as u8
}

fn physical( points: f32, pixels_per_point: f32 ) -> u32
{
    let pixels = ( points * pixels_per_point ).round().max( 0.0 );

    widgets::from_f32( pixels ).try_into().unwrap_or( u32::MAX )
}
