use std::path::PathBuf;

use eframe::egui::layers::ShapeIdx;
use eframe::egui::epaint::RectShape;
use eframe::egui::{ self, Align, Color32, CornerRadius, LayerId, Layout, Order, Pos2, Rect, Shape, TextureHandle, TextureId, UiBuilder, Vec2 };
use eframe::egui_wgpu::RenderState;

use rust_i18n::t;
use crate::audio_viewer::AudioViewer;
use crate::font_viewer::FontViewer;
use crate::hex_viewer::HexViewer;
use crate::map_viewer::MapViewer;
use crate::model_viewer::ModelViewer;
use crate::preview::Preview;
use crate::scene_viewer::SceneViewer;
use crate::settings::ViewerSettings;
use crate::texture_viewer::TextureViewer;
use crate::theme::{ self, Palette };
use crate::widgets::{ self, BAR_HEIGHT };
use crate::xml_viewer::XmlViewer;

const BAR_PADDING: f32 = 8.0;

/// Gap between the view edge and a floating plate.
const PLATE_INSET: f32 = 4.0;

const PLATE_PADDING: f32     = 4.0;
const PLATE_ROW:     f32     = 22.0;
const PLATE_GAP:     f32     = 4.0;

pub enum Viewer
{
    Texture( TextureViewer ),
    Font( FontViewer ),
    Xml( XmlViewer ),
    Audio( AudioViewer ),
    Model( ModelViewer ),
    Map( MapViewer ),
    Scene( SceneViewer ),
    Binary( HexViewer ),
}

impl Viewer
{
    /// Falls back to the binary view when the file does not parse; the error is returned alongside.
    pub fn new( ctx: &egui::Context, path: &str, data: &[u8], resources: &Resources ) -> (Self, Option<String>)
    {
        let loaded = Preview::new( path, data ).and_then(
            |preview| match preview
            {
                Preview::Texture( texture ) => TextureViewer::new( ctx, texture ).map( Self::Texture ),
                Preview::Image( image ) => Ok( Self::Texture( TextureViewer::from_image( ctx, image ) ) ),
                Preview::Font( font ) => Ok( Self::Font( FontViewer::new( ctx, font ) ) ),
                Preview::Text( text ) => Ok( Self::Xml( XmlViewer::new( text ) ) ),
                Preview::Audio( audio ) => Ok( Self::Audio( AudioViewer::new( audio ) ) ),
                Preview::Model( model ) => match resources.render_state
                {
                    Some( state ) => Ok( Self::Model( ModelViewer::new( state, &model, resources.find_file ) ) ),
                    None => Err( t!( "model.needs_wgpu" ).into_owned() ),
                },
                Preview::Scene => match resources.render_state
                {
                    Some( state ) => Ok( Self::Scene( SceneViewer::new( state, resources.archives.to_vec(), path.to_owned() ) ) ),
                    None => Err( t!( "model.needs_wgpu" ).into_owned() ),
                },
                Preview::Map( map ) => Ok( Self::Map( MapViewer::new( ctx, map, resources.find_file ) ) ),
                Preview::Binary => Ok( Self::Binary( HexViewer::new( resources.hex_bytes_per_row ) ) ),
            }
        );

        match loaded
        {
            Ok( viewer ) => ( viewer, None ),
            Err( err ) => ( Self::Binary( HexViewer::new( resources.hex_bytes_per_row ) ), Some( err ) ),
        }
    }

