use std::collections::{ BTreeMap, HashMap, HashSet };
use std::path::{ Path, PathBuf };

use morvfs::{ Entry, Vfs };

use crate::preview::{ self, Kind };

/// A file inside one of the opened archives.
#[ derive( Clone, Copy, PartialEq, Eq, Hash, Debug ) ]
pub struct FileKey
{
    pub archive: usize,
    pub entry:   usize,
}

/// An archive (`folder` empty) or a folder inside it, `/`-separated.
#[ derive( Clone, PartialEq, Eq, Hash, Debug ) ]
pub struct NodeKey
{
    pub archive: usize,
    pub folder:  String,
}

pub struct Archive
{
    pub name:    String,
    pub vfs:     Vfs,
    pub entries: Vec<Entry>,
    root:        Folder,
}

#[ derive( Default ) ]
struct Folder
{
    folders: BTreeMap<String, Folder>,
    files:   Vec<usize>,
}

pub enum RowKind
{
    Archive,
    Folder,
    File( FileKey, Kind ),
}

pub struct Row
{
    pub depth:    usize,
    pub label:    String,
    pub kind:     RowKind,
    pub node:     Option<NodeKey>,
    pub expanded: bool,
}

#[ derive( Default ) ]
pub struct Library
{
    pub archives: Vec<Archive>,

    /// Lowercase file name to the first file with it, for references between files.
    by_name: HashMap<String, FileKey>,
}

impl Archive
{
    pub fn open( path: &Path ) -> Result<Self, String>
    {
        let vfs = Vfs::open( path ).map_err( |err| format!( "{}: {err}", path.display() ) )?;
        let entries: Vec<Entry> = vfs.entries().cloned().collect();
        let mut root = Folder::default();
        for ( index, entry ) in entries.iter().enumerate()
        {
            let mut folder = &mut root;
            let mut parts  = entry.path.split( '/' ).peekable();
            while let Some( part ) = parts.next()
            {
                if parts.peek().is_none()
                {
                    folder.files.push( index );
                }
                else
                {
                    folder = folder.folders.entry( part.to_owned() ).or_default();
                }
            }
        }
        root.sort_files( &entries );

        Ok( Self { name: path_name( path ), vfs, entries, root } )
    }
}

impl Folder
{
    fn sort_files( &mut self, entries: &[Entry] )
    {
        self.files.sort_by_cached_key( |&index| file_name( &entries[ index ].path ).to_lowercase() );
        for child in self.folders.values_mut()
        {
            child.sort_files( entries );
        }
    }
}

impl Library
{
    pub fn open_folder( dir: &Path ) -> (Self, Vec<String>)
    {
        match list_archives( dir )
        {
            Ok( paths ) => Self::open_paths( &paths ),
            Err( err ) => ( Self::default(), vec![ format!( "{}: {err}", dir.display() ) ] ),
        }
    }

    /// Opens the archives again by their paths, e.g. for a loader thread of its own.
    pub fn open_paths( paths: &[PathBuf] ) -> (Self, Vec<String>)
    {
        let mut archives = Vec::new();
        let mut errors   = Vec::new();

        for path in paths
        {
            match Archive::open( path )
            {
                Ok( archive ) => archives.push( archive ),
                Err( err ) => errors.push( err ),
            }
        }

        ( Self::new( archives ), errors )
    }

    /// Adds an archive next to the open ones; one already open is not opened twice. Returns its index.
    pub fn add_archive( &mut self, path: &Path ) -> Result<usize, String>
    {
        let canonical = |path: &Path| std::fs::canonicalize( path ).unwrap_or_else( |_| path.to_owned() );
        let wanted    = canonical( path );
        if let Some( index ) = self.archives.iter().position( |archive| canonical( archive.vfs.path() ) == wanted )
        {
            return Ok( index );
        }

        let archive = Archive::open( path )?;
        self.archives.push( archive );
        self.by_name = names( &self.archives );

        Ok( self.archives.len() - 1 )
    }

    /// Closes an archive; the ones after it move down by one.
    pub fn remove( &mut self, index: usize )
    {
        if index < self.archives.len()
        {
            self.archives.remove( index );
            self.by_name = names( &self.archives );
        }
    }

    pub fn paths( &self ) -> Vec<PathBuf>
    {
        self.archives.iter().map( |archive| archive.vfs.path().to_owned() ).collect()
    }

    fn new( archives: Vec<Archive> ) -> Self
    {
        let by_name = names( &archives );

        Self { archives, by_name }
    }

