use std::collections::{ HashMap, HashSet };
use std::ops::Range;

use mormesh::layout::interior::InteriorSurface;
use mormesh::{ LightmapPage, Material, Mesh, Surface };
use rust_i18n::t;
use crate::render::animation::Skeleton;

/// Base color of water under its reflection: nearly black, the game's water shows the reflection and the bed only.
/// The game sets it from code; picked by eye.
const WATER_COLOR: [f32; 4] = [ 0.04, 0.04, 0.04, 0.0 ];

/// `WaterMesh::kind` of the river; the pond is 1.
const RIVER_KIND: u8 = 2;

/// Grid step clusters are cut by, in world units: a few houses of a city.
const CLUSTER_CELL: f32 = 2048.0;

/// Position, normal, uv, lightmap uv.
pub type Vertex = [f32; 10];

/// Bounding box of triangles of one mesh in one grid cell; culled as a whole.
pub struct Cluster
{
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Triangles and edges of a part that lie in one cluster.
pub struct Draw
{
    pub cluster:   u32,
    pub triangles: Range<u32>,
    pub edges:     Range<u32>,
}

/// One surface of the merged buffers.
pub struct Part
{
    pub triangles: Range<u32>,
    pub edges:     Range<u32>,

    /// The ranges above cut by cluster, in order.
    pub draws: Vec<Draw>,

    pub texture: Option<String>,

    /// Base color; alpha is 1 when the texture is alpha-tested.
    pub color: [f32; 4],

    /// Lightmap overlay: index into [`Model::lightmaps`] and the light it is baked for.
    /// Drawn over the plain parts, adding that light's share.
    pub lightmap: Option<( usize, u32 )>,

    /// Height of the water level the part lies on, if it is water; parts of different levels stay apart.
    pub water: Option<f32>,

    /// Water that mirrors the scene, as the river; the pond only shines.
    pub mirrors: bool,
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
    pub clusters:  Vec<Cluster>,
    pub center:    [f32; 3],
    pub radius:    f32,

    /// Source surfaces; `parts` joins neighbours with the same material.
    pub surfaces: usize,

    /// Translation key of the mesh layout.
    pub kind: &'static str,

    /// Bones and animations of a character; its vertices are the mesh's, in order.
    pub skeleton: Option<Skeleton>,
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

    /// Mesh the surface comes from: clusters do not mix meshes.
    mesh: u32,

    /// A water surface.
    water: bool,

    /// Water that mirrors the scene: the river, the game's `CGeometryWater2`.
    mirrors: bool,
}

impl Source<'_>
{
    pub fn in_mesh( mut self, mesh: u32 ) -> Self
    {
        self.mesh = mesh;

        self
    }
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
    clusters:  Vec<Cluster>,

    /// Cluster ids by mesh and grid cell.
    cells: HashMap<(u32, [u32; 3]), u32>,

    /// Cluster of each triangle and edge, in buffer order.
    triangle_clusters: Vec<u32>,
    edge_clusters:     Vec<u32>,
}

impl Builder
{
    /// Cluster of the triangle's centre, grown over its corners.
    fn cluster( &mut self, mesh: u32, corners: [u32; 3] ) -> u32
    {
        let points = corners.map( |index| self.vertices.get( to_usize( index ) ).map_or( [ 0.0; 3 ], |vertex| [ vertex[ 0 ], vertex[ 1 ], vertex[ 2 ] ] ) );
        // Adding zero turns -0 into 0, so both land in one cell.
        let cell   = [ 0, 1, 2 ].map( |axis| ( ( points[ 0 ][ axis ] + points[ 1 ][ axis ] + points[ 2 ][ axis ] ) / ( 3.0 * CLUSTER_CELL ) ).floor() + 0.0 ).map( f32::to_bits );

        let next = to_u32( self.clusters.len() );
        let id   = *self.cells.entry( ( mesh, cell ) ).or_insert( next );
        if id == next
        {
            self.clusters.push( Cluster { min: [ f32::MAX; 3 ], max: [ f32::MIN; 3 ] } );
        }
        let Some( cluster ) = self.clusters.get_mut( to_usize( id ) ) else { return id };

        for point in points
        {
            for axis in 0..3
            {
                cluster.min[ axis ] = cluster.min[ axis ].min( point[ axis ] );
                cluster.max[ axis ] = cluster.max[ axis ].max( point[ axis ] );
            }
        }

        id
    }

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
            let cluster     = self.cluster( source.mesh, [ base + a, base + b, base + c ] );
            self.triangles.extend( [ base + a, base + b, base + c ] );
            self.triangle_clusters.push( cluster );
            for ( from, to ) in [ ( a, b ), ( b, c ), ( c, a ) ]
            {
                if seen.insert( ( from.min( to ), from.max( to ) ) )
                {
                    self.edges.extend( [ base + from, base + to ] );
                    self.edge_clusters.push( cluster );
                }
            }
        }
        if to_u32( self.triangles.len() ) == triangle_start
        {
            return;
        }