    /// Returns an error to show in the status bar.
    pub fn show( &mut self, ui: &mut egui::Ui, palette: &Palette, checker: &TextureHandle, data: &[u8], view: &ViewerSettings ) -> Option<String>
    {
        match self
        {
            Self::Texture( viewer ) => viewer.show( &mut Sections::floating( ui, palette ), palette, checker ),
            Self::Font( viewer ) =>
            {
                viewer.show( &mut Sections::floating( ui, palette ), palette );
                None
            }
            Self::Xml( viewer ) =>
            {
                viewer.show( &mut Sections::floating( ui, palette ), palette );
                None
            }
            // Audio keeps its docked bars until the design fills its viewer in.
            Self::Audio( viewer ) => viewer.show( &mut Sections::docked( ui ), palette ),
            Self::Model( viewer ) =>
            {
                viewer.show( &mut Sections::floating( ui, &theme::DARK ), palette, view );
                None
            }
            Self::Scene( viewer ) => viewer.show( &mut Sections::floating( ui, &theme::DARK ), palette, view ),
            Self::Map( viewer ) =>
            {
                viewer.show( &mut Sections::floating( ui, palette ), palette );
                None
            }
            Self::Binary( viewer ) =>
            {
                viewer.show( &mut Sections::floating( ui, palette ), palette, data );
                None
            }
        }
    }
}

/// What viewers need from outside the file itself.
pub struct Resources<'a>
{
    pub render_state: Option<&'a RenderState>,

    /// Path and data of another file by its name, e.g. a texture a mesh refers to.
    pub find_file: &'a dyn Fn( &str ) -> Option<(String, Vec<u8>)>,

    pub hex_bytes_per_row: usize,

    /// Paths of the opened archives, for viewers that read on a thread of their own.
    pub archives: &'a [PathBuf],
}

/// Placeholder shown when no tab is open.
pub fn show_blank( ui: &mut egui::Ui, palette: &Palette, hint: &str )
{
    let mut sections = Sections::floating( ui, palette );
    let view         = &mut sections.view;
    view.centered_and_justified( |ui| ui.label( egui::RichText::new( hint ).color( palette.text_weak ) ) );
}

/// Viewer layout: the view area plus action bar and footer parts, each split into left and right.
///
/// Floating sections sit over the view on translucent plates sized to their content;
/// docked ones are solid bars above and below it.
pub struct Sections
{
    pub action_left: egui::Ui,

    /// Right-to-left: the first widget added is the rightmost.
    pub action_right: egui::Ui,

    pub view:        egui::Ui,
    pub footer_left: egui::Ui,

    /// Right-to-left: the first widget added is the rightmost.
    pub footer_right: egui::Ui,

    /// Floating only: a row of its own above the footer, left to right, as for playback.
    pub timeline: Option<egui::Ui>,

    /// Background slots reserved under each floating part, the timeline last, filled once its size is known.
    plates: Option<[ShapeIdx; 5]>,

    plate_fill: Color32,

    /// Blurred view and the rect it covers, shown through the plates.
    backdrop: Option<(TextureId, Rect)>,
}

impl Sections
{
    pub fn floating( ui: &mut egui::Ui, palette: &Palette ) -> Self
    {
        let rect     = ui.max_rect();
        let mut view = ui.new_child( UiBuilder::new().max_rect( rect ).layout( Layout::top_down( Align::Min ) ) );
        view.set_clip_rect( rect );

        let layer  = LayerId::new( Order::Middle, ui.scope_id().with( "viewer_plates" ) );
        let inner  = rect.shrink( PLATE_INSET + PLATE_PADDING );
        let top    = Rect::from_min_size( inner.min, Vec2::new( inner.width(), PLATE_ROW ) );
        let bottom = Rect::from_min_max( Pos2::new( inner.min.x, inner.max.y - PLATE_ROW ), inner.max );
        let above  = bottom.translate( Vec2::new( 0.0, -( PLATE_ROW + ( 2.0 * PLATE_PADDING ) + PLATE_GAP ) ) );

        let plate = |ui: &mut egui::Ui, row: Rect, layout: Layout|
        {
            let mut child = ui.new_child( UiBuilder::new().max_rect( row ).layout( layout ).layer_id( layer ) );
            child.set_clip_rect( rect );
            child.spacing_mut().item_spacing.x = PLATE_GAP;
            let slot = child.painter().add( Shape::Noop );
            ( child, slot )
        };
        let ( action_left, a ) = plate( ui, top, Layout::left_to_right( Align::Center ) );
        let ( action_right, b ) = plate( ui, top, Layout::right_to_left( Align::Center ) );
        let ( footer_left, c ) = plate( ui, bottom, Layout::left_to_right( Align::Center ) );
        let ( footer_right, d ) = plate( ui, bottom, Layout::right_to_left( Align::Center ) );
        let ( timeline, e ) = plate( ui, above, Layout::left_to_right( Align::Center ) );

        Self { action_left, action_right, view, footer_left, footer_right, timeline: Some( timeline ), plates: Some( [ a, b, c, d, e ] ), plate_fill: palette.plate, backdrop: None }
    }

