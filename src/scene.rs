use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{ AtomicUsize, Ordering };

use eframe::egui::Color32;
use encoding_rs::WINDOWS_1251;
use mormesh::Mesh;
use morscene::Scene;
use morscene::actor::{ ActorFile, LightActor };
use morscene::path::PathFinding;

use crate::library::Library;
use crate::model::{ self, Model };
use crate::model_viewer::{ self, PathLines, SceneLight, Textures };

/// Wider textures load from a smaller mip: a city uses hundreds of them.
const TEXTURE_LIMIT: u32 = 512;

/// The player's start point; the camera starts there.
const START_POINT: &str = "birth";

/// Eye height above a point on the ground, in world units.
const EYE_HEIGHT: f32 = 170.0;

/// Path lines sit this far above the floors they lie on, so the floors do not hide them.
const PATH_LIFT: f32 = 4.0;

const PATH_CELL_COLOR: [f32; 3] = [ 0.2, 0.8, 0.9 ];
const PATH_LINK_COLOR: [f32; 3] = [ 1.0, 0.6, 0.2 ];

/// Meshes read so far, for the loading message.
#[ derive( Default ) ]
pub struct Progress
{
    pub done:  AtomicUsize,
    pub total: AtomicUsize,
}

pub struct LoadedScene
{
    pub model:    Model,
    pub textures: Textures,
    pub missing:  Vec<String>,
    pub lights:   Vec<SceneLight>,
    pub paths:    Option<PathLines>,

    /// `<Ambient>` of interiors, 0 to 255; outdoor scenes have the sun instead.
    pub ambient: Option<[f32; 3]>,

    /// Game position and direction of the start point.
    pub start: Option<([f32; 3], [f32; 3])>,
}

/// Every mesh the scene's actors and terrain name, merged into one model.
/// Meshes are stored in world space, so they need no placement.
pub fn load( archives: &[PathBuf], file: &str, progress: &Progress ) -> Result<LoadedScene, String>
{
    let ( library, _ ) = Library::open_paths( archives );
    let read = |name: &str| library.read_by_name( name ).map( |( _, data )| data );

    let scene = Scene::load( file, read ).map_err( |err| err.to_string() )?;

    let actors: Vec<(&str, Vec<u8>)> = scene.actors.iter().filter_map( |actor| actor.name.as_deref() ).filter_map( |name| Some( ( name, read( name )? ) ) ).collect();
    let lights = actors.iter().flat_map( |( name, data )| actor_lights( name, data ) ).collect();

    let mut names: Vec<String> = actors.iter().filter_map( |( _, data )| actor_mesh( data ) ).collect();
    names.extend( scene.land.as_deref().and_then( |land| actor_mesh( &read( land )? ) ) );
    let mut seen = HashSet::new();
    names.retain( |name| seen.insert( name.to_lowercase() ) );
    progress.total.store( names.len(), Ordering::Relaxed );

    let mut meshes = Vec::new();

    for name in &names
    {
        if let Some( mesh ) = read( name ).and_then( |data| Mesh::parse( &data ).ok() )
        {
            meshes.push( mesh );
        }
        progress.done.fetch_add( 1, Ordering::Relaxed );
    }

    let mut sources = Vec::new();

    for mesh in &meshes
    {
        if let Ok( ( _, list ) ) = model::sources( mesh )
        {
            sources.extend( list );
        }
    }
    model::sort_by_material( &mut sources );
    let model = Model::merge( "model.scene", &sources );
    drop( sources );
    drop( meshes );

    let find_texture          = |name: &str| library.read_by_name( name );
    let ( textures, missing ) = model_viewer::decode_textures( &model, &find_texture, Some( TEXTURE_LIMIT ) );
    let start                 = scene.points.iter().find( |point| point.name.eq_ignore_ascii_case( START_POINT ) ).map( |point| ( eye( point.position ), point.direction ) );

    let paths  = scene.path_finding.as_deref().and_then( |file| path_lines( &read( file )? ) );
    let loaded = LoadedScene { model, textures, missing, lights, paths, ambient: scene.ambient, start };

    Ok( loaded )
}

fn eye( [ x, y, z ]: [f32; 3] ) -> [f32; 3]
{
    [ x, y + EYE_HEIGHT, z ]
}

/// Outlines of the navigation cells, and the graph: a node at each cell edge's middle, linked to the edges reachable from it.
fn path_lines( data: &[u8] ) -> Option<PathLines>
{
    let path   = PathFinding::parse( data ).ok()?;
    let vertex = |[ x, y, z ]: [f32; 3], [ r, g, b ]: [f32; 3]| [ x, y + PATH_LIFT, z, r, g, b, 0.0, 0.0, 0.0, 0.0 ];

    let mut vertices: Vec<_> = path.vertices.iter().map( |&position| vertex( position, PATH_CELL_COLOR ) ).collect();
    let vertex_count         = model::to_u32( vertices.len() );
    let mut cells            = Vec::new();
    let mut seen             = HashSet::new();

    for cell in &path.cells
    {
        let next = cell.vertices.iter().cycle().skip( 1 );
        for ( &a, &b ) in cell.vertices.iter().zip( next )
        {
            if a < vertex_count && b < vertex_count && seen.insert( ( a.min( b ), a.max( b ) ) )
            {
                cells.extend( [ a, b ] );
            }
        }
    }

    let nodes = vertex_count;

    for edge in &path.edges
    {
        let from   = path.vertices.get( usize::try_from( edge.from ).ok()? )?;
        let to     = path.vertices.get( usize::try_from( edge.to ).ok()? )?;
        let middle = [ 0, 1, 2 ].map( |axis| ( from[ axis ] + to[ axis ] ) / 2.0 );
        vertices.push( vertex( middle, PATH_LINK_COLOR ) );
    }

    let edge_count = model::to_u32( path.edges.len() );
    let mut links  = Vec::new();
    seen.clear();

    for ( index, edge ) in ( 0.. ).zip( &path.edges )
    {
        for link in edge.links.iter().filter( |link| link.edge < edge_count )
        {
            if seen.insert( ( index.min( link.edge ), index.max( link.edge ) ) )
            {
                links.extend( [ nodes + index, nodes + link.edge ] );
            }
        }
    }

    let lines = PathLines { vertices, cells, links };

    Some( lines )
}

/// Lights of a light actor: the light itself, or each light of a group.
fn actor_lights( name: &str, data: &[u8] ) -> Vec<SceneLight>
{
    let Ok( ActorFile::Light( actor ) ) = ActorFile::parse( data ) else { return Vec::new() };
    let LightActor { color: [ red, green, blue, _ ], light, group, .. } = actor;
    let color  = Color32::from_rgb( red, green, blue );
    let points = group.unwrap_or_else( || vec![ light ] );

    let lights = points.into_iter().map( |point| SceneLight { name: name.to_owned(), id: actor.id, position: point.position, color, radius: point.radius } ).collect();

    lights
}

/// Mesh of a geometry actor. The terrain actor has its own layout that starts with the mesh name.
fn actor_mesh( data: &[u8] ) -> Option<String>
{
    ActorFile::parse( data ).ok().and_then( |file| file.mesh().map( str::to_owned ) ).or_else( || leading_mesh_name( data ) )
}

/// `u8 length, cp1251 name` at the start, if the name is a mesh file.
fn leading_mesh_name( data: &[u8] ) -> Option<String>
{
    let length = usize::from( *data.first()? );
    let name   = WINDOWS_1251.decode_without_bom_handling( data.get( 1..1 + length )? ).0.into_owned();

    name.to_lowercase().ends_with( ".mesh" ).then_some( name )
}
