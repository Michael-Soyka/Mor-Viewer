use std::collections::HashMap;

use eframe::egui::ecolor::Hsva;
use eframe::egui::emath::GuiRounding;
use eframe::egui::{ self, Color32, ColorImage, Pos2, Rect, ScrollArea, Sense, Stroke, TextureHandle, TextureOptions, Vec2 };
use morscene::map::{ AreaRect, CityMap, District, NO_AREA };
use mortex::Texture;

use rust_i18n::t;
use crate::i18n;
use crate::texture_viewer::to_usize;
use crate::theme::Palette;
use crate::viewer::Sections;
use crate::widgets::{ self, Zoom, to_f32 };

const ZOOM_MIN:       f32     = 0.05;
const ZOOM_MAX:       f32     = 16.0;
const FIT_MAX:        f32     = 8.0;
const AREA_ALPHA:     f32     = 0.45;
const DISTRICT_COLOR: Color32 = Color32::from_rgb( 0xFF, 0xC8, 0x3C );
const HOVER_COLOR:    Color32 = Color32::WHITE;

/// Label plate: gap above the highlighted area and padding around the text.
const LABEL_GAP:     f32 = 6.0;
const LABEL_PADDING: f32 = 4.0;

/// Area names in `Strings.vfs`: `@<area>` lines, each followed by its name.
const LABELS_FILE: &str = "map.txt";

/// Segment between two world points.
type Edge = [[f32; 2]; 2];

/// Path and data of another file by its name.
type FindFile<'a> = &'a dyn Fn( &str ) -> Option<(String, Vec<u8>)>;

pub struct MapViewer
{
    map: CityMap,

    /// `None` when the map texture is not in the opened archives.
    picture: Option<TextureHandle>,

    areas: TextureHandle,

    /// Outer edges of each district, without the seams between its convex parts.
    outlines: Vec<Vec<Edge>>,

    /// Names by [`AreaRect::area`]; empty when the strings are not in the opened archives.
    labels: HashMap<u32, String>,

    show_areas:     bool,
    show_districts: bool,
    zoom:           f32,
    fit:            bool,

    /// Until the first frame: shrink the map to the view, keep it at 100% if it fits.
    initial: bool,
}

impl MapViewer
{
    pub fn new( ctx: &egui::Context, map: CityMap, find_file: FindFile ) -> Self
    {
        let picture  = find_file( &map.texture ).and_then( |( _, data )| upload_picture( ctx, &data ) );
        let areas    = upload_areas( ctx, &map );
        let outlines = map.districts.iter().map( outline ).collect();
        let labels   = find_file( LABELS_FILE ).map( |( _, data )| area_labels( &data ) ).unwrap_or_default();

        Self { map, picture, areas, outlines, labels, show_areas: true, show_districts: true, zoom: 1.0, fit: false, initial: true }
    }

    pub fn show( &mut self, sections: &mut Sections, palette: &Palette )
    {
        widgets::channel_toggle( &mut sections.action_left, palette, &t!( "map.areas" ), palette.text, &mut self.show_areas );
        widgets::channel_toggle( &mut sections.action_left, palette, &t!( "map.districts" ), DISTRICT_COLOR, &mut self.show_districts );

        let hovered = self.canvas( &mut sections.view, palette );

        let mut segments = vec!
        [
            format!( "{}×{}", self.map.width, self.map.height ),
            i18n::count( "map.district_count", self.map.districts.len() ),
            i18n::count( "map.area_count", self.map.rects.len() ),
        ];

        if self.picture.is_none()
        {
            segments.push( t!( "map.no_picture", name = self.map.texture ).into_owned() );
        }
        if let Some( ( pixel, area ) ) = hovered
        {
            let [ x, z ] = self.world( pixel );
            segments.push( format!( "x {x:.0}, z {z:.0}" ) );
            if let Some( area ) = area
            {
                let label = self.labels.get( &area.area ).cloned().unwrap_or_else( || t!( "map.area", id = area.area ).into_owned() );
                segments.push( label );
            }
        }
        sections.footer_segments( palette, &segments );

        match widgets::zoom_control( &mut sections.footer_right, palette, self.zoom, ( ZOOM_MIN, ZOOM_MAX ), self.fit )
        {
            Zoom::Unchanged => {}
            Zoom::Set( zoom ) =>
            {
                self.zoom = zoom;
                self.fit = false;
            }
            Zoom::Fit => self.fit = true,
        }
    }

