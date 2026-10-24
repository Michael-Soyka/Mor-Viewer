use eframe::egui::emath::GuiRounding;
use eframe::egui::{ self, Color32, ColorImage, CornerRadius, Pos2, Rect, ScrollArea, Sense, TextureHandle, TextureOptions, Vec2 };
use morft::Font;

use rust_i18n::t;
use crate::i18n;
use crate::preview;
use crate::texture_viewer::to_usize;
use crate::theme::{ self, Palette };
use crate::viewer::Sections;
use crate::widgets::{ self, Zoom, from_f32, to_f32 };

const SCALE_MIN:    f32 = 1.0;
const SCALE_MAX:    f32 = 8.0;
const PADDING:      f32 = 6.0;
const CARD_SIZE:    f32 = 60.0;
const CARD_GAP:     f32 = 4.0;
const CARD_PADDING: f32 = 4.0;
const SECTION_GAP:  f32 = 20.0;
const LINE_GAP:     f32 = 5.0;

/// Glyphs of one wrapped line with their pen positions, in font pixels.
struct Line
{
    glyphs: Vec<(char, f32)>,
    width:  f32,
}

pub struct FontViewer
{
    font: Font,

    /// The sample phrase, then the alphabet rows.
    texts: Vec<String>,

    /// Sorted by code; `None` for empty glyphs.
    glyphs: Vec<(char, Option<TextureHandle>)>,

    scale: f32,
}

impl FontViewer
{
    pub fn new( ctx: &egui::Context, font: Font ) -> Self
    {
        let mut texts = vec![ preview::font_sample( &font ).to_owned() ];
        texts.extend( preview::alphabet_rows( &font ).into_iter().filter( |row| !row.is_empty() ) );

        let mut glyphs: Vec<(char, Option<TextureHandle>)> = font
            .glyphs()
            .map(
                |( ch, glyph )|
                {
                    let width = to_usize( glyph.width );
                    ( ch, ( width > 0 ).then( || upload_coverage( ctx, width, to_usize( glyph.height ), &glyph.coverage ) ) )
                }
            )
            .collect();
        glyphs.sort_unstable_by_key( |( ch, _ )| *ch );

        Self { font, texts, glyphs, scale: 1.0 }
    }

    pub fn show( &mut self, sections: &mut Sections, palette: &Palette )
    {
        let widest      = self.font.glyphs().map( |( _, glyph )| to_usize( glyph.width ) ).max().unwrap_or( 0 );
        let glyph_box   = Vec2::new( to_f32( widest ), to_f32( to_usize( self.font.glyph_height() ) ) ) * self.scale;
        let code_height = theme::CAPTION_TEXT_SIZE + 2.0;

        let card = Vec2::new(
            CARD_SIZE.max( glyph_box.x + ( CARD_PADDING * 2.0 ) ),
            CARD_SIZE.max( glyph_box.y + code_height + ( CARD_PADDING * 3.0 ) ),
        );

        ScrollArea::vertical().id_salt( "font" ).auto_shrink( false ).show(
            &mut sections.view,
            |ui|
            {
                egui::Frame::NONE.inner_margin( PADDING ).show(
                    ui,
                    |ui|
                    {
                        ui.spacing_mut().item_spacing = Vec2::new( PADDING, LINE_GAP );
                        heading( ui, palette, &t!( "font.text" ) );
                        if let Some( sample ) = self.texts.first()
                        {
                            self.wrapped_text( ui, palette, sample );
                        }
                        ui.add_space( SECTION_GAP );

                        heading( ui, palette, &t!( "font.alphabet" ) );
                        for row in self.texts.iter().skip( 1 )
                        {
                            self.wrapped_text( ui, palette, row );
                        }
                        ui.add_space( SECTION_GAP );

                        heading( ui, palette, &t!( "font.glyphs" ) );
                        self.glyph_grid( ui, palette, card, code_height );
                    }
                );
            }
        );

        sections.footer_text( palette, &i18n::count( "font.glyph_count", self.font.glyphs().len() ) );
        if let Zoom::Set( scale ) = widgets::zoom_control( &mut sections.footer_right, palette, self.scale, ( SCALE_MIN, SCALE_MAX ), false )
        {
            self.scale = scale;
        }
    }

