use std::collections::{ BTreeMap, HashMap, HashSet };
use std::path::{ Path, PathBuf };

use morvfs::{ Entry, Vfs };

use crate::data::preview::{ self, Kind };

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

/// An opened `.vfs` archive or a folder of unpacked game files; also, unseen in the tree, the files opened by themselves.
pub struct Archive
{
    pub name:    String,
    pub entries: Vec<Entry>,
    source:      Source,
    root:        Folder,
}

/// Where an archive's files are read from.
enum Source
{
    Vfs( Vfs ),

    /// Files unpacked into a folder; entry paths are relative to it.
    Unpacked( PathBuf ),

    /// Files opened by themselves: the path of each entry, in order; `None` once its tab is closed.
    Files( Vec<Option<PathBuf>> ),
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
    /// A `.vfs` archive, or a folder of unpacked files.
    pub fn open( path: &Path ) -> Result<Self, String>
    {
        let ( source, entries ) = if path.is_dir() { open_unpacked( path )? } else { open_vfs( path )? };
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

        Ok( Self { name: path_name( path ), entries, source, root } )
    }

    /// The group of files opened by themselves.
    fn files() -> Self
    {
        Self { name: String::new(), entries: Vec::new(), source: Source::Files( Vec::new() ), root: Folder::default() }
    }

    /// Whether this is the group of files opened by themselves, which the tree leaves out.
    pub fn is_loose( &self ) -> bool
    {
        matches!( self.source, Source::Files( _ ) )
    }

    /// Where a file opened by itself lies on disk.
    pub fn loose_path( &self, index: usize ) -> Option<&Path>
    {
        let Source::Files( files ) = &self.source else { return None };

        files.get( index )?.as_deref()
    }

    /// Whether the entry can be read: all but the files let go of.
    fn is_live( &self, index: usize ) -> bool
    {
        !self.is_loose() || self.loose_path( index ).is_some()
    }

    /// What the archive was opened from: the archive or the folder, or each of its files.
    fn sources( &self ) -> Vec<&Path>
    {
        match &self.source
        {
            Source::Vfs( vfs ) => vec![ vfs.path() ],
            Source::Unpacked( dir ) => vec![ dir.as_path() ],
            Source::Files( files ) => files.iter().flatten().map( PathBuf::as_path ).collect(),
        }
    }

    pub fn read( &self, index: usize ) -> Result<Vec<u8>, String>
    {
        let entry = self.entries.get( index ).ok_or_else( || format!( "no entry {index}" ) )?;
        match &self.source
        {
            Source::Vfs( vfs ) => vfs.read_entry( entry ).map_err( |err| err.to_string() ),
            Source::Unpacked( dir ) => std::fs::read( dir.join( &entry.path ) ).map_err( |err| err.to_string() ),
            Source::Files( files ) =>
            {
                let path = files.get( index ).and_then( Option::as_ref ).ok_or_else( || format!( "{}: closed", entry.path ) )?;
                std::fs::read( path ).map_err( |err| err.to_string() )
            }
        }
    }

