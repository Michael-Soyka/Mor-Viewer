use morft::Font;
use morscene::map::CityMap;
use mortex::{ Image, Texture };

use rust_i18n::t;
use crate::render::model::Model;

pub const MODEL_EXTENSIONS: [&str; 4] = [ "mesh", "mms", "mma", "lsh" ];
pub const TEXT_EXTENSIONS:  [&str; 3] = [ "xml", "txt", "json" ];
pub const OGG_EXTENSION:    &str      = "ogg";
pub const MAP_EXTENSION:    &str      = "map";
pub const PATH_EXTENSION:   &str      = "pfd";

pub const SAMPLE_CYRILLIC: &str =
    "Трагики съели все французские булочки! Да и чай тоже выпили… Ёмкую чашу щербета эти хитрые ребята осилить уже не смогли.";
pub const SAMPLE_LATIN: &str = "The Tragedians devoured six juicy, plump French buns! And quickly drank the warm, zesty tea too…";

#[ derive( Clone, Copy, PartialEq, Eq, Debug ) ]
pub enum Kind
{
    Texture,
    Font,
    Model,
    Text,
    Audio,
    Map,
    Scene,
    Binary,
}

pub fn kind_of( path: &str ) -> Kind
{
    if has_extension( path, mortex::EXTENSION ) || image_format( path ).is_some()
    {
        return Kind::Texture;
    }
    if has_extension( path, morft::EXTENSION )
    {
        return Kind::Font;
    }
    if MODEL_EXTENSIONS.iter().any( |ext| has_extension( path, ext ) )
    {
        return Kind::Model;
    }
    if TEXT_EXTENSIONS.iter().any( |ext| has_extension( path, ext ) )
    {
        return Kind::Text;
    }
    if has_extension( path, OGG_EXTENSION )
    {
        return Kind::Audio;
    }
    if has_extension( path, MAP_EXTENSION )
    {
        return Kind::Map;
    }
    if has_extension( path, morscene::INTERIOR_EXTENSION ) || has_extension( path, morscene::OUTDOOR_EXTENSION )
    {
        return Kind::Scene;
    }

    Kind::Binary
}

pub enum Preview
{
    Texture( Texture ),
    Image( Image ),
    Font( Font ),
    Text( DecodedText ),
    Audio( Audio ),
    Model( Model ),
    Map( CityMap ),

    /// Read by the scene viewer itself, it needs the other archives.
    Scene,

    Binary,
}

impl Preview
{
    pub fn new( path: &str, data: &[u8] ) -> Result<Self, String>
    {
        let preview = match kind_of( path )
        {
            Kind::Texture if image_format( path ).is_some() => Self::Image( decode_image( path, data )? ),
            Kind::Texture => Self::Texture( Texture::parse( data ).map_err( |err| err.to_string() )? ),
            Kind::Font => Self::Font( Font::parse( data ).map_err( |err| err.to_string() )? ),
            Kind::Text => Self::Text( decode_text( data ) ),
            Kind::Audio => Self::Audio( decode_ogg( data )? ),
            Kind::Scene => Self::Scene,
            Kind::Map => Self::Map( CityMap::parse( data ).map_err( |err| err.to_string() )? ),
            Kind::Model if has_extension( path, mormesh::EXTENSION ) => Self::Model( Model::parse( data )? ),
            Kind::Model | Kind::Binary => Self::Binary,
        };

        Ok( preview )
    }
}

pub struct DecodedText
{
    pub text:     String,
    pub encoding: &'static str,
}

/// BOM first, then UTF-8 if valid, otherwise the game's cp1251.
pub fn decode_text( data: &[u8] ) -> DecodedText
{
    if let Some( ( encoding, bom ) ) = encoding_rs::Encoding::for_bom( data )
    {
        let text    = encoding.decode_without_bom_handling( &data[ bom.. ] ).0.into_owned();
        let decoded = DecodedText { text, encoding: encoding.name() };
        return decoded;
    }
    let decoded = match std::str::from_utf8( data )
    {
        Ok( text ) => DecodedText { text: text.to_owned(), encoding: encoding_rs::UTF_8.name() },
        Err( _ ) =>
        {
            let text = encoding_rs::WINDOWS_1251.decode_without_bom_handling( data ).0.into_owned();
            DecodedText { text, encoding: encoding_rs::WINDOWS_1251.name() }
        }
    };

    decoded
}