    /// Lays `text` out with the game font at the current scale, wrapped to the width, each line centered.
    fn wrapped_text( &self, ui: &mut egui::Ui, palette: &Palette, text: &str )
    {
        let full_uv     = Rect::from_min_max( Pos2::ZERO, Pos2::new( 1.0, 1.0 ) );
        let width       = ui.available_width();
        let line_height = unsigned( self.font.line_height() ) * self.scale;
        // The scroll bar floats over the right edge.
        let usable      = width - ui.spacing().scroll.bar_width;
        let lines       = self.wrap( text, usable / self.scale );
        let ( area, _ ) = ui.allocate_exact_size( Vec2::new( width, line_height * to_f32( lines.len() ) ), Sense::hover() );

        for ( row, line ) in lines.iter().enumerate()
        {
            let top  = area.min.y + ( line_height * to_f32( row ) );
            let left = area.min.x + ( ( width - ( line.width * self.scale ) ) / 2.0 );

            for &( ch, pen ) in &line.glyphs
            {
                let ( Some( glyph ), Some( handle ) ) = ( self.font.glyph( ch ), self.handle( ch ) ) else { continue };
                let min  = Pos2::new( left + ( ( pen + signed( glyph.offset_x ) ) * self.scale ), top + ( signed( glyph.offset_y ) * self.scale ) );
                let rect = Rect::from_min_size( min, handle.size_vec2() * self.scale ).round_to_pixels( ui.pixels_per_point() );
                ui.painter().image( handle.id(), rect, full_uv, palette.text );
            }
        }
    }

    /// Breaks `text` into lines at most `max_width` font pixels wide: at spaces, and between glyphs inside longer words.
    fn wrap( &self, text: &str, max_width: f32 ) -> Vec<Line>
    {
        let space       = self.font.glyph( ' ' ).map_or( 0.0, |glyph| unsigned( glyph.advance ) );
        let mut lines   = Vec::new();
        let mut current = Line { glyphs: Vec::new(), width: 0.0 };

        for word in text.split( ' ' ).filter( |word| !word.is_empty() )
        {
            let ( glyphs, width ) = self.measure( word );
            if width > max_width
            {
                let mut pen = current.width;
                for ( ch, _ ) in glyphs
                {
                    let Some( glyph ) = self.font.glyph( ch ) else { continue };
                    if !current.glyphs.is_empty() && pen + ink_right( glyph ) > max_width
                    {
                        lines.push( std::mem::replace( &mut current, Line { glyphs: Vec::new(), width: 0.0 } ) );
                        pen = 0.0;
                    }
                    current.glyphs.push( ( ch, pen ) );
                    current.width = pen + ink_right( glyph );
                    pen += unsigned( glyph.advance );
                }
                continue;
            }

            if !current.glyphs.is_empty() && current.width + space + width > max_width
            {
                lines.push( std::mem::replace( &mut current, Line { glyphs: Vec::new(), width: 0.0 } ) );
            }
            let start = if current.glyphs.is_empty() { 0.0 } else { current.width + space };
            current.glyphs.extend( glyphs.into_iter().map( |( ch, pen )| ( ch, start + pen ) ) );
            current.width = start + width;
        }

        if !current.glyphs.is_empty()
        {
            lines.push( current );
        }
        lines
    }

    /// Pen positions of a word's glyphs with kerning, and its width up to the rightmost ink.
    fn measure( &self, word: &str ) -> (Vec<(char, f32)>, f32)
    {
        let mut glyphs   = Vec::new();
        let mut pen      = 0.0;
        let mut right    = 0.0f32;
        let mut previous = None;

        for ch in word.chars()
        {
            let Some( glyph ) = self.font.glyph( ch ) else { continue };
            if let Some( left ) = previous
            {
                pen += signed( self.font.kerning( left, ch ) );
            }
            glyphs.push( ( ch, pen ) );
            right = right.max( pen + ink_right( glyph ) );
            pen += unsigned( glyph.advance );
            previous = Some( ch );
        }

        ( glyphs, right )
    }

    fn handle( &self, ch: char ) -> Option<&TextureHandle>
    {
        let index = self.glyphs.binary_search_by_key( &ch, |( glyph, _ )| *glyph ).ok()?;

        self.glyphs[ index ].1.as_ref()
    }