    pub fn docked( ui: &mut egui::Ui ) -> Self
    {
        let rect      = ui.max_rect();
        let action    = Rect::from_min_size( rect.min, Vec2::new( rect.width(), BAR_HEIGHT ) );
        let footer    = Rect::from_min_max( Pos2::new( rect.min.x, rect.max.y - BAR_HEIGHT ), rect.max );
        let view_rect = Rect::from_min_max( Pos2::new( rect.min.x, action.max.y ), Pos2::new( rect.max.x, footer.min.y ) );
        let row = |ui: &mut egui::Ui, rect: Rect, layout: Layout|
        {
            let mut child = ui.new_child( UiBuilder::new().max_rect( rect.shrink2( Vec2::new( BAR_PADDING, 0.0 ) ) ).layout( layout ) );
            child.spacing_mut().item_spacing.x = BAR_PADDING;
            child
        };
        let mut view = ui.new_child( UiBuilder::new().max_rect( view_rect ).layout( Layout::top_down( Align::Min ) ) );
        view.set_clip_rect( view_rect );

        let sections = Self
        {
            action_left:  row( ui, action, Layout::left_to_right( Align::Center ) ),
            action_right: row( ui, action, Layout::right_to_left( Align::Center ) ),
            view,
            footer_left:  row( ui, footer, Layout::left_to_right( Align::Center ) ),
            footer_right: row( ui, footer, Layout::right_to_left( Align::Center ) ),
            timeline:     None,
            plates:       None,
            plate_fill:   Color32::TRANSPARENT,
            backdrop:     None,
        };

        sections
    }

    /// `texture` is a blurred copy of what is drawn over `rect`; the plates painted after this show it through.
    pub fn set_backdrop( &mut self, texture: TextureId, rect: Rect )
    {
        self.backdrop = Some( ( texture, rect ) );
    }

    /// Rounded plate over `rect`: the backdrop under it, if any, toned with `fill`; `opacity` fades both.
    fn plate( &self, rect: Rect, fill: Color32, opacity: f32 ) -> Shape
    {
        let radius = CornerRadius::same( theme::PANEL_RADIUS );
        let tone   = Shape::Rect( RectShape::filled( rect, radius, fill.gamma_multiply( opacity ) ) );
        let Some( ( texture, view ) ) = self.backdrop else { return tone };

        let uv    = Rect::from_min_max( ( ( rect.min - view.min ) / view.size() ).to_pos2(), ( ( rect.max - view.min ) / view.size() ).to_pos2() );
        let glass = RectShape::filled( rect, radius, Color32::WHITE.gamma_multiply( opacity ) ).with_texture( texture, uv );

        Shape::Vec( vec![ Shape::Rect( glass ), tone ] )
    }

    pub fn footer_text( &mut self, palette: &Palette, text: &str )
    {
        self.footer_left.label( egui::RichText::new( text ).size( theme::STATUS_TEXT_SIZE ).color( palette.text ) );
    }

