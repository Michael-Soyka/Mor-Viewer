//! Time of day as the game's `weather.bin` runs it: four looks of sky, light and fog,
//! and the hours the script moves between them.

use crate::ui::widgets::to_f32;
use crate::render::texture::FindTexture;

/// Sky texture sets, in the order their layers are uploaded; a face is `{set}_{face}.tex`.
const SETS: [&str; 7] = [ "night", "night_rain", "dawn", "dawn_rain", "day", "day_clear", "day_rain" ];

/// Faces of a set, in layer order; the game has no bottom one.
const FACES: [&str; 5] = [ "bk", "ft", "lt", "rt", "up" ];

/// Faces are read at this size or below.
const SKY_LIMIT: u32 = 512;

/// The script's fog, the same at every hour.
pub const FOG_START: f32 = 3_000.0;
pub const FOG_END:   f32 = 5_000.0;

/// What one `ChangeWeather` call sets: the sky sets for dry and rainy weather, the light and the fog.
#[ derive( Clone, Copy ) ]
struct Look
{
    sky:      usize,
    rain_sky: usize,
    sun:      [u8; 3],
    ambient:  [u8; 3],
    fog:      [u8; 3],
    rain_fog: [u8; 3],
}

const NIGHT:     Look = Look { sky: 0, rain_sky: 1, sun: [ 0, 0, 0 ], ambient: [ 70, 70, 70 ], fog: [ 50, 50, 50 ], rain_fog: [ 50, 50, 50 ] };
const DAWN:      Look = Look { sky: 2, rain_sky: 3, sun: [ 130, 70, 46 ], ambient: [ 180, 129, 82 ], fog: [ 153, 112, 72 ], rain_fog: [ 124, 87, 54 ] };
const DAY:       Look = Look { sky: 4, rain_sky: 6, sun: [ 60, 52, 43 ], ambient: [ 150, 140, 100 ], fog: [ 93, 87, 66 ], rain_fog: [ 95, 83, 74 ] };
const DAY_CLEAR: Look = Look { sky: 5, rain_sky: 6, sun: [ 100, 100, 100 ], ambient: [ 150, 130, 120 ], fog: [ 150, 130, 110 ], rain_fog: [ 95, 83, 74 ] };

/// The script's day: from 6 at night, a change to dawn by 8, to day by 11, to dusk by 19, to night by 21.
/// Each change runs from its start hour to its end hour.
const CHANGES: [(f32, f32); 4] = [ ( 6.0, 8.0 ), ( 10.0, 11.0 ), ( 18.0, 19.0 ), ( 20.0, 21.0 ) ];

/// The world script's street lights: the `SwitchLights` of group [`STREET_GROUP`], off from 7, on from 20.
pub const STREET_GROUP:  u32 = 0;
const STREET_LIGHTS_OFF: f32 = 7.0;
const STREET_LIGHTS_ON:  f32 = 20.0;

/// Whether the street lights burn at `hour`.
pub fn street_lights_on( hour: f32 ) -> bool
{
    !( STREET_LIGHTS_OFF..STREET_LIGHTS_ON ).contains( &hour )
}

#[ derive( Clone, Copy, PartialEq, Eq ) ]
pub enum Weather
{
    Clear,
    Cloudy,
    Rain,
}

impl Weather
{
    pub const ALL: [Self; 3] = [ Self::Clear, Self::Cloudy, Self::Rain ];

    pub fn key( self ) -> &'static str
    {
        match self
        {
            Self::Clear => "model.weather_clear",
            Self::Cloudy => "model.weather_cloudy",
            Self::Rain => "model.weather_rain",
        }
    }
}

/// The look at an hour, ready for the frame: two sky sets blended, the light and the fog, colors 0 to 1.
pub struct Sky
{
    /// First layer of each sky set and the share of the second.
    pub from:  f32,
    pub to:    f32,
    pub blend: f32,

    pub sun:     [f32; 3],
    pub ambient: [f32; 3],
    pub fog:     [f32; 3],
}

/// The look at `hour`, 0 to 24, in `weather`.
pub fn at( hour: f32, weather: Weather ) -> Sky
{
    let day   = if weather == Weather::Clear { DAY_CLEAR } else { DAY };
    let looks = [ NIGHT, DAWN, day, DAWN, NIGHT ];

    // The look before each change is the one it leaves; past the last change it is night again.
    let mut from   = NIGHT;
    let mut to     = NIGHT;
    let mut amount = 0.0;

    for ( ( &( start, end ), &before ), &after ) in CHANGES.iter().zip( &looks ).zip( looks.iter().skip( 1 ) )
    {
        if hour < start
        {
            break;
        }
        ( from, to ) = ( before, after );
        amount = ( ( hour - start ) / ( end - start ) ).clamp( 0.0, 1.0 );
    }

    let rain  = weather == Weather::Rain;
    let sky   = |look: Look| if rain { look.rain_sky } else { look.sky };
    let mix   = |a: [u8; 3], b: [u8; 3]| std::array::from_fn( |channel| ( ( f32::from( a[ channel ] ) * ( 1.0 - amount ) ) + ( f32::from( b[ channel ] ) * amount ) ) / 255.0 );
    let fog   = |look: Look| if rain { look.rain_fog } else { look.fog };
    let first = |set: usize| to_f32( set * FACES.len() );

    let sky = Sky
    {
        from:    first( sky( from ) ),
        to:      first( sky( to ) ),
        blend:   amount,
        sun:     mix( from.sun, to.sun ),
        ambient: mix( from.ambient, to.ambient ),
        fog:     mix( fog( from ), fog( to ) ),
    };

    sky
}

