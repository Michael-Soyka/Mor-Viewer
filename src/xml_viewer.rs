use std::ops::Range;

use eframe::egui::{ Align2, Color32, FontId, Pos2, ScrollArea, Sense, TextFormat, TextStyle, Vec2, text::LayoutJob };

use crate::preview::DecodedText;
use crate::theme::{ Palette, Syntax };
use crate::viewer::Sections;
use crate::widgets::{ from_f32, to_f32 };

const PADDING:    f32  = 6.0;
const ROW_GAP:    f32  = 2.0;
const NUMBER_GAP: f32  = 16.0;
const TAB:        &str = "    ";

#[ derive( Clone, Copy, PartialEq, Eq ) ]
enum Token
{
    Text,
    Punctuation,
    Tag,
    Attribute,
    Value,
    Comment,
}

pub struct XmlViewer
{
    text:     String,
    encoding: &'static str,

    /// Byte ranges of each line with their highlighting.
    lines: Vec<Vec<(Range<usize>, Token)>>,

    longest: usize,
}

impl XmlViewer
{
    pub fn new( decoded: DecodedText ) -> Self
    {
        let text    = decoded.text.replace( "\r\n", "\n" ).replace( '\t', TAB );
        let lines   = split_lines( &text, &tokenize( &text ) );
        let longest = text.lines().map( |line| line.chars().count() ).max().unwrap_or( 0 );

        Self { text, encoding: decoded.encoding, lines, longest }
    }

    pub fn show( &self, sections: &mut Sections, palette: &Palette )
    {
        let ui         = &mut sections.view;
        let font       = TextStyle::Monospace.resolve( ui.style() );
        let char_width = ui.ctx().fonts_mut( |fonts| fonts.glyph_width( &font, '0' ) );
        let row_height = ui.text_style_height( &TextStyle::Monospace ) + ROW_GAP;
        let rows       = self.lines.len();
        let digits     = rows.max( 1 ).to_string().len();
        let text_x     = PADDING + ( char_width * to_f32( digits ) ) + NUMBER_GAP;
        let width      = text_x + ( char_width * to_f32( self.longest ) ) + PADDING;
        let height     = ( row_height * to_f32( rows ) ) + ( PADDING * 2.0 );

        ScrollArea::both().id_salt( "xml" ).auto_shrink( false ).show_viewport(
            ui,
            |ui, viewport|
            {
                let ( content, _ ) = ui.allocate_exact_size( Vec2::new( width.max( viewport.width() ), height ), Sense::hover() );
                let origin  = content.min + Vec2::splat( PADDING );
                let painter = ui.painter();
                let first   = from_f32( ( viewport.min.y / row_height ).floor().max( 0.0 ) );
                let last    = from_f32( ( viewport.max.y / row_height ).ceil().max( 0.0 ) ).min( rows );
                for ( index, spans ) in self.lines.iter().enumerate().take( last ).skip( first )
                {
                    let y        = origin.y + ( row_height * to_f32( index ) );
                    let number   = ( index + 1 ).to_string();
                    let number_x = origin.x + ( char_width * to_f32( digits ) );
                    painter.text( Pos2::new( number_x, y ), Align2::RIGHT_TOP, number, font.clone(), palette.syntax.line_number );
                    let galley = painter.layout_job( line_job( &self.text, spans, &font, &palette.syntax ) );
                    painter.galley( Pos2::new( content.min.x + text_x, y ), galley, palette.syntax.text );
                }
            }
        );

        sections.footer_text( palette, &format!( "{} lines · {}", self.lines.len(), self.encoding ) );
    }
}

fn line_job( text: &str, spans: &[(Range<usize>, Token)], font: &FontId, syntax: &Syntax ) -> LayoutJob
{
    let mut job = LayoutJob::default();

    for ( range, token ) in spans
    {
        let color = token_color( *token, syntax );
        job.append( &text[ range.clone() ], 0.0, TextFormat::simple( font.clone(), color ) );
    }

    job
}

fn token_color( token: Token, syntax: &Syntax ) -> Color32
{
    match token
    {
        Token::Text => syntax.text,
        Token::Punctuation => syntax.punctuation,
        Token::Tag => syntax.tag,
        Token::Attribute => syntax.attribute,
        Token::Value => syntax.value,
        Token::Comment => syntax.comment,
    }
}

