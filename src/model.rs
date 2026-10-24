use std::collections::HashSet;
use std::ops::Range;

use mormesh::layout::interior::InteriorSurface;
use mormesh::{ LightmapPage, Material, Mesh, Surface };
use rust_i18n::t;

/// The game draws water with its own shader; a flat tint stands in for it.
const WATER_COLOR: [f32; 4] = [ 0.25, 0.42, 0.55, 0.0 ];

/// Position, normal, uv, lightmap uv.
pub type Vertex = [f32; 10];

/// One surface of the merged buffers.
pub struct Part
{
    pub triangles: Range<u32>,
    pub edges:     Range<u32>,
    pub texture:   Option<String>,

    /// Base color; alpha is 1 when the texture is alpha-tested.
    pub color: [f32; 4],

    /// Lightmap overlay: index into [`Model::lightmaps`] and the light it is baked for.
    /// Drawn over the plain parts, adding that light's share.
    pub lightmap: Option<( usize, u32 )>,
}

/// 8-bit luminance with its mip levels: light of nearby sources, added to the lighting.
pub struct LightmapImage
{
    pub width:  u32,
    pub height: u32,
    pub mips:   Vec<Vec<u8>>,
}

pub struct Model
{
    pub vertices:  Vec<Vertex>,
    pub triangles: Vec<u32>,
    pub edges:     Vec<u32>,
    pub parts:     Vec<Part>,
    pub lightmaps: Vec<LightmapImage>,
    pub center:    [f32; 3],
    pub radius:    f32,

    /// Source surfaces; `parts` joins neighbours with the same material.
    pub surfaces: usize,

    /// Translation key of the mesh layout.
    pub kind: &'static str,
}

/// Surface geometry borrowed from a parsed mesh.
pub struct Source<'a>
{
    positions: &'a [[f32; 3]],
    normals:   &'a [[f32; 3]],
    uvs:       &'a [[f32; 2]],
    triangles: &'a [[u16; 3]],
    texture:   Option<&'a str>,
    color:     [f32; 4],

    /// Lightmap pages per light, by the light actor's id.
    lightmaps: Vec<( u32, &'a [LightmapPage] )>,
}

/// Buffers of [`Model::merge`] as they fill up.
#[ derive( Default ) ]
struct Builder
{
    vertices:  Vec<Vertex>,
    triangles: Vec<u32>,
    edges:     Vec<u32>,
    parts:     Vec<Part>,
    lightmaps: Vec<LightmapImage>,
}

impl Builder
{
    /// Lit triangles go in a part per lightmap page, with the page's own triangulation; the rest as stored.
    fn add( &mut self, source: &Source )
    {
        let normals = if source.normals.len() == source.positions.len()
        {
            source.normals.to_vec()
        }
        else
        {
            smooth_normals( source.positions, source.triangles )
        };
        let vertex = |index: usize, lightmap_uv: [f32; 2]|
        {
            let [ x, y, z ]    = source.positions.get( index ).copied().unwrap_or_default();
            let [ nx, ny, nz ] = normals.get( index ).copied().unwrap_or_default();
            let [ u, v ]       = source.uvs.get( index ).copied().unwrap_or_default();

            [ x, y, z, nx, ny, nz, u, v, lightmap_uv[ 0 ], lightmap_uv[ 1 ] ]
        };

        let base = to_u32( self.vertices.len() );
        self.vertices.extend( ( 0..source.positions.len() ).map( |index| vertex( index, [ 0.0; 2 ] ) ) );
        self.part( source, base, source.triangles.iter().copied(), None );

        for &( light, pages ) in &source.lightmaps
        {
            for page in pages
            {
                let Some( image ) = lightmap_image( page ) else { continue };
                let base = to_u32( self.vertices.len() );
                self.vertices.extend( page.vertices.iter().map( |lightmap_vertex| vertex( usize::from( lightmap_vertex.source ), lightmap_vertex.uv ) ) );
                let lightmap = self.lightmaps.len();
                self.lightmaps.push( image );
                self.part( source, base, page.triangles.iter().copied(), Some( ( lightmap, light ) ) );
            }
        }
    }