    /// Picture pixel under the pointer and the area there, if any.
    fn canvas( &mut self, ui: &mut egui::Ui, palette: &Palette ) -> Option<(Pos2, Option<AreaRect>)>
    {
        let map_size  = Vec2::new( u32_to_f32( self.map.width ), u32_to_f32( self.map.height ) );
        let available = ui.available_size();
        let fitted    = ( available.x / map_size.x ).min( available.y / map_size.y );

        if self.fit
        {
            self.zoom = fitted.clamp( ZOOM_MIN, FIT_MAX );
        }
        else if std::mem::take( &mut self.initial )
        {
            self.zoom = fitted.clamp( ZOOM_MIN, 1.0 );
        }
        let hovered    = ui.rect_contains_pointer( ui.max_rect() );
        let zoom_delta = ui.input( |input| input.zoom_delta() );
        if hovered && zoom_delta != 1.0
        {
            self.zoom = ( self.zoom * zoom_delta ).clamp( ZOOM_MIN, ZOOM_MAX );
            self.fit = false;
        }

        let size = map_size * self.zoom;

        let output = ScrollArea::both().id_salt( "map" ).auto_shrink( false ).show(
            ui,
            |ui|
            {
                let ( area, response ) = ui.allocate_exact_size( size.max( ui.available_size() ), Sense::hover() );
                let rect    = Rect::from_center_size( area.center(), size ).round_to_pixels( ui.pixels_per_point() );
                let painter = ui.painter();
                let full_uv = Rect::from_min_max( Pos2::ZERO, Pos2::new( 1.0, 1.0 ) );

                if let Some( picture ) = &self.picture
                {
                    painter.image( picture.id(), rect, full_uv, Color32::WHITE );
                }
                else
                {
                    painter.rect_filled( rect, 0.0, Color32::from_gray( 40 ) );
                }
                if self.show_areas
                {
                    painter.image( self.areas.id(), rect, full_uv, Color32::WHITE );
                }
                if self.show_districts
                {
                    let stroke = Stroke::new( 1.5, DISTRICT_COLOR );
                    for edge in self.outlines.iter().flatten()
                    {
                        let points = edge.map( |point| rect.min + ( self.pixel( point ).to_vec2() * self.zoom ) );
                        painter.line_segment( points, stroke );
                    }
                }

                let pixel = response.hover_pos().filter( |pos| rect.contains( *pos ) ).map( |pos| ( ( pos - rect.min ) / self.zoom ).to_pos2() );
                let area  = pixel.and_then( |pixel| self.map.area( f32_to_u32( pixel.x ), f32_to_u32( pixel.y ) ).copied() );
                if let Some( area ) = &area
                {
                    let bounds    = area_bounds( area );
                    let highlight = Rect::from_min_max( rect.min + ( bounds.min.to_vec2() * self.zoom ), rect.min + ( bounds.max.to_vec2() * self.zoom ) );
                    painter.rect_stroke( highlight, 0.0, Stroke::new( 1.0, HOVER_COLOR ), egui::StrokeKind::Outside );
                    if let Some( label ) = self.labels.get( &area.area )
                    {
                        paint_label( painter, palette, label, highlight.center_top() );
                    }
                }

                pixel.map( |pixel| ( pixel, area ) )
            }
        );

        output.inner
    }

    /// World x and z to picture pixels; z grows up the picture.
    fn pixel( &self, [ x, z ]: [f32; 2] ) -> Pos2
    {
        let half = Vec2::new( u32_to_f32( self.map.width ), u32_to_f32( self.map.height ) ) / 2.0;

        Pos2::new( half.x + ( ( x - self.map.center[ 0 ] ) * self.map.scale ), half.y - ( ( z - self.map.center[ 1 ] ) * self.map.scale ) )
    }

    fn world( &self, pixel: Pos2 ) -> [f32; 2]
    {
        let half = Vec2::new( u32_to_f32( self.map.width ), u32_to_f32( self.map.height ) ) / 2.0;

        [ self.map.center[ 0 ] + ( ( pixel.x - half.x ) / self.map.scale ), self.map.center[ 1 ] - ( ( pixel.y - half.y ) / self.map.scale ) ]
    }
}

