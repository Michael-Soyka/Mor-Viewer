use std::collections::HashSet;

use eframe::egui::{ self, Color32, Pos2, Rect, Stroke, Vec2 };

use rust_i18n::t;
use crate::render::math::dot;
use crate::render::scene::SceneLight;
use crate::ui::theme::Palette;
use crate::viewer::model::camera::{ Camera, FOV_Y };

/// Light markers: dot radius and how close the pointer has to be to pick one.
const LIGHT_MARKER: f32 = 4.0;
const LIGHT_PICK:   f32 = 7.0;

/// Axis gizmo: plate, line length from its center, line width.
pub const PIVOT_PLATE: f32 = 72.0;
const PIVOT_LENGTH:    f32 = 26.0;
const PIVOT_STROKE:    f32 = 2.0;

/// Game X, Y and Z in the mirrored space, with their colors.
const PIVOT_AXES: [([f32; 3], Color32); 3] =
[
    ( [ 1.0, 0.0, 0.0 ], Color32::from_rgb( 0x98, 0x4B, 0x4B ) ),
    ( [ 0.0, 1.0, 0.0 ], Color32::from_rgb( 0x51, 0x98, 0x4B ) ),
    ( [ 0.0, 0.0, -1.0 ], Color32::from_rgb( 0x56, 0x93, 0xE3 ) ),
];

/// The light whose marker is under the pointer; the topmost, as the markers are drawn in order.
pub fn light_under( rect: Rect, camera: &Camera, lights: &[SceneLight], pointer: Option<Pos2> ) -> Option<usize>
{
    let pointer = pointer?;
    let matrix  = camera.view_proj( rect.width() / rect.height(), None );

    lights.iter().rposition( |light| project( matrix, light.position, rect ).is_some_and( |( position, _ )| pointer.distance( position ) <= LIGHT_PICK ) )
}

/// Light markers over the view; the hovered one also shows its range and details.
pub fn paint_lights( ui: &egui::Ui, palette: &Palette, rect: Rect, camera: &Camera, lights: &[SceneLight], switched_off: &HashSet<u32>, hovered: Option<usize> )
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
    let galley = painter.layout_no_wrap( text.into_owned(), egui::FontId::proportional( crate::ui::theme::TEXT_SIZE ), palette.text );
    let label  = Rect::from_min_size( position + Vec2::new( LIGHT_PICK, LIGHT_PICK ), galley.size() );
    painter.rect( label.expand( 4.0 ), egui::CornerRadius::same( crate::ui::theme::CONTROL_RADIUS ), palette.window, Stroke::new( 1.0, palette.border ), egui::StrokeKind::Outside );
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
pub fn paint_pivot( painter: &egui::Painter, center: Pos2, camera: &Camera )
{
    let ( side, up, forward ) = camera.basis();

    let mut axes = PIVOT_AXES.map( |( axis, color )| ( Vec2::new( dot( side, axis ), -dot( up, axis ) ), -dot( forward, axis ), color ) );
    axes.sort_by( |a, b| a.1.total_cmp( &b.1 ) );

    for ( direction, _, color ) in axes
    {
        painter.line_segment( [ center, center + ( direction * PIVOT_LENGTH ) ], Stroke::new( PIVOT_STROKE, color ) );
    }
}