        let water = source.water.then( || mid_height( source.positions ) );

        let same_material = lightmap.is_none() && self.parts.last().is_some_and(
            |last: &Part| last.lightmap.is_none() && last.water == water && last.mirrors == source.mirrors && last.color == source.color && last.texture.as_deref().map( str::to_lowercase ) == source.texture.map( str::to_lowercase )
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
                    draws:     Vec::new(),
                    texture:   source.texture.map( str::to_owned ),
                    color:     source.color,
                    lightmap,
                    water,
                    mirrors:   source.mirrors,
                };
                self.parts.push( part );
            }
        }
    }

    /// Orders each part's triangles and edges by cluster and cuts the part into a draw per cluster.
    fn split_draws( &mut self )
    {
        for part in &mut self.parts
        {
            let triangles = sort_by_cluster::<3>( &mut self.triangles, &self.triangle_clusters, &part.triangles );
            let mut edges = sort_by_cluster::<2>( &mut self.edges, &self.edge_clusters, &part.edges ).into_iter().peekable();

            // An edge takes the cluster of a triangle it belongs to, so both lists run through the clusters in the same order.
            for ( cluster, triangles ) in triangles
            {
                let edges = edges.next_if( |( edge_cluster, _ )| *edge_cluster == cluster ).map_or( 0..0, |( _, run )| run );
                part.draws.push( Draw { cluster, triangles, edges } );
            }
        }
    }
}

/// Stable-sorts the primitives of `N` indices in `range` by cluster; runs of one cluster with their index ranges.
fn sort_by_cluster<const N: usize>( indices: &mut [u32], clusters: &[u32], range: &Range<u32> ) -> Vec<(u32, Range<u32>)>
{
    let Some( slice ) = indices.get_mut( to_usize( range.start )..to_usize( range.end ) ) else { return Vec::new() };
    let first = to_usize( range.start ) / N;

    let mut primitives: Vec<(u32, [u32; N])> = slice
        .chunks_exact( N )
        .zip( clusters.get( first.. ).unwrap_or_default() )
        .map( |( chunk, &cluster )| ( cluster, <[u32; N]>::try_from( chunk ).unwrap_or( [ 0; N ] ) ) )
        .collect();
    primitives.sort_by_key( |&( cluster, _ )| cluster );

    for ( chunk, ( _, primitive ) ) in slice.chunks_exact_mut( N ).zip( &primitives )
    {
        chunk.copy_from_slice( primitive );
    }

    let step     = to_u32( N );
    let mut runs = Vec::<(u32, Range<u32>)>::new();

    for ( offset, &( cluster, _ ) ) in ( range.start.. ).step_by( N ).zip( &primitives )
    {
        match runs.last_mut()
        {
            Some( ( last, run ) ) if *last == cluster => run.end = offset + step,
            _ => runs.push( ( cluster, offset..offset + step ) ),
        }
    }

    runs
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
        let mut model         = Self::merge( kind, &sources );

        if let Mesh::Character( character ) = &mesh
        {
            model.skeleton = Skeleton::new( character );
        }

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
        builder.split_draws();

        let Builder { vertices, triangles, edges, parts, lightmaps, clusters, .. } = builder;
        let center = [ 0, 1, 2 ].map( |axis| ( min[ axis ] + max[ axis ] ) / 2.0 );
        let radius = vertices
            .iter()
            .map( |vertex| ( ( vertex[ 0 ] - center[ 0 ] ).powi( 2 ) + ( vertex[ 1 ] - center[ 1 ] ).powi( 2 ) + ( vertex[ 2 ] - center[ 2 ] ).powi( 2 ) ).sqrt() )
            .fold( 0.0f32, f32::max )
            .max( 0.001 );

        Self { vertices, triangles, edges, parts, lightmaps, clusters, center, radius, surfaces: sources.len(), kind, skeleton: None }
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
                mesh:      0,
                water:     false,
                mirrors:   false,
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
                mesh:      0,
                water:     true,
                mirrors:   mesh.kind == RIVER_KIND,
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
                mesh:      0,
                water:     false,
                mirrors:   false,
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
        mesh:      0,
        water:     false,
        mirrors:   false,
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
        mesh:      0,
        water:     false,
        mirrors:   false,
    };

    source
}

/// Middle of the points' heights: the level of a water surface, flat or nearly so.
fn mid_height( positions: &[[f32; 3]] ) -> f32
{
    let ( low, high ) = positions.iter().fold( ( f32::MAX, f32::MIN ), |( low, high ), position| ( low.min( position[ 1 ] ), high.max( position[ 1 ] ) ) );

    if low > high { 0.0 } else { ( low + high ) / 2.0 }
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

pub fn to_usize( value: u32 ) -> usize
{
    usize::try_from( value ).unwrap_or( usize::MAX )
}