    /// Path and data of a file referenced by name from another file, e.g. a mesh texture.
    pub fn read_by_name( &self, name: &str ) -> Option<(String, Vec<u8>)>
    {
        let key = self.by_name.get( &file_name( &name.replace( '\\', "/" ) ).to_lowercase() )?;
        let ( archive, entry ) = self.entry( *key )?;
        let data = archive.vfs.read_entry( entry ).ok()?;

        Some( ( entry.path.clone(), data ) )
    }

    pub fn entry( &self, key: FileKey ) -> Option<(&Archive, &Entry)>
    {
        let archive = self.archives.get( key.archive )?;
        let entry   = archive.entries.get( key.entry )?;

        Some( ( archive, entry ) )
    }

    /// Visible tree rows. A non-empty `filter` shows matching files with their folders open, but for those in `collapsed`.
    pub fn rows( &self, expanded: &HashSet<NodeKey>, collapsed: &HashSet<NodeKey>, filter: &str ) -> Vec<Row>
    {
        let needle   = filter.to_lowercase();
        let open     = Open { expanded, collapsed, needle: &needle };
        let mut rows = Vec::new();

        for ( index, archive ) in self.archives.iter().enumerate()
        {
            let node         = NodeKey { archive: index, folder: String::new() };
            let is_open      = open.is_open( &node );
            let mut children = Vec::new();
            if is_open || open.searching()
            {
                archive.folder_rows( index, &archive.root, "", 1, &open, &mut children );
            }
            if open.searching() && children.is_empty()
            {
                continue;
            }

            let row = Row { depth: 0, label: archive.name.clone(), kind: RowKind::Archive, node: Some( node ), expanded: is_open };
            rows.push( row );
            if is_open
            {
                rows.append( &mut children );
            }
        }

        rows
    }
}

impl Archive
{
    fn folder_rows(
        &self,
        archive: usize,
        folder: &Folder,
        prefix: &str,
        depth: usize,
        open: &Open,
        rows: &mut Vec<Row>,
    )
    {
        for ( name, child ) in &folder.folders
        {
            let path = if prefix.is_empty() { name.clone() } else { format!( "{prefix}/{name}" ) };
            let node         = NodeKey { archive, folder: path.clone() };
            let is_open      = open.is_open( &node );
            let mut children = Vec::new();
            if is_open || open.searching()
            {
                self.folder_rows( archive, child, &path, depth + 1, open, &mut children );
                if open.searching() && children.is_empty()
                {
                    continue;
                }
            }

            let row = Row { depth, label: name.clone(), kind: RowKind::Folder, node: Some( node ), expanded: is_open };
            rows.push( row );
            if is_open
            {
                rows.append( &mut children );
            }
        }

        for &entry in &folder.files
        {
            let path = &self.entries[ entry ].path;
            if open.searching() && !path.to_lowercase().contains( open.needle )
            {
                continue;
            }
            let key   = FileKey { archive, entry };
            let label = file_name( path ).to_owned();
            let row   = Row { depth, label, kind: RowKind::File( key, preview::kind_of( path ) ), node: None, expanded: false };
            rows.push( row );
        }
    }
}

/// Which folders are open: the expanded ones, or while searching all but the collapsed ones.
struct Open<'a>
{
    expanded:  &'a HashSet<NodeKey>,
    collapsed: &'a HashSet<NodeKey>,

    /// Lowercase filter; empty when not searching.
    needle: &'a str,
}

impl Open<'_>
{
    fn searching( &self ) -> bool
    {
        !self.needle.is_empty()
    }

    fn is_open( &self, node: &NodeKey ) -> bool
    {
        if self.searching() { !self.collapsed.contains( node ) } else { self.expanded.contains( node ) }
    }
}

/// Lowercase file name to the first file with it, archives in order.
fn names( archives: &[Archive] ) -> HashMap<String, FileKey>
{
    let mut by_name = HashMap::new();

    for ( archive_index, archive ) in archives.iter().enumerate()
    {
        for ( entry, item ) in archive.entries.iter().enumerate()
        {
            by_name.entry( file_name( &item.path ).to_lowercase() ).or_insert( FileKey { archive: archive_index, entry } );
        }
    }

    by_name
}

pub fn file_name( path: &str ) -> &str
{
    path.rsplit( '/' ).next().unwrap_or( path )
}

fn path_name( path: &Path ) -> String
{
    path.file_name().map( |name| name.to_string_lossy().into_owned() ).unwrap_or_default()
}

fn list_archives( dir: &Path ) -> std::io::Result<Vec<PathBuf>>
{
    let mut archives: Vec<PathBuf> = std::fs::read_dir( dir )?
        .filter_map( |entry| entry.ok().map( |entry| entry.path() ) )
        .filter( |path| path.is_file() && preview::has_extension( &path.to_string_lossy(), morvfs::EXTENSION ) )
        .collect();
    archives.sort();

    Ok( archives )
}