    /// Glyph cards in centered rows.
    fn glyph_grid( &self, ui: &mut egui::Ui, palette: &Palette, card: Vec2, code_height: f32 )
    {
        let full_uv = Rect::from_min_max( Pos2::ZERO, Pos2::new( 1.0, 1.0 ) );
        let width   = ui.available_width();
        let columns = from_f32( ( ( width + CARD_GAP ) / ( card.x + CARD_GAP ) ).floor() ).max( 1 );
        let rows    = self.glyphs.len().div_ceil( columns );
        let height  = ( to_f32( rows ) * ( card.y + CARD_GAP ) ) - CARD_GAP;
        let ( area, _ ) = ui.allocate_exact_size( Vec2::new( width, height.max( 0.0 ) ), Sense::hover() );

        for ( index, ( ch, handle ) ) in self.glyphs.iter().enumerate()
        {
            let ( row, column ) = ( index / columns, index % columns );
            let in_row = columns.min( self.glyphs.len() - ( row * columns ) );
            let used   = ( to_f32( in_row ) * ( card.x + CARD_GAP ) ) - CARD_GAP;
            let left   = area.min.x + ( ( width - used ) / 2.0 ) + ( to_f32( column ) * ( card.x + CARD_GAP ) );
            let rect   = Rect::from_min_size( Pos2::new( left, area.min.y + ( to_f32( row ) * ( card.y + CARD_GAP ) ) ), card );
            if !ui.is_rect_visible( rect )
            {
                continue;
            }
            let Some( glyph ) = self.font.glyph( *ch ) else { continue };

            let response = ui.interact( rect, ui.scope_id().with( ( "glyph", index ) ), Sense::hover() );
            let painter  = ui.painter();
            if response.hovered()
            {
                painter.rect_filled( rect, CornerRadius::same( theme::CONTROL_RADIUS ), palette.hovered );
            }
            let glyph_area = Rect::from_min_max(
                rect.min + Vec2::splat( CARD_PADDING ),
                Pos2::new( rect.max.x - CARD_PADDING, rect.max.y - CARD_PADDING - code_height ),
            );

            if let Some( handle ) = handle
            {
                let glyph_rect = Rect::from_center_size( glyph_area.center(), handle.size_vec2() * self.scale );
                painter.image( handle.id(), glyph_rect.round_to_pixels( ui.pixels_per_point() ), full_uv, palette.text );
            }
            let code      = u32::from( *ch );
            let code_pos  = Pos2::new( rect.center().x, rect.max.y - CARD_PADDING );
            let code_font = egui::FontId::proportional( theme::CAPTION_TEXT_SIZE );
            painter.text( code_pos, egui::Align2::CENTER_BOTTOM, format!( "{code:04X}" ), code_font, palette.text );
            let info = t!(
                "font.glyph_info",
                char = format!( "{ch:?}" ),
                code = format!( "{code:04X}" ),
                width = glyph.width,
                height = glyph.height,
                x = glyph.offset_x,
                y = glyph.offset_y,
                advance = glyph.advance
            );
            response.on_hover_text( info );
        }
    }
}

fn heading( ui: &mut egui::Ui, palette: &Palette, text: &str )
{
    ui.label( egui::RichText::new( text ).font( theme::bold_font( theme::HEADING_TEXT_SIZE ) ).color( palette.text ) );
}

/// Right edge of the glyph's bitmap from its pen position; italics and wide letters reach past the advance.
fn ink_right( glyph: &morft::Glyph ) -> f32
{
    ( signed( glyph.offset_x ) + unsigned( glyph.width ) ).max( unsigned( glyph.advance ) )
}

fn unsigned( value: u32 ) -> f32
{
    to_f32( to_usize( value ) )
}

/// Font offsets and kerning are small; egui lays out in f32.
fn signed( value: i32 ) -> f32
{
    value as f32
}

/// White texture with coverage in alpha, tinted when painted.
fn upload_coverage( ctx: &egui::Context, width: usize, height: usize, coverage: &[u8] ) -> TextureHandle
{
    let pixels = coverage.iter().map( |&alpha| Color32::from_white_alpha( alpha ) ).collect();

    ctx.load_texture( "glyph", ColorImage::new( [ width, height ], pixels ), TextureOptions::NEAREST )
}