    /// Square of `size` at the bottom right of the view on a plate like the other parts,
    /// lifted above the right footer part when that holds anything. Call after filling the footer.
    pub fn corner( &self, size: f32 ) -> Rect
    {
        let view   = self.view.max_rect();
        let footer = self.footer_right.min_rect();
        let bottom = if footer.width() > 0.0 { footer.min.y - PLATE_PADDING - PLATE_GAP } else { view.max.y - PLATE_INSET };
        let rect   = Rect::from_min_max( Pos2::new( view.max.x - PLATE_INSET - size, bottom - size ), Pos2::new( view.max.x - PLATE_INSET, bottom ) );
        self.view.painter().add( self.plate( rect, self.plate_fill, 1.0 ) );

        rect
    }

    /// Footer segments split by thin separators, as in the status bar.
    pub fn footer_segments( &mut self, palette: &Palette, segments: &[String] )
    {
        status_segments( &mut self.footer_left, palette, segments.iter() );
    }

    /// Same on the right, in reading order.
    pub fn footer_right_segments( &mut self, palette: &Palette, segments: &[String] )
    {
        status_segments( &mut self.footer_right, palette, segments.iter().rev() );
    }

    /// Text on a plate centered at the top of the view, in the row of the action bar.
    pub fn top_text( &self, palette: &Palette, text: &str )
    {
        let painter = self.view.painter();
        let view    = self.view.max_rect();
        let galley  = painter.layout_no_wrap( text.to_owned(), egui::FontId::proportional( theme::STATUS_TEXT_SIZE ), palette.text );
        let center  = Pos2::new( view.center().x, view.min.y + PLATE_INSET + PLATE_PADDING + ( PLATE_ROW / 2.0 ) );
        let text    = Rect::from_center_size( center, galley.size() );

        painter.add( self.plate( text.expand( PLATE_PADDING ), palette.plate, 1.0 ) );
        painter.galley( text.min, galley, palette.text );
    }

    /// Text on a plate centered over the view, just above the footer and the timeline if it holds anything; `opacity` fades it out.
    pub fn hint( &self, palette: &Palette, text: &str, opacity: f32 )
    {
        let painter = self.view.painter();
        let galley  = painter.layout_no_wrap( text.to_owned(), egui::FontId::proportional( theme::STATUS_TEXT_SIZE ), palette.text.gamma_multiply( opacity ) );
        let row     = self.timeline.as_ref().filter( |timeline| timeline.min_rect().width() > 0.0 ).unwrap_or( &self.footer_left );
        let bottom  = row.max_rect().min.y - PLATE_PADDING - PLATE_GAP;
        let center  = Pos2::new( self.view.max_rect().center().x, bottom - PLATE_PADDING - ( galley.size().y / 2.0 ) );
        let text    = Rect::from_center_size( center, galley.size() );

        painter.add( self.plate( text.expand( PLATE_PADDING ), palette.plate, opacity ) );
        painter.galley( text.min, galley, palette.text );
    }
}

fn status_segments<'a>( ui: &mut egui::Ui, palette: &Palette, segments: impl Iterator<Item = &'a String> )
{
    for ( index, segment ) in segments.enumerate()
    {
        if index > 0
        {
            widgets::status_separator( ui, palette );
        }
        ui.label( egui::RichText::new( segment ).size( theme::STATUS_TEXT_SIZE ).color( palette.text ) );
    }
}

impl Drop for Sections
{
    fn drop( &mut self )
    {
        let Some( slots ) = self.plates else { return };
        let parts = [ Some( &self.action_left ), Some( &self.action_right ), Some( &self.footer_left ), Some( &self.footer_right ), self.timeline.as_ref() ];
        for ( part, slot ) in parts.into_iter().zip( slots )
        {
            let Some( part ) = part else { continue };
            let content = part.min_rect();
            if content.width() <= 0.0
            {
                continue;
            }
            part.painter().set( slot, self.plate( content.expand( PLATE_PADDING ), self.plate_fill, 1.0 ) );
        }
    }
}
