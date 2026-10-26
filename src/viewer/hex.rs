use eframe::egui::{ Align2, Pos2, Rect, ScrollArea, Sense, Stroke, TextStyle, Vec2 };

use rust_i18n::t;
use crate::i18n;
use crate::ui::theme::Palette;
use crate::ui::widgets::{ self, from_f32, to_f32 };
use crate::viewer::Sections;

const PADDING:       f32        = 6.0;
const ROW_GAP:       f32        = 2.0;
const ROW_WIDTHS:    [usize; 4] = [ 8, 16, 32, 64 ];
const NON_PRINTABLE: char       = '.';
const OFFSET_CHARS:  usize      = 8;

/// Bytes per group; groups and columns are split by `GAP`.
const GROUP: usize = 8;
const GAP:   f32   = 5.0;

pub struct HexViewer
{
    per_row: usize,
}

impl HexViewer
{
    pub fn new( per_row: usize ) -> Self
    {
        Self { per_row }
    }

    pub fn show( &mut self, sections: &mut Sections, palette: &Palette, data: &[u8] )
    {
        let per_row    = self.per_row;
        let ui         = &mut sections.view;
        let font       = TextStyle::Monospace.resolve( ui.style() );
        let char_width = ui.ctx().fonts_mut( |fonts| fonts.glyph_width( &font, '0' ) );
        let row_height = ui.text_style_height( &TextStyle::Monospace ) + ROW_GAP;
        let rows       = data.len().div_ceil( per_row );
        let groups     = per_row.div_ceil( GROUP );

        // Columns: offset | byte groups | decoded text, split by thin separators.
        let chars           = |count: usize| char_width * to_f32( count );
        let first_separator = PADDING + chars( OFFSET_CHARS ) + GAP;
        let group_width     = chars( ( GROUP * 3 ) - 1 );
        let group_x: Vec<f32> = ( 0..groups ).map( |group| first_separator + GAP + ( to_f32( group ) * ( group_width + GAP ) ) ).collect();
        let second_separator = group_x.last().copied().unwrap_or( first_separator ) + group_width + GAP;
        let text_x           = second_separator + GAP;
        let width            = text_x + chars( per_row ) + PADDING;
        let header_height    = PADDING + row_height;
        let height           = header_height + ( row_height * to_f32( rows ) ) + PADDING;

        let labels: Vec<String> = ( 0..groups )
            .map(
                |group|
                {
                    let columns: Vec<String> = ( group * GROUP..per_row.min( ( group + 1 ) * GROUP ) ).map( |column| format!( "{column:02X}" ) ).collect();
                    columns.join( " " )
                }
            )
            .collect();

        ScrollArea::both().id_salt( "hex" ).auto_shrink( false ).show_viewport(
            ui,
            |ui, viewport|
            {
                let ( content, _ ) = ui.allocate_exact_size( Vec2::new( width, height ), Sense::hover() );
                let origin  = content.min;
                let painter = ui.painter();

                let text = |x: f32, y: f32, value: String, color|
                {
                    painter.text( Pos2::new( origin.x + x, y ), Align2::LEFT_TOP, value, font.clone(), color );
                };

                let first = from_f32( ( ( viewport.min.y - header_height ) / row_height ).floor().max( 0.0 ) );
                let last  = from_f32( ( ( viewport.max.y - header_height ) / row_height ).ceil().max( 0.0 ) ).min( rows );
                for row in first..last
                {
                    let start = row * per_row;
                    let chunk = &data[ start..data.len().min( start + per_row ) ];
                    let y     = origin.y + header_height + ( row_height * to_f32( row ) );
                    text( PADDING, y, format!( "{start:08X}" ), palette.text );
                    for ( group, x ) in group_x.iter().enumerate()
                    {
                        let bytes = chunk.iter().skip( group * GROUP ).take( GROUP );
                        let hex: Vec<String> = bytes.map( |byte| format!( "{byte:02X}" ) ).collect();
                        text( *x, y, hex.join( " " ), palette.text );
                    }
                    let ( printable, dots ) = decode_row( chunk );
                    text( text_x, y, printable, palette.text );
                    text( text_x, y, dots, palette.text_weak );
                }

                // Sticky header over the rows, separators through the whole height.
                let header_top = origin.y + viewport.min.y;
                let header     = Rect::from_min_size( Pos2::new( origin.x, header_top ), Vec2::new( width, header_height ) );
                painter.rect_filled( header, 0.0, palette.view );
                let y = header_top + PADDING;

                for ( label, x ) in labels.iter().zip( &group_x )
                {
                    text( *x, y, label.clone(), palette.text );
                }
                text( text_x, y, t!( "hex.decoded" ).into_owned(), palette.text );
                let lines = Rect::from_min_max( Pos2::new( origin.x, header_top + PADDING ), Pos2::new( origin.x + width, origin.y + viewport.max.y ) );

                for x in [ first_separator, second_separator ]
                {
                    painter.vline( origin.x + x, lines.y_range(), Stroke::new( 1.0, palette.border ) );
                }
            }
        );

        let response = widgets::dropdown_button( &mut sections.footer_right, palette, &i18n::count( "hex.bytes", per_row ) );
        eframe::egui::Popup::menu( &response ).show(
            |ui|
            {
                for width in ROW_WIDTHS
                {
                    if ui.selectable_label( width == per_row, i18n::count( "hex.bytes", width ) ).clicked()
                    {
                        self.per_row = width;
                    }
                }
            }
        );
    }
}

/// Decoded text as two overlaid strings: printable characters, and dots for the rest.
fn decode_row( chunk: &[u8] ) -> (String, String)
{
    let mut printable = String::new();
    let mut dots      = String::new();

    for &byte in chunk
    {
        if byte.is_ascii_graphic()
        {
            printable.push( char::from( byte ) );
            dots.push( ' ' );
        }
        else
        {
            printable.push( ' ' );
            dots.push( NON_PRINTABLE );
        }
    }

    ( printable, dots )
}