/// Lenient XML highlighter: markup, comments, declarations and CDATA; anything else is text.
fn tokenize( text: &str ) -> Vec<(Range<usize>, Token)>
{
    let bytes     = text.as_bytes();
    let mut spans = Vec::new();
    let mut pos   = 0;
    let push = |range: Range<usize>, token: Token, spans: &mut Vec<(Range<usize>, Token)>|
    {
        if !range.is_empty()
        {
            spans.push( ( range, token ) );
        }
    };
    let find = |from: usize, pattern: &str| text[ from.. ].find( pattern ).map_or( text.len(), |offset| from + offset + pattern.len() );

    while pos < bytes.len()
    {
        if bytes[ pos ] != b'<'
        {
            let end = text[ pos.. ].find( '<' ).map_or( text.len(), |offset| pos + offset );
            push( pos..end, Token::Text, &mut spans );
            pos = end;
            continue;
        }
        let rest = &text[ pos.. ];
        if rest.starts_with( "<!--" )
        {
            let end = find( pos, "-->" );
            push( pos..end, Token::Comment, &mut spans );
            pos = end;
            continue;
        }
        if rest.starts_with( "<![CDATA[" )
        {
            let end = find( pos, "]]>" );
            push( pos..end, Token::Text, &mut spans );
            pos = end;
            continue;
        }

        // `<`, `</`, `<?` or `<!` then the name.
        let mut cursor = pos + 1;
        if matches!( bytes.get( cursor ), Some( b'/' | b'?' | b'!' ) )
        {
            cursor += 1;
        }
        push( pos..cursor, Token::Punctuation, &mut spans );
        let name_end = scan( bytes, cursor, |byte| !is_name_end( byte ) );
        push( cursor..name_end, Token::Tag, &mut spans );
        cursor = name_end;

        while cursor < bytes.len()
        {
            let byte = bytes[ cursor ];
            if byte == b'>'
            {
                push( cursor..cursor + 1, Token::Punctuation, &mut spans );
                cursor += 1;
                break;
            }
            if ( byte == b'/' || byte == b'?' ) && bytes.get( cursor + 1 ) == Some( &b'>' )
            {
                push( cursor..cursor + 2, Token::Punctuation, &mut spans );
                cursor += 2;
                break;
            }

            if byte == b'"' || byte == b'\''
            {
                let end = text[ cursor + 1.. ].find( char::from( byte ) ).map_or( text.len(), |offset| cursor + 1 + offset + 1 );
                push( cursor..end, Token::Value, &mut spans );
                cursor = end;
            }
            else if byte == b'='
            {
                push( cursor..cursor + 1, Token::Punctuation, &mut spans );
                cursor += 1;
            }
            else if byte.is_ascii_whitespace()
            {
                let end = scan( bytes, cursor, |byte| byte.is_ascii_whitespace() );
                push( cursor..end, Token::Text, &mut spans );
                cursor = end;
            }
            else if byte == b'<'
            {
                break;
            }
            else
            {
                let end = scan( bytes, cursor, |byte| !is_name_end( byte ) && byte != b'"' && byte != b'\'' ).max( cursor + 1 );
                let end = next_char_boundary( text, end );
                push( cursor..end, Token::Attribute, &mut spans );
                cursor = end;
            }
        }
        pos = cursor;
    }

    spans
}

fn is_name_end( byte: u8 ) -> bool
{
    byte.is_ascii_whitespace() || matches!( byte, b'>' | b'/' | b'=' | b'<' | b'?' )
}

fn scan( bytes: &[u8], from: usize, keep: impl Fn( u8 ) -> bool ) -> usize
{
    bytes[ from.. ].iter().position( |&byte| !keep( byte ) ).map_or( bytes.len(), |offset| from + offset )
}

fn next_char_boundary( text: &str, mut index: usize ) -> usize
{
    while index < text.len() && !text.is_char_boundary( index )
    {
        index += 1;
    }

    index
}

/// Cuts token spans at line breaks, dropping the `\n` itself.
fn split_lines( text: &str, spans: &[(Range<usize>, Token)] ) -> Vec<Vec<(Range<usize>, Token)>>
{
    let mut lines = vec![ Vec::new() ];

    for ( range, token ) in spans
    {
        let mut start = range.start;

        for ( offset, _ ) in text[ range.clone() ].match_indices( '\n' )
        {
            let newline = range.start + offset;

            if let Some( line ) = lines.last_mut()
                && newline > start
            {
                line.push( ( start..newline, *token ) );
            }
            lines.push( Vec::new() );
            start = newline + 1;
        }
        if let Some( line ) = lines.last_mut()
            && range.end > start
        {
            line.push( ( start..range.end, *token ) );
        }
    }

    lines
}