    fn part( &mut self, source: &Source, base: u32, triangles: impl Iterator<Item = [u16; 3]>, lightmap: Option<( usize, u32 )> )
    {
        let triangle_start = to_u32( self.triangles.len() );
        let edge_start     = to_u32( self.edges.len() );
        let mut seen       = HashSet::new();

        for triangle in triangles
        {
            let [ a, b, c ] = triangle.map( u32::from );
            self.triangles.extend( [ base + a, base + b, base + c ] );
            for ( from, to ) in [ ( a, b ), ( b, c ), ( c, a ) ]
            {
                if seen.insert( ( from.min( to ), from.max( to ) ) )
                {
                    self.edges.extend( [ base + from, base + to ] );
                }
            }
        }
        if to_u32( self.triangles.len() ) == triangle_start
        {
            return;
        }

        let same_material = lightmap.is_none() && self.parts.last().is_some_and(
            |last: &Part| last.lightmap.is_none() && last.color == source.color && last.texture.as_deref().map( str::to_lowercase ) == source.texture.map( str::to_lowercase )
        );
        match self.parts.last_mut()
        {
            Some( last ) if same_material =>
            {
                last.triangles.end = to_u32( self.triangles.len() );
                last.edges.end = to_u32( self.edges.len() );
            }
            _ =>
            {
                let part = Part
                {
                    triangles: triangle_start..to_u32( self.triangles.len() ),
                    edges:     edge_start..to_u32( self.edges.len() ),
                    texture:   source.texture.map( str::to_owned ),
                    color:     source.color,
                    lightmap,
                };
                self.parts.push( part );
            }
        }
    }
}

/// The first of the page's two images; what the second holds is not established.
fn lightmap_image( page: &LightmapPage ) -> Option<LightmapImage>
{
    let [ levels, _ ] = page.decode().ok()?;
    let first = levels.first()?;
    let image = LightmapImage { width: first.width, height: first.height, mips: levels.iter().map( |level| level.luminance.clone() ).collect() };

    Some( image )
}

impl Model
{
    pub fn parse( data: &[u8] ) -> Result<Self, String>
    {
        let mesh              = Mesh::parse( data ).map_err( |err| err.to_string() )?;
        let ( kind, sources ) = sources( &mesh )?;
        let model             = Self::merge( kind, &sources );

        Ok( model )
    }

