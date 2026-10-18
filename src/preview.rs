use encoding_rs::WINDOWS_1251;
use mortex::{ Image, Texture };

/// Text bigger than this is shown as hex: a TextEdit over megabytes is unusable.
const TEXT_LIMIT: usize = 512 * 1024;

const TEXT_PROBE:  usize = 4096;
pub const HEX_ROW: usize = 16;

pub enum Preview
{
    Texture( Texture ),
    Text( String ),
    Binary,
}

impl Preview
{
    pub fn new( path: &str, data: &[u8] ) -> Result<Self, String>
    {
        if has_extension( path, mortex::EXTENSION )
        {
            let texture = Texture::parse( data ).map_err( |err| err.to_string() )?;
            return Ok( Self::Texture( texture ) );
        }
        if data.len() <= TEXT_LIMIT && looks_like_text( data )
        {
            return Ok( Self::Text( decode_text( data ) ) );
        }

        Ok( Self::Binary )
    }
}

pub fn has_extension( path: &str, extension: &str ) -> bool
{
    path.rsplit_once( '.' ).is_some_and( |( _, ext )| ext.eq_ignore_ascii_case( extension ) )
}

fn looks_like_text( data: &[u8] ) -> bool
{
    let probe = &data[ ..data.len().min( TEXT_PROBE ) ];

    !probe.is_empty() && probe.iter().all( |&byte| byte >= 0x20 || matches!( byte, b'\t' | b'\n' | b'\r' ) )
}

/// UTF-8 when valid, otherwise the game's cp1251.
fn decode_text( data: &[u8] ) -> String
{
    let data = data.strip_prefix( b"\xEF\xBB\xBF" ).unwrap_or( data );

    match std::str::from_utf8( data )
    {
        Ok( text ) => text.to_owned(),
        Err( _ ) => WINDOWS_1251.decode_without_bom_handling( data ).0.into_owned(),
    }
}

pub fn hex_row( data: &[u8], row: usize ) -> String
{
    let start    = row * HEX_ROW;
    let chunk    = &data[ start..data.len().min( start + HEX_ROW ) ];
    let mut line = format!( "{start:08X}  " );

    for column in 0..HEX_ROW
    {
        match chunk.get( column )
        {
            Some( byte ) => line.push_str( &format!( "{byte:02X} " ) ),
            None => line.push_str( "   " ),
        }
        if column == 7
        {
            line.push( ' ' );
        }
    }
    line.push( ' ' );
    line.extend( chunk.iter().map( |&byte| if byte.is_ascii_graphic() || byte == b' ' { char::from( byte ) } else { '.' } ) );

    line
}

#[ derive( Clone, Copy, PartialEq, Eq ) ]
pub enum Channels
{
    Rgba,
    Rgb,
    Alpha,
}

impl Channels
{
    pub const ALL: [Self; 3] = [ Self::Rgba, Self::Rgb, Self::Alpha ];

    pub fn label( self ) -> &'static str
    {
        match self
        {
            Self::Rgba => "RGBA",
            Self::Rgb => "RGB",
            Self::Alpha => "Альфа",
        }
    }

    pub fn apply( self, image: &Image ) -> Vec<u8>
    {
        let mut rgba = image.rgba.clone();

        for pixel in rgba.chunks_exact_mut( 4 )
        {
            match self
            {
                Self::Rgba => {}
                Self::Rgb => pixel[ 3 ] = u8::MAX,
                Self::Alpha => pixel.copy_from_slice( &[ pixel[ 3 ], pixel[ 3 ], pixel[ 3 ], u8::MAX ] ),
            }
        }

        rgba
    }
}
