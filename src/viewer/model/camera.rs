use eframe::egui::{ self, Key, Pos2, Rect, Vec2 };

use crate::render::math::{ add, cross, look_at, mul, normalize, perspective, sub };
use crate::render::model::Model;

pub const FOV_Y: f32 = std::f32::consts::FRAC_PI_4;

const ORBIT_SPEED: f32 = 0.01;
const ZOOM_SPEED:  f32 = 0.0015;
const PITCH_LIMIT: f32 = 1.55;
const LOOK_SPEED:  f32 = 0.004;
const FLY_SPEED:   f32 = 800.0;
const FLY_BOOST:   f32 = 5.0;
const FLY_NEAR:    f32 = 5.0;

/// The game uses left-handed coordinates: flipping z turns them into right-handed ones.
const MIRROR: [[f32; 4]; 4] = [ [ 1.0, 0.0, 0.0, 0.0 ], [ 0.0, 1.0, 0.0, 0.0 ], [ 0.0, 0.0, -1.0, 0.0 ], [ 0.0, 0.0, 0.0, 1.0 ] ];

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

    pub fn eye( &self ) -> [f32; 3]
    {
        match self
        {
            Self::Orbit( orbit ) => orbit.eye(),
            Self::Fly( fly ) => fly.position,
        }
    }

    /// Game coordinates of the free camera.
    pub fn position( &self ) -> Option<[f32; 3]>
    {
        let Self::Fly( fly ) = self else { return None };
        let [ x, y, z ] = fly.position;

        Some( [ x, y, -z ] )
    }

    /// Right, up and forward of the view.
    pub fn basis( &self ) -> ([f32; 3], [f32; 3], [f32; 3])
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
    pub fn view_proj( &self, aspect: f32, far: Option<f32> ) -> [[f32; 4]; 4]
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

    /// `delta` is the drag's movement; returns whether the view changed.
    pub fn on_input( &mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect, delta: Vec2 ) -> bool
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

    /// `delta` is the drag's movement; returns whether the view changed.
    pub fn on_input( &mut self, ui: &egui::Ui, response: &egui::Response, delta: Vec2 ) -> bool
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

/// Hides the cursor and holds it where the drag began, so looking around never runs into the window edge;
/// the drag then moves by the raw mouse motion.
#[ derive( Default ) ]
pub struct CursorCapture
{
    /// Where the cursor is held, while captured.
    anchor: Option<Pos2>,
}

impl CursorCapture
{
    /// The drag's movement this frame, in points.
    pub fn on_drag( &mut self, ui: &egui::Ui, response: &egui::Response ) -> Vec2
    {
        let ctx = ui.ctx();

        if !response.dragged()
        {
            if self.anchor.take().is_some()
            {
                ctx.send_viewport_cmd( egui::ViewportCommand::CursorGrab( egui::CursorGrab::None ) );
                ctx.send_viewport_cmd( egui::ViewportCommand::CursorVisible( true ) );
            }
            return response.drag_delta();
        }

        let anchor = match self.anchor
        {
            Some( anchor ) => anchor,
            None =>
            {
                let Some( origin ) = ui.input( |input| input.pointer.press_origin() ) else { return response.drag_delta() };
                // Windows only confines the cursor, macOS only locks it; holding it in place covers both.
                let grab = if cfg!( target_os = "macos" ) { egui::CursorGrab::Locked } else { egui::CursorGrab::Confined };
                ctx.send_viewport_cmd( egui::ViewportCommand::CursorGrab( grab ) );
                ctx.send_viewport_cmd( egui::ViewportCommand::CursorVisible( false ) );
                *self.anchor.insert( origin )
            }
        };
        ctx.send_viewport_cmd( egui::ViewportCommand::CursorPosition( anchor ) );

        let ( motion, pixels_per_point ) = ui.input( |input| ( input.pointer.motion(), input.pixels_per_point ) );

        motion.map_or_else( || response.drag_delta(), |motion| motion / pixels_per_point )
    }
}