/// Edges used by one convex part only; the ones two parts share lie inside the district.
fn outline( district: &District ) -> Vec<Edge>
{
    let key = |point: [f32; 2]| point.map( f32::to_bits );

    let mut uses: HashMap<[[u32; 2]; 2], (Edge, usize)> = HashMap::new();

    for polygon in &district.polygons
    {
        let next = polygon.points.iter().cycle().skip( 1 );
        for ( &from, &to ) in polygon.points.iter().zip( next )
        {
            let ( a, b ) = ( key( from ), key( to ) );
            let entry    = uses.entry( if a < b { [ a, b ] } else { [ b, a ] } ).or_insert( ( [ from, to ], 0 ) );
            entry.1 += 1;
        }
    }
    let edges = uses.into_values().filter( |&( _, count )| count == 1 ).map( |( edge, _ )| edge ).collect();

    edges
}

fn upload_picture( ctx: &egui::Context, data: &[u8] ) -> Option<TextureHandle>
{
    let image = Texture::parse( data ).ok()?.decode( 0, 0 ).ok()?;
    let color = ColorImage::from_rgba_unmultiplied( [ to_usize( image.width ), to_usize( image.height ) ], &image.rgba );

    Some( ctx.load_texture( "map picture", color, TextureOptions::LINEAR ) )
}

/// Name on a plate centered above `anchor`.
fn paint_label( painter: &egui::Painter, palette: &Palette, label: &str, anchor: Pos2 )
{
    let galley = painter.layout_no_wrap( label.to_owned(), egui::FontId::proportional( crate::theme::TEXT_SIZE ), palette.text );
    let text   = Rect::from_center_size( anchor - Vec2::new( 0.0, ( galley.size().y / 2.0 ) + LABEL_GAP ), galley.size() );

    painter.rect( text.expand( LABEL_PADDING ), egui::CornerRadius::same( crate::theme::CONTROL_RADIUS ), palette.window, Stroke::new( 1.0, palette.border ), egui::StrokeKind::Outside );
    painter.galley( text.min, galley, palette.text );
}

/// `@<area>` lines from [`LABELS_FILE`], each with the lines up to the next one as the name.
fn area_labels( data: &[u8] ) -> HashMap<u32, String>
{
    let text       = crate::preview::decode_text( data ).text;
    let mut labels = HashMap::new();
    let mut key    = None;

    for line in text.lines().map( str::trim )
    {
        if let Some( area ) = line.strip_prefix( '@' ).and_then( |id| id.parse::<u32>().ok() )
        {
            key = Some( area );
        }
        else if let Some( area ) = key
            && !line.is_empty()
        {
            labels.entry( area ).and_modify( |label: &mut String| { label.push( ' ' ); label.push_str( line ); } ).or_insert_with( || line.to_owned() );
        }
    }

    labels
}

/// One translucent color per area, spread around the hue circle.
fn upload_areas( ctx: &egui::Context, map: &CityMap ) -> TextureHandle
{
    let colors: HashMap<u16, Color32> = map.areas.iter().copied().filter( |&area| area != NO_AREA ).map( |area| ( area, area_color( area ) ) ).collect();
    let pixels = map.areas.iter().map( |area| colors.get( area ).copied().unwrap_or( Color32::TRANSPARENT ) ).collect();
    let image  = ColorImage::new( [ to_usize( map.width ), to_usize( map.height ) ], pixels );

    ctx.load_texture( "map areas", image, TextureOptions::NEAREST )
}

fn area_color( area: u16 ) -> Color32
{
    let hue = ( f32::from( area ) * 0.618_034 ).fract();

    Color32::from( Hsva::new( hue, 0.75, 1.0, AREA_ALPHA ) )
}

fn area_bounds( rect: &AreaRect ) -> Rect
{
    Rect::from_min_max( Pos2::new( u32_to_f32( rect.left ), u32_to_f32( rect.top ) ), Pos2::new( u32_to_f32( rect.right ), u32_to_f32( rect.bottom ) ) )
}

fn u32_to_f32( value: u32 ) -> f32
{
    to_f32( to_usize( value ) )
}

/// Pointer pixels are never negative here; truncation picks the pixel under the pointer.
fn f32_to_u32( value: f32 ) -> u32
{
    u32::try_from( widgets::from_f32( value ) ).unwrap_or( u32::MAX )
}