    pub fn merge( kind: &'static str, sources: &[Source] ) -> Self
    {
        let mut builder = Builder::default();

        let ( mut min, mut max ) = ( [ f32::MAX; 3 ], [ f32::MIN; 3 ] );

        for source in sources
        {
            for position in source.positions
            {
                for axis in 0..3
                {
                    min[ axis ] = min[ axis ].min( position[ axis ] );
                    max[ axis ] = max[ axis ].max( position[ axis ] );
                }
            }
            builder.add( source );
        }

        let Builder { vertices, triangles, edges, parts, lightmaps } = builder;
        let center = [ 0, 1, 2 ].map( |axis| ( min[ axis ] + max[ axis ] ) / 2.0 );
        let radius = vertices
            .iter()
            .map( |vertex| ( ( vertex[ 0 ] - center[ 0 ] ).powi( 2 ) + ( vertex[ 1 ] - center[ 1 ] ).powi( 2 ) + ( vertex[ 2 ] - center[ 2 ] ).powi( 2 ) ).sqrt() )
            .fold( 0.0f32, f32::max )
            .max( 0.001 );

        Self { vertices, triangles, edges, parts, lightmaps, center, radius, surfaces: sources.len(), kind }
    }

    pub fn triangle_count( &self ) -> usize
    {
        self.triangles.len() / 3
    }
}

/// Surfaces of any mesh layout and the translation key of the layout.
pub fn sources( mesh: &Mesh ) -> Result<(&'static str, Vec<Source<'_>>), String>
{
    let sources = match mesh
    {
        Mesh::Static( mesh ) => ( "model.static", mesh.surfaces.iter().map( surface_source ).collect() ),
        Mesh::Movable( mesh ) => ( "model.movable", mesh.surfaces.iter().map( surface_source ).collect() ),
        Mesh::Land( mesh ) => ( "model.land", mesh.tiles.iter().flat_map( |tile| &tile.surfaces ).map( surface_source ).collect() ),
        Mesh::Interior( mesh ) => ( "model.interior", mesh.surfaces.iter().map( interior_source ).collect() ),
        Mesh::Character( mesh ) =>
        {
            let character = Source
            {
                positions: &mesh.positions,
                normals:   &mesh.normals,
                uvs:       &mesh.uvs,
                triangles: &mesh.triangles,
                texture:   Some( mesh.texture.as_str() ),
                color:     [ 1.0, 1.0, 1.0, 0.0 ],
                lightmaps: Vec::new(),
            };
            ( "model.character", vec![ character ] )
        }
        Mesh::Water( mesh ) =>
        {
            let water = Source
            {
                positions: &mesh.positions,
                normals:   &[],
                uvs:       &[],
                triangles: &mesh.triangles,
                texture:   None,
                color:     WATER_COLOR,
                lightmaps: Vec::new(),
            };
            ( "model.water", vec![ water ] )
        }
        Mesh::Detail( mesh ) => return Err( t!( "model.detail", count = mesh.prototypes.len() ).into_owned() ),
        Mesh::Raw( mesh ) =>
        {
            let raw = Source
            {
                positions: &mesh.positions,
                normals:   &[],
                uvs:       &mesh.uvs,
                triangles: &mesh.triangles,
                texture:   Some( mesh.texture.as_str() ),
                color:     [ 1.0, 1.0, 1.0, 0.0 ],
                lightmaps: Vec::new(),
            };
            ( "model.raw", vec![ raw ] )
        }
    };

    Ok( sources )
}

/// Groups surfaces by material, so `Model::merge` joins them into fewer parts.
pub fn sort_by_material( sources: &mut [Source] )
{
    sources.sort_by_cached_key( |source| ( source.texture.map( str::to_lowercase ), source.color.map( f32::to_bits ) ) );
}

fn surface_source( surface: &Surface ) -> Source<'_>
{
    let ( color, alpha_test ) = match &surface.material
    {
        Material::Color { colors } => ( [ colors[ 0 ][ 0 ], colors[ 0 ][ 1 ], colors[ 0 ][ 2 ] ], false ),
        Material::Diffuse { alpha_test, .. }
        | Material::DiffuseBump { alpha_test, .. }
        | Material::DiffuseSpecular { alpha_test, .. }
        | Material::Unlit { alpha_test, .. } => ( [ 1.0; 3 ], *alpha_test ),
        // Foliage: the alpha channel cuts the leaves out.
        Material::DiffuseOpacity { .. } => ( [ 1.0; 3 ], true ),
        Material::Special { .. } => ( [ 1.0; 3 ], false ),
    };
    let alpha = if alpha_test { 1.0 } else { 0.0 };
    let source = Source
    {
        positions: &surface.positions,
        normals:   &surface.normals,
        uvs:       &surface.uvs,
        triangles: &surface.triangles,
        texture:   surface.material.texture(),
        color:     [ color[ 0 ], color[ 1 ], color[ 2 ], alpha ],
        lightmaps: surface.lightmaps.iter().map( |lightmap| ( lightmap.id, lightmap.pages.as_slice() ) ).collect(),
    };

    source
}

fn interior_source( surface: &InteriorSurface ) -> Source<'_>
{
    let alpha_test = matches!( surface.material, Material::Diffuse { alpha_test: true, .. } | Material::Unlit { alpha_test: true, .. } );
    let alpha      = if alpha_test { 1.0 } else { 0.0 };
    let source = Source
    {
        positions: &surface.positions,
        normals:   &surface.normals,
        uvs:       &surface.uvs,
        triangles: &surface.triangles,
        texture:   surface.material.texture(),
        color:     [ 1.0, 1.0, 1.0, alpha ],
        lightmaps: surface.lightmaps.iter().map( |lightmap| ( lightmap.light, lightmap.pages.as_slice() ) ).collect(),
    };

    source
}

/// Area-weighted vertex normals for surfaces stored without them.
fn smooth_normals( positions: &[[f32; 3]], triangles: &[[u16; 3]] ) -> Vec<[f32; 3]>
{
    let mut normals = vec![ [ 0.0f32; 3 ]; positions.len() ];

    for triangle in triangles
    {
        let [ a, b, c ] = triangle.map( usize::from );
        let ( Some( pa ), Some( pb ), Some( pc ) ) = ( positions.get( a ), positions.get( b ), positions.get( c ) ) else { continue };
        let ab = [ pb[ 0 ] - pa[ 0 ], pb[ 1 ] - pa[ 1 ], pb[ 2 ] - pa[ 2 ] ];
        let ac = [ pc[ 0 ] - pa[ 0 ], pc[ 1 ] - pa[ 1 ], pc[ 2 ] - pa[ 2 ] ];

        let face =
        [
            ( ab[ 1 ] * ac[ 2 ] ) - ( ab[ 2 ] * ac[ 1 ] ),
            ( ab[ 2 ] * ac[ 0 ] ) - ( ab[ 0 ] * ac[ 2 ] ),
            ( ab[ 0 ] * ac[ 1 ] ) - ( ab[ 1 ] * ac[ 0 ] ),
        ];

        for index in [ a, b, c ]
        {
            for axis in 0..3
            {
                normals[ index ][ axis ] += face[ axis ];
            }
        }
    }

    for normal in &mut normals
    {
        let length = ( ( normal[ 0 ] * normal[ 0 ] ) + ( normal[ 1 ] * normal[ 1 ] ) + ( normal[ 2 ] * normal[ 2 ] ) ).sqrt();
        if length > 0.0
        {
            *normal = normal.map( |value| value / length );
        }
    }

    normals
}

pub fn to_u32( value: usize ) -> u32
{
    u32::try_from( value ).unwrap_or( u32::MAX )
}