    /// Adds `path` to a group of files opened by themselves; returns its entry, the existing one if it is there.
    fn add_file( &mut self, path: &Path ) -> Result<usize, String>
    {
        let Source::Files( files ) = &mut self.source else { return Err( format!( "{}: not a file group", self.name ) ) };
        let wanted = canonical( path );
        if let Some( index ) = files.iter().position( |file| file.as_deref().is_some_and( |file| canonical( file ) == wanted ) )
        {
            return Ok( index );
        }

        let metadata = std::fs::metadata( path ).map_err( |err| format!( "{}: {err}", path.display() ) )?;
        files.push( Some( path.to_owned() ) );
        self.entries.push( file_entry( path_name( path ), &metadata ) );

        Ok( self.entries.len() - 1 )
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
    /// The game's data folder: its `.vfs` archives, and with `unpacked` the folders next to them;
    /// with `unpacked` and no archives, `dir` opens whole as unpacked files.
    pub fn open_folder( dir: &Path, unpacked: bool ) -> (Self, Vec<String>)
    {
        match list_sources( dir, unpacked )
        {
            Ok( paths ) => Self::open_paths( &paths ),
            Err( err ) => ( Self::default(), vec![ format!( "{}: {err}", dir.display() ) ] ),
        }
    }

    /// Opens archives, unpacked folders and files by their paths, leaving out empty folders; also for a loader thread of its own.
    pub fn open_paths( paths: &[PathBuf] ) -> (Self, Vec<String>)
    {
        let mut archives = Vec::new();
        let mut files    = Vec::new();
        let mut errors   = Vec::new();

        for path in paths
        {
            if is_game_file( path )
            {
                files.push( path );
                continue;
            }
            match Archive::open( path )
            {
                Ok( archive ) if archive.entries.is_empty() => {}
                Ok( archive ) => archives.push( archive ),
                Err( err ) => errors.push( err ),
            }
        }

        let mut library = Self::new( archives );

        for path in files
        {
            if let Err( err ) = library.add_file( path )
            {
                errors.push( err );
            }
        }

        ( library, errors )
    }

    /// Adds an archive or an unpacked folder next to the open ones; one already open is not opened twice. Returns its index.
    pub fn add_archive( &mut self, path: &Path ) -> Result<usize, String>
    {
        let wanted = canonical( path );
        if let Some( index ) = self.archives.iter().position( |archive| archive.sources().iter().any( |source| canonical( source ) == wanted ) )
        {
            return Ok( index );
        }

        let archive = Archive::open( path )?;
        if archive.entries.is_empty()
        {
            return Err( format!( "{}: no files", path.display() ) );
        }
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

    /// Adds a game file opened by itself, for its tab and for references by name; one already open is not added twice.
    pub fn add_file( &mut self, path: &Path ) -> Result<FileKey, String>
    {
        let archive = match self.archives.iter().position( Archive::is_loose )
        {
            Some( index ) => index,
            None =>
            {
                self.archives.push( Archive::files() );
                self.archives.len() - 1
            }
        };
        let added = self.archives[ archive ].add_file( path );

        if self.archives[ archive ].entries.is_empty()
        {
            self.archives.remove( archive );
        }
        self.by_name = names( &self.archives );

        Ok( FileKey { archive, entry: added? } )
    }

    /// Lets go of the files opened by themselves but those in `open`, the ones still in tabs.
    pub fn retain_files( &mut self, open: &HashSet<FileKey> )
    {
        let Some( archive ) = self.archives.iter().position( Archive::is_loose ) else { return };
        let Source::Files( files ) = &mut self.archives[ archive ].source else { return };
        let mut changed = false;

        for ( entry, file ) in files.iter_mut().enumerate()
        {
            if file.is_some() && !open.contains( &FileKey { archive, entry } )
            {
                *file = None;
                changed = true;
            }
        }
        if changed
        {
            self.by_name = names( &self.archives );
        }
    }

    /// Archives and unpacked folders, without the files opened by themselves.
    pub fn archive_count( &self ) -> usize
    {
        self.archives.iter().filter( |archive| !archive.is_loose() ).count()
    }

    /// What the archives were opened from, to open them again with [`Library::open_paths`].
    pub fn paths( &self ) -> Vec<PathBuf>
    {
        self.archives.iter().flat_map( |archive| archive.sources() ).map( Path::to_path_buf ).collect()
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
        let data = archive.read( key.entry ).ok()?;

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

        for ( index, archive ) in self.archives.iter().enumerate().filter( |( _, archive )| !archive.is_loose() )
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

/// Lowercase file name to the first file with it: files opened by themselves and unpacked folders before archives, each in order.
fn names( archives: &[Archive] ) -> HashMap<String, FileKey>
{
    let mut order: Vec<usize> = ( 0..archives.len() ).collect();
    order.sort_by_key( |&index| matches!( archives[ index ].source, Source::Vfs( _ ) ) );

    let mut by_name = HashMap::new();

    for archive_index in order
    {
        let archive = &archives[ archive_index ];
        for ( entry, item ) in archive.entries.iter().enumerate().filter( |&( entry, _ )| archive.is_live( entry ) )
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

fn open_vfs( path: &Path ) -> Result<(Source, Vec<Entry>), String>
{
    let vfs     = Vfs::open( path ).map_err( |err| format!( "{}: {err}", path.display() ) )?;
    let entries = vfs.entries().cloned().collect();

    Ok( ( Source::Vfs( vfs ), entries ) )
}

/// Every file under `dir` but the archives, with paths relative to it.
fn open_unpacked( dir: &Path ) -> Result<(Source, Vec<Entry>), String>
{
    let mut entries = Vec::new();
    collect_files( dir, "", &mut entries ).map_err( |err| format!( "{}: {err}", dir.display() ) )?;

    Ok( ( Source::Unpacked( dir.to_owned() ), entries ) )
}

fn collect_files( dir: &Path, prefix: &str, entries: &mut Vec<Entry> ) -> std::io::Result<()>
{
    for item in std::fs::read_dir( dir )?
    {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        let path = if prefix.is_empty() { name } else { format!( "{prefix}/{name}" ) };
        let kind = item.file_type()?;
        if kind.is_dir()
        {
            collect_files( &item.path(), &path, entries )?;
        }
        else if kind.is_file() && !preview::has_extension( &path, morvfs::EXTENSION )
        {
            entries.push( file_entry( path, &item.metadata()? ) );
        }
    }

    Ok( () )
}

/// An entry for a file on disk, at `path` in its archive.
fn file_entry( path: String, metadata: &std::fs::Metadata ) -> Entry
{
    let size     = u32::try_from( metadata.len() ).unwrap_or( u32::MAX );
    let modified = metadata.modified().map_or( 0, morvfs::system_time_to_filetime );

    Entry { path, offset: 0, size, modified }
}

/// A file that opens by itself rather than as an archive.
pub fn is_game_file( path: &Path ) -> bool
{
    path.is_file() && !preview::has_extension( &path.to_string_lossy(), morvfs::EXTENSION )
}

fn canonical( path: &Path ) -> PathBuf
{
    std::fs::canonicalize( path ).unwrap_or_else( |_| path.to_owned() )
}

/// The `.vfs` archives in `dir` by name, and with `unpacked` the folders next to them, or `dir` itself without archives.
fn list_sources( dir: &Path, unpacked: bool ) -> std::io::Result<Vec<PathBuf>>
{
    let paths: Vec<PathBuf> = std::fs::read_dir( dir )?.filter_map( |entry| entry.ok().map( |entry| entry.path() ) ).collect();
    let is_archive          = |path: &PathBuf| path.is_file() && preview::has_extension( &path.to_string_lossy(), morvfs::EXTENSION );
    if unpacked && !paths.iter().any( is_archive )
    {
        return Ok( vec![ dir.to_owned() ] );
    }

    let mut sources: Vec<PathBuf> = paths.into_iter().filter( |path| is_archive( path ) || ( unpacked && path.is_dir() ) ).collect();
    sources.sort();

    Ok( sources )
}