/// Decoded Ogg Vorbis: interleaved samples in -1..1.
pub struct Audio
{
    pub channels:    u16,
    pub sample_rate: u32,
    pub samples:     Vec<f32>,
    pub comments:    Vec<(String, String)>,
}

pub fn decode_ogg( data: &[u8] ) -> Result<Audio, String>
{
    let mut reader  = lewton::inside_ogg::OggStreamReader::new( std::io::Cursor::new( data ) ).map_err( |err| err.to_string() )?;
    let mut samples = Vec::new();
    while let Some( packet ) = reader.read_dec_packet_itl().map_err( |err| err.to_string() )?
    {
        samples.extend( packet.into_iter().map( |sample| f32::from( sample ) / 32768.0 ) );
    }
    let audio = Audio
    {
        channels:    u16::from( reader.ident_hdr.audio_channels ),
        sample_rate: reader.ident_hdr.audio_sample_rate,
        samples,
        comments:    reader.comment_hdr.comment_list,
    };

    Ok( audio )
}

/// Plain image formats the `image` crate is built with. TGA has no signature, so the extension decides.
pub fn image_format( path: &str ) -> Option<image::ImageFormat>
{
    let extension = path.rsplit_once( '.' )?.1;
    let format    = image::ImageFormat::from_extension( extension )?;

    format.reading_enabled().then_some( format )
}

pub fn decode_image( path: &str, data: &[u8] ) -> Result<Image, String>
{
    let format  = image_format( path ).ok_or( t!( "image.unsupported" ) )?;
    let decoded = image::load_from_memory_with_format( data, format ).map_err( |err| err.to_string() )?.to_rgba8();
    let image   = Image { width: decoded.width(), height: decoded.height(), rgba: decoded.into_raw() };

    Ok( image )
}

/// Picks the sample phrase the font can actually draw.
pub fn font_sample( font: &Font ) -> &'static str
{
    if font.glyph( 'Ж' ).is_some() { SAMPLE_CYRILLIC } else { SAMPLE_LATIN }
}

/// Font glyphs split into rows: uppercase, lowercase, then everything else.
pub fn alphabet_rows( font: &Font ) -> [String; 3]
{
    let mut rows: [Vec<char>; 3] = Default::default();

    for ( ch, _ ) in font.glyphs()
    {
        if ch.is_whitespace() || ch.is_control()
        {
            continue;
        }
        let row = if ch.is_uppercase()
        {
            0
        }
        else if ch.is_lowercase()
        {
            1
        }
        else
        {
            2
        };
        rows[ row ].push( ch );
    }

    rows.map(
        |mut chars|
        {
            chars.sort_unstable();
            let mut line = String::new();

            for ch in chars
            {
                if !line.is_empty()
                {
                    line.push( ' ' );
                }
                line.push( ch );
            }
            line
        }
    )
}

pub fn has_extension( path: &str, extension: &str ) -> bool
{
    path.rsplit_once( '.' ).is_some_and( |( _, ext )| ext.eq_ignore_ascii_case( extension ) )
}

/// Which of R, G, B, A are shown.
#[ derive( Clone, Copy, PartialEq, Eq ) ]
pub struct Channels( pub [bool; 4] );

impl Channels
{
    pub const ALL: Self = Self( [ true; 4 ] );

    /// A single channel is shown as grayscale; without alpha the image is opaque.
    pub fn apply( self, image: &Image ) -> Vec<u8>
    {
        let mut rgba = image.rgba.clone();
        let [ r, g, b, a ] = self.0;
        let single = match self.0.iter().filter( |&&on| on ).count()
        {
            1 => self.0.iter().position( |&on| on ),
            _ => None,
        };

        for pixel in rgba.chunks_exact_mut( 4 )
        {
            if let Some( channel ) = single
            {
                let value = pixel[ channel ];
                pixel.copy_from_slice( &[ value, value, value, u8::MAX ] );
                continue;
            }
            for ( value, on ) in pixel[ ..3 ].iter_mut().zip( [ r, g, b ] )
            {
                if !on
                {
                    *value = 0;
                }
            }
            if !a
            {
                pixel[ 3 ] = u8::MAX;
            }
        }

        rgba
    }
}