/// Every face of every set as square RGBA layers, sets in [`SETS`] order and faces in [`FACES`] order.
/// The side faces hold the upper half of the cube's; their last row runs on down to the bottom.
pub struct SkyImages
{
    pub side:   u32,
    pub layers: u32,
    pub rgba:   Vec<u8>,
}

impl SkyImages
{
    /// `None` when a face is missing: a scene without the game's sky.
    pub fn load( find_texture: FindTexture ) -> Option<Self>
    {
        let mut rgba = Vec::new();

        for set in SETS
        {
            for face in FACES
            {
                let image = decode( &format!( "{set}_{face}.tex" ), find_texture )?;
                rgba.extend( square( &image ) );
            }
        }

        let layers = u32::try_from( SETS.len() * FACES.len() ).ok()?;
        let images = Self { side: SKY_LIMIT, layers, rgba };

        Some( images )
    }
}

/// The face [`SKY_LIMIT`] wide, its height in proportion: some faces come without mips, some smaller.
fn decode( name: &str, find_texture: FindTexture ) -> Option<mortex::Image>
{
    let ( _, data ) = find_texture( name )?;
    let texture     = mortex::Texture::parse( &data ).ok()?;
    let last        = texture.mip_count().saturating_sub( 1 );
    let mip         = ( 0..last ).find( |&mip| ( texture.width() >> mip ) <= SKY_LIMIT ).unwrap_or( last );
    let image       = texture.decode( 0, mip ).ok()?;

    let height = ( u64::from( image.height ) * u64::from( SKY_LIMIT ) ) / u64::from( image.width.max( 1 ) );
    let fitted = resize( &image, SKY_LIMIT, u32::try_from( height ).ok()?.max( 1 ) );

    Some( fitted )
}

/// `image` at `width` by `height`, each texel the average of the block it covers, or the nearest one when enlarging.
fn resize( image: &mortex::Image, width: u32, height: u32 ) -> mortex::Image
{
    if ( image.width, image.height ) == ( width, height )
    {
        return mortex::Image { width, height, rgba: image.rgba.clone() };
    }

    let ( source_width, source_height ) = ( to_usize( image.width ), to_usize( image.height ) );
    let ( target_width, target_height ) = ( to_usize( width ), to_usize( height ) );
    let mut rgba = Vec::with_capacity( target_width * target_height * 4 );

    for y in 0..target_height
    {
        let rows = span( y, target_height, source_height );
        for x in 0..target_width
        {
            let columns   = span( x, target_width, source_width );
            let mut sum   = [ 0u32; 4 ];
            let mut count = 0u32;

            for row in rows.clone()
            {
                for column in columns.clone()
                {
                    let start = ( ( row * source_width ) + column ) * 4;
                    let Some( texel ) = image.rgba.get( start..start + 4 ) else { continue };
                    for ( total, &channel ) in sum.iter_mut().zip( texel )
                    {
                        *total += u32::from( channel );
                    }
                    count += 1;
                }
            }
            rgba.extend( sum.map( |total| u8::try_from( total / count.max( 1 ) ).unwrap_or( u8::MAX ) ) );
        }
    }

    mortex::Image { width, height, rgba }
}

/// Source texels under target texel `index` of `target` along an axis of `source`; at least one.
fn span( index: usize, target: usize, source: usize ) -> std::ops::Range<usize>
{
    let start = ( index * source ) / target;
    let end   = ( ( ( index + 1 ) * source ) / target ).max( start + 1 ).min( source );

    start..end
}

fn to_usize( value: u32 ) -> usize
{
    usize::try_from( value ).unwrap_or( 0 )
}

/// `image` as a square of its width: the rows it has, then its last row repeated.
fn square( image: &mortex::Image ) -> Vec<u8>
{
    let row      = usize::try_from( image.width ).unwrap_or( 0 ) * 4;
    let rows     = usize::try_from( image.width ).unwrap_or( 0 );
    let last     = image.rgba.len().saturating_sub( row );
    let mut full = Vec::with_capacity( row * rows );

    for index in 0..rows
    {
        let start = ( index * row ).min( last );
        full.extend_from_slice( image.rgba.get( start..start + row ).unwrap_or_default() );
    }

    full
}
