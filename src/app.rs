use std::collections::{ HashMap, HashSet };
use std::path::{ Path, PathBuf };
use std::time::{ Duration, Instant };

use eframe::egui_wgpu::wgpu;
use eframe::egui::{
    self, Align, CornerRadius, CursorIcon, FontId, Key, Layout, Pos2, Rect, ScrollArea, Sense, Stroke, StrokeKind, TextureHandle, UiBuilder, Vec2,
};

use mormesh::Mesh;
use mormesh::gltf::Gltf;
use morscene::Scene;

use rust_i18n::t;
use crate::gltf;
use crate::i18n;
use crate::icons;
use crate::library::{ FileKey, Library, NodeKey, Row, RowKind, file_name };
use crate::model_viewer::{ self, ModelThumbnail };
use crate::preview::Kind;
use crate::settings::{ self, AboutInfo, Page, Settings, SidebarLayout };
use crate::texture_viewer;
use crate::theme::{ self, Palette };
use crate::title_bar::{ self, TitleBar };
use crate::viewer::{ self, Resources, Viewer };
use crate::widgets::{ self, ROW_GAP, ROW_HEIGHT, RowIcon, to_f32 };

const GUTTER:                 f32   = 4.0;
const SIDEBAR_WIDTH:          f32   = 200.0;
const SIDEBAR_MIN:            f32   = 140.0;
const SIDEBAR_PADDING:        f32   = 2.0;
const SEARCH_GAP:             f32   = 4.0;
const TABS_HEIGHT:            f32   = 21.0;
const MAIN_PADDING:           f32   = 2.0;
const TABS_SCROLL_STEP:       f32   = 120.0;
const STATUS_HEIGHT:          f32   = 25.0;
const CARD_SIZE:              Vec2  = Vec2::new( 72.0, 104.0 );
const CARD_GAP:               f32   = 5.0;
const THUMB_SIZE:             f32   = 64.0;
const THUMBS_PER_FRAME:       usize = 3;
const MODEL_THUMBS_PER_FRAME: usize = 1;

/// Model thumbnails are rendered at twice the card slot for high-DPI screens.
const MODEL_THUMB_PIXELS: u32 = 128;

struct Tab
{
    key:    FileKey,
    name:   String,
    data:   Vec<u8>,
    viewer: Viewer,

    /// Unpinned tabs are reused by the next single-click open.
    pinned: bool,
}

#[ derive( Clone, Copy ) ]
enum Export
{
    Raw,
    Png,
    Gltf,
}

impl Export
{
    /// Format a file converts to besides its raw bytes: textures to PNG, meshes and scenes to glTF.
    fn conversion( name: &str ) -> Option<Self>
    {
        match crate::preview::kind_of( name )
        {
            Kind::Texture => Some( Self::Png ),
            Kind::Model if crate::preview::has_extension( name, mormesh::EXTENSION ) => Some( Self::Gltf ),
            Kind::Scene => Some( Self::Gltf ),
            _ => None,
        }
    }

    fn label( self ) -> &'static str
    {
        match self
        {
            Self::Raw => "context.export_raw",
            Self::Png => "context.export_png",
            Self::Gltf => "context.export_gltf",
        }
    }
}

enum Action
{
    Toggle( NodeKey ),
    Open( FileKey, bool ),
    Export( FileKey, Export ),
    CloseArchive( usize ),
}

enum TabAction
{
    Activate( usize ),
    Pin( usize ),
    Close( usize ),
    CloseOthers( usize ),
    Export( usize, Export ),
}

enum Thumbnail
{
    Image( TextureHandle ),
    Model( ModelThumbnail ),
}

impl Thumbnail
{
    fn id( &self ) -> egui::TextureId
    {
        match self
        {
            Self::Image( handle ) => handle.id(),
            Self::Model( model ) => model.id(),
        }
    }

    fn size( &self ) -> Vec2
    {
        match self
        {
            Self::Image( handle ) => handle.size_vec2(),
            Self::Model( _ ) => Vec2::splat( THUMB_SIZE ),
        }
    }
}

enum GridLine<'a>
{
    Header( &'a Row ),
    Cards( Vec<&'a Row> ),
}

pub struct ViewerApp
{
    palette:    &'static Palette,
    title_bar:  TitleBar,
    library:    Library,
    expanded:   HashSet<NodeKey>,

    /// Folders closed while searching; the search opens all others.
    collapsed: HashSet<NodeKey>,

    filter:     String,
    rows:       Vec<Row>,
    rows_dirty: bool,
    settings:   Settings,

    /// Open preferences page, if any.
    preferences: Option<Page>,

    thumbnails:    HashMap<FileKey, Option<Thumbnail>>,
    tabs:          Vec<Tab>,
    active:        Option<usize>,
    tabs_overflow: bool,

    /// Tab strip state the active tab was last scrolled into view for: active, tab count, overflow.
    revealed_tab: Option<(Option<usize>, usize, bool)>,

    sidebar_width: f32,
    scroll_to_row: Option<usize>,
    list_offset:   f32,
    list_height:   f32,
    checker:       TextureHandle,
    render_state:  Option<eframe::egui_wgpu::RenderState>,
    message:       Option<String>,

    /// When the last frame's UI finished, for the FPS limit.
    frame_end: Option<Instant>,
}

impl ViewerApp
{
    pub fn new( cc: &eframe::CreationContext<'_>, start: Option<PathBuf> ) -> Self
    {
        let settings: Settings = cc.storage.and_then( |storage| eframe::get_value( storage, settings::STORAGE_KEY ) ).unwrap_or_default();
        let palette = settings.theme.palette( cc.egui_ctx.system_theme() );
        theme::apply_palette( &cc.egui_ctx, palette );
        egui_extras::install_image_loaders( &cc.egui_ctx );
        cc.egui_ctx.set_zoom_factor( settings.ui_scale );
        i18n::apply( settings.language );
        let start = start.or_else( || settings.game_data.clone() );

        let mut app = Self
        {
            palette,
            title_bar:     TitleBar::new( &cc.egui_ctx ),
            library:       Library::default(),
            expanded:      HashSet::new(),
            collapsed:     HashSet::new(),
            filter:        String::new(),
            rows:          Vec::new(),
            rows_dirty:    true,
            settings,
            preferences:   None,
            thumbnails:    HashMap::new(),
            tabs:          Vec::new(),
            active:        None,
            tabs_overflow: false,
            revealed_tab:  None,
            sidebar_width: SIDEBAR_WIDTH,
            scroll_to_row: None,
            list_offset:   0.0,
            list_height:   0.0,
            checker:       texture_viewer::new_checker( &cc.egui_ctx ),
            render_state:  cc.wgpu_render_state.clone(),
            message:       None,
            frame_end:     None,
        };

        if let Some( path ) = start
        {
            if path.is_dir()
            {
                app.on_open_folder( &path );
            }
            else
            {
                app.on_open_archives( &[ path ] );
            }
        }

        app
    }

    fn on_open_folder( &mut self, dir: &Path )
    {
        let ( library, errors ) = Library::open_folder( dir );
        self.message = match ( errors.is_empty(), library.archives.is_empty() )
        {
            ( false, _ ) => Some( errors.join( "; " ) ),
            ( true, true ) => Some( t!( "status.no_archives", path = dir.display() ).into_owned() ),
            ( true, false ) => None,
        };
        self.set_library( library );
    }

    /// Adds the archives to the open ones and expands them.
    fn on_open_archives( &mut self, paths: &[PathBuf] )
    {
        let mut errors = Vec::new();

        for path in paths
        {
            match self.library.add_archive( path )
            {
                Ok( archive ) =>
                {
                    self.expanded.insert( NodeKey { archive, folder: String::new() } );
                }
                Err( err ) => errors.push( err ),
            }
        }
        self.message = ( !errors.is_empty() ).then( || errors.join( "; " ) );
        self.rows_dirty = true;
    }

    /// Closes the archive and its tabs; keys of the archives after it move down by one.
    fn on_close_archive( &mut self, index: usize )
    {
        if index >= self.library.archives.len()
        {
            return;
        }

        let active = self.active_tab().map( |tab| tab.key );
        self.library.remove( index );

        let shift = |archive: usize| match archive.cmp( &index )
        {
            std::cmp::Ordering::Less => Some( archive ),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some( archive - 1 ),
        };
        let shift_key = |key: FileKey| shift( key.archive ).map( |archive| FileKey { archive, ..key } );

        self.tabs.retain_mut(
            |tab|
            {
                let Some( key ) = shift_key( tab.key ) else { return false };
                tab.key = key;

                true
            }
        );
        let active = active.and_then( shift_key ).and_then( |key| self.tabs.iter().position( |tab| tab.key == key ) );
        self.active = active.or( ( !self.tabs.is_empty() ).then_some( 0 ) );

        self.thumbnails = std::mem::take( &mut self.thumbnails ).into_iter().filter_map( |( key, thumbnail )| Some( ( shift_key( key )?, thumbnail ) ) ).collect();
        self.expanded = std::mem::take( &mut self.expanded ).into_iter().filter_map( |node| Some( NodeKey { archive: shift( node.archive )?, ..node } ) ).collect();
        self.collapsed = std::mem::take( &mut self.collapsed ).into_iter().filter_map( |node| Some( NodeKey { archive: shift( node.archive )?, ..node } ) ).collect();
        self.rows_dirty = true;
    }

    fn set_library( &mut self, library: Library )
    {
        self.library = library;
        self.expanded.clear();
        self.collapsed.clear();
        self.thumbnails.clear();
        self.tabs.clear();
        self.active = None;
        self.rows_dirty = true;
    }

    /// A cleared search forgets what was collapsed in it.
    fn on_filter_change( &mut self )
    {
        if self.filter.is_empty()
        {
            self.collapsed.clear();
        }
        self.rows_dirty = true;
    }

    /// While searching, a toggle collapses or reopens the folder; otherwise it expands or collapses it.
    fn on_toggle( &mut self, node: NodeKey )
    {
        let set = if self.filter.is_empty() { &mut self.expanded } else { &mut self.collapsed };
        if !set.remove( &node )
        {
            set.insert( node );
        }
        self.rows_dirty = true;
    }

    fn on_open_file( &mut self, ctx: &egui::Context, key: FileKey, pin: bool )
    {
        if let Some( index ) = self.tabs.iter().position( |tab| tab.key == key )
        {
            self.active = Some( index );
            self.tabs[ index ].pinned |= pin;
            return;
        }

        let Some( ( archive, entry ) ) = self.library.entry( key ) else { return };
        let data = match archive.vfs.read_entry( entry )
        {
            Ok( data ) => data,
            Err( err ) =>
            {
                self.message = Some( format!( "{}: {err}", entry.path ) );
                return;
            }
        };
        let path      = entry.path.clone();
        let library   = &self.library;
        let find_file = |name: &str| library.read_by_name( name );
        let archives  = library.paths();
        let resources = Resources
        {
            render_state:      self.render_state.as_ref(),
            find_file:         &find_file,
            hex_bytes_per_row: self.settings.hex_bytes_per_row,
            archives:          &archives,
        };
        let ( viewer, error ) = Viewer::new( ctx, &path, &data, &resources );
        self.message = error.map( |err| format!( "{path}: {err}" ) );

        let tab = Tab { key, name: file_name( &path ).to_owned(), data, viewer, pinned: pin };
        let index = match self.tabs.iter().position( |tab| !tab.pinned )
        {
            Some( index ) =>
            {
                self.tabs[ index ] = tab;
                index
            }
            None =>
            {
                let index = self.active.map_or( self.tabs.len(), |active| active + 1 );
                self.tabs.insert( index, tab );
                index
            }
        };
        self.active = Some( index );
    }

    fn on_close_tab( &mut self, index: usize )
    {
        if index >= self.tabs.len()
        {
            return;
        }
        self.tabs.remove( index );
        self.active = match self.active
        {
            _ if self.tabs.is_empty() => None,
            Some( active ) if active > index || active == self.tabs.len() => Some( active - 1 ),
            active => active,
        };
    }

    fn on_step_selection( &mut self, ctx: &egui::Context, forward: bool )
    {
        let files: Vec<(usize, FileKey)> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(
                |( index, row )| match row.kind
                {
                    RowKind::File( key, _ ) => Some( ( index, key ) ),
                    _ => None,
                }
            )
            .collect();
        let current  = self.active_tab().map( |tab| tab.key );
        let position = files.iter().position( |&( _, key )| Some( key ) == current );
        let next = match ( position, forward )
        {
            ( None, _ ) => 0,
            ( Some( position ), true ) => ( position + 1 ).min( files.len().saturating_sub( 1 ) ),
            ( Some( position ), false ) => position.saturating_sub( 1 ),
        };
        if let Some( &( row, key ) ) = files.get( next )
            && Some( key ) != current
        {
            self.scroll_to_row = Some( row );
            self.on_open_file( ctx, key, false );
        }
    }

    fn on_close_others( &mut self, index: usize )
    {
        if index >= self.tabs.len()
        {
            return;
        }
        let tab = self.tabs.swap_remove( index );
        self.tabs = vec![ tab ];
        self.active = Some( 0 );
    }

    /// PNG exports what the tab shows, with its channel toggles.
    fn on_export_tab( &mut self, index: usize, export: Export )
    {
        let Some( tab ) = self.tabs.get( index ) else { return };
        let message = match ( export, &tab.viewer )
        {
            ( Export::Raw, _ ) => save_raw_as( &tab.name, &tab.data ),
            ( Export::Png, Viewer::Texture( viewer ) ) => save_png_as( &tab.name, |target| viewer.export_png( target ) ),
            ( Export::Png, _ ) => None,
            ( Export::Gltf, _ ) => save_gltf_as( &tab.name, &tab.data, &self.library ),
        };
        if message.is_some()
        {
            self.message = message;
        }
    }

    fn on_export_file( &mut self, key: FileKey, export: Export )
    {
        let Some( ( archive, entry ) ) = self.library.entry( key ) else { return };
        let data = match archive.vfs.read_entry( entry )
        {
            Ok( data ) => data,
            Err( err ) =>
            {
                self.message = Some( format!( "{}: {err}", entry.path ) );
                return;
            }
        };
        let name    = file_name( &entry.path );
        let message = match export
        {
            Export::Raw => save_raw_as( name, &data ),
            Export::Png => save_png_as(
                name,
                |target|
                {
                    let image = texture_viewer::decode( &entry.path, &data )?;
                    texture_viewer::save_png( &image.rgba, image.width, image.height, target )
                }
            ),
            Export::Gltf => save_gltf_as( name, &data, &self.library ),
        };
        if message.is_some()
        {
            self.message = message;
        }
    }

    fn active_tab( &self ) -> Option<&Tab>
    {
        self.active.and_then( |index| self.tabs.get( index ) )
    }

    fn menu( &mut self, ui: &mut egui::Ui )
    {
        widgets::menu_visuals( ui, self.palette );

        egui::MenuBar::new().style( title_menu_style ).ui(
            ui,
            |ui|
            {
                ui.menu_button(
                    t!( "menu.file" ),
                    |ui|
                    {
                        if ui.button( t!( "menu.open_folder" ) ).clicked()
                            && let Some( dir ) = rfd::FileDialog::new().pick_folder()
                        {
                            self.on_open_folder( &dir );
                        }
                        if ui.button( t!( "menu.open_archive" ) ).clicked()
                            && let Some( paths ) = rfd::FileDialog::new().add_filter( "VFS", &[ morvfs::EXTENSION ] ).pick_files()
                        {
                            self.on_open_archives( &paths );
                        }
                        ui.separator();
                        if ui.button( t!( "menu.preferences" ) ).clicked()
                        {
                            self.preferences = Some( Page::General );
                        }
                        ui.separator();
                        if ui.button( t!( "menu.exit" ) ).clicked()
                        {
                            ui.ctx().send_viewport_cmd( egui::ViewportCommand::Close );
                        }
                    }
                );
                ui.menu_button(
                    t!( "menu.view" ),
                    |ui|
                    {
                        ui.radio_value( &mut self.settings.sidebar, SidebarLayout::List, t!( "menu.list" ) );
                        ui.radio_value( &mut self.settings.sidebar, SidebarLayout::Grid, t!( "menu.thumbnails" ) );
                    }
                );
            }
        );
    }

    /// Returns the top of the file list, so the main panel lines up with it.
    fn sidebar( &mut self, ui: &mut egui::Ui ) -> f32
    {
        let palette = self.palette;
        let rect    = ui.max_rect();

        let search = widgets::search( ui, palette, &mut self.filter, &t!( "sidebar.search" ) );
        if search.changed()
        {
            self.on_filter_change();
        }

        let list = Rect::from_min_max( Pos2::new( rect.min.x, search.rect.max.y + SEARCH_GAP ), rect.max );
        ui.painter().rect_stroke( list, CornerRadius::same( theme::PANEL_RADIUS ), Stroke::new( 1.0, palette.border ), StrokeKind::Inside );
        let inner  = list.shrink( 1.0 + SIDEBAR_PADDING );
        let mut ui = ui.new_child( UiBuilder::new().max_rect( inner ).layout( Layout::top_down( Align::Min ) ) );
        ui.set_clip_rect( inner );
        ui.spacing_mut().item_spacing = Vec2::new( 0.0, ROW_GAP );

        if self.rows_dirty
        {
            self.rows = self.library.rows( &self.expanded, &self.collapsed, &self.filter );
            self.rows_dirty = false;
        }
        let actions = match self.settings.sidebar
        {
            SidebarLayout::List => self.tree_list( &mut ui ),
            SidebarLayout::Grid => self.tree_grid( &mut ui ),
        };
        for action in actions
        {
            match action
            {
                Action::Toggle( node ) => self.on_toggle( node ),
                Action::Open( key, pin ) => self.on_open_file( ui.ctx(), key, pin ),
                Action::Export( key, export ) => self.on_export_file( key, export ),
                Action::CloseArchive( archive ) => self.on_close_archive( archive ),
            }
        }

        list.min.y
    }

    fn tree_list( &mut self, ui: &mut egui::Ui ) -> Vec<Action>
    {
        let palette  = self.palette;
        let current  = self.active_tab().map( |tab| tab.key );
        let mut area = ScrollArea::vertical().id_salt( "tree" ).auto_shrink( false );

        if let Some( row ) = self.scroll_to_row.take()
        {
            let top = to_f32( row ) * ( ROW_HEIGHT + ROW_GAP );
            if top < self.list_offset
            {
                area = area.vertical_scroll_offset( top );
            }
            else if top + ROW_HEIGHT > self.list_offset + self.list_height
            {
                area = area.vertical_scroll_offset( top + ROW_HEIGHT - self.list_height );
            }
        }

        let mut actions = Vec::new();
        let output = area.show_rows(
            ui,
            ROW_HEIGHT,
            self.rows.len(),
            |ui, range|
            {
                for row in &self.rows[ range ]
                {
                    let indent = widgets::row_indent( row.depth );
                    let response = match row.kind
                    {
                        RowKind::Archive | RowKind::Folder =>
                        {
                            widgets::tree_row( ui, palette, indent, RowIcon::Chevron( row.expanded ), &row.label, row.depth == 0, false )
                        }
                        RowKind::File( key, _ ) =>
                        {
                            widgets::tree_row( ui, palette, indent, RowIcon::Image( file_icon( &row.label ) ), &row.label, false, current == Some( key ) )
                        }
                    };
                    actions.extend( row_action( row, &response ) );
                    actions.extend( file_menu( &response, palette, row ) );
                    actions.extend( archive_menu( &response, palette, row ) );
                }
            }
        );
        self.list_offset = output.state.clamped_offset().y;
        self.list_height = output.inner_rect.height();

        actions
    }

    fn tree_grid( &mut self, ui: &mut egui::Ui ) -> Vec<Action>
    {
        let palette = self.palette;
        let current = self.active_tab().map( |tab| tab.key );
        let width   = ui.available_width();
        let columns = ( ( width + CARD_GAP ) / ( CARD_SIZE.x + CARD_GAP ) ).floor().max( 1.0 );

        // Taken out while drawing so thumbnails can be cached on `self`.
        let rows = std::mem::take( &mut self.rows );
        let mut lines: Vec<GridLine> = Vec::new();
        for row in &rows
        {
            match row.kind
            {
                RowKind::Archive | RowKind::Folder => lines.push( GridLine::Header( row ) ),
                RowKind::File( .. ) => match lines.last_mut()
                {
                    Some( GridLine::Cards( cards ) ) if to_f32( cards.len() ) < columns && cards[ 0 ].depth == row.depth => cards.push( row ),
                    _ => lines.push( GridLine::Cards( vec![ row ] ) ),
                },
            }
        }

        let mut budget       = THUMBS_PER_FRAME;
        let mut model_budget = MODEL_THUMBS_PER_FRAME;
        let mut pending      = false;
        let mut actions      = Vec::new();
        let ctx              = ui.ctx().clone();

        ScrollArea::vertical().id_salt( "grid" ).auto_shrink( false ).show_viewport(
            ui,
            |ui, viewport|
            {
                let origin = ui.max_rect().min;
                let mut y  = 0.0;

                for line in &lines
                {
                    let height = match line
                    {
                        GridLine::Header( _ ) => ROW_HEIGHT,
                        GridLine::Cards( _ ) => CARD_SIZE.y,
                    };
                    let line_rect = Rect::from_min_size( origin + Vec2::new( 0.0, y ), Vec2::new( width, height ) );

                    y += height + ROW_GAP;
                    if line_rect.max.y < origin.y + viewport.min.y || line_rect.min.y > origin.y + viewport.max.y
                    {
                        continue;
                    }

                    let mut child = ui.new_child( UiBuilder::new().max_rect( line_rect ).layout( Layout::left_to_right( Align::Min ) ) );

                    match line
                    {
                        GridLine::Header( row ) =>
                        {
                            let indent   = widgets::row_indent( row.depth );
                            let response = widgets::tree_row( &mut child, palette, indent, RowIcon::Chevron( row.expanded ), &row.label, row.depth == 0, false );
                            actions.extend( row_action( row, &response ) );
                            actions.extend( archive_menu( &response, palette, row ) );
                        }
                        GridLine::Cards( cards ) =>
                        {
                            child.add_space( 20.0 * to_f32( cards[ 0 ].depth.saturating_sub( 1 ) ) );
                            child.spacing_mut().item_spacing.x = CARD_GAP;
                            for row in cards
                            {
                                let RowKind::File( key, kind ) = row.kind else { continue };
                                let wanted = match kind
                                {
                                    Kind::Texture => Some( &mut budget ),
                                    Kind::Model if self.render_state.is_some() => Some( &mut model_budget ),
                                    _ => None,
                                };
                                if let Some( left ) = wanted
                                    && !self.thumbnails.contains_key( &key )
                                {
                                    if *left > 0
                                    {
                                        *left -= 1;
                                        let thumb = self.load_thumbnail( &ctx, key, kind );
                                        self.thumbnails.insert( key, thumb );
                                    }
                                    else
                                    {
                                        pending = true;
                                    }
                                }
                                let thumb    = self.thumbnails.get( &key ).and_then( Option::as_ref );
                                let response = card( &mut child, palette, &row.label, thumb, current == Some( key ) );
                                actions.extend( row_action( row, &response ) );
                                actions.extend( file_menu( &response, palette, row ) );
                            }
                        }
                    }
                }
                ui.allocate_space( Vec2::new( width, y ) );
            }
        );

        drop( lines );
        self.rows = rows;
        if pending
        {
            ctx.request_repaint();
        }

        actions
    }

    fn load_thumbnail( &self, ctx: &egui::Context, key: FileKey, kind: Kind ) -> Option<Thumbnail>
    {
        let ( archive, entry ) = self.library.entry( key )?;
        let data = archive.vfs.read_entry( entry ).ok()?;

        match kind
        {
            Kind::Texture => texture_viewer::thumbnail( ctx, &entry.path, &data, 64 ).map( Thumbnail::Image ),
            Kind::Model if crate::preview::has_extension( &entry.path, mormesh::EXTENSION ) =>
            {
                let state     = self.render_state.as_ref()?;
                let find_file = |name: &str| self.library.read_by_name( name );
                model_viewer::thumbnail( state, &data, &find_file, MODEL_THUMB_PIXELS ).map( Thumbnail::Model )
            }
            _ => None,
        }
    }

    /// Tabs share the search row; the panel below starts at `panel_top` whether tabs exist or not.
    fn main_view( &mut self, ui: &mut egui::Ui, panel_top: f32 )
    {
        let palette = self.palette;
        let rect    = ui.max_rect();
        let panel   = Rect::from_min_max( Pos2::new( rect.min.x, panel_top ), rect.max );
        ui.painter().rect( panel, CornerRadius::same( theme::PANEL_RADIUS ), palette.view, Stroke::new( 1.0, palette.border ), StrokeKind::Inside );

        let tabs_top  = rect.min.y + ( ( panel_top - SEARCH_GAP - rect.min.y - TABS_HEIGHT ) / 2.0 ).max( 0.0 );
        let arrows    = if self.tabs_overflow { 2.0 * ( TABS_HEIGHT + MAIN_PADDING ) } else { 0.0 };
        let tabs_rect = Rect::from_min_size( Pos2::new( rect.min.x, tabs_top ), Vec2::new( rect.width() - arrows, TABS_HEIGHT ) );
        let body      = panel.shrink( 1.0 + MAIN_PADDING );

        let mut scroll = None;

        if self.tabs_overflow
        {
            let next = Rect::from_min_size( Pos2::new( rect.max.x - TABS_HEIGHT, tabs_top ), Vec2::splat( TABS_HEIGHT ) );
            let prev = next.translate( Vec2::new( -( TABS_HEIGHT + MAIN_PADDING ), 0.0 ) );
            if widgets::scroll_button( ui, palette, prev, true ).clicked()
            {
                scroll = Some( TABS_SCROLL_STEP );
            }
            if widgets::scroll_button( ui, palette, next, false ).clicked()
            {
                scroll = Some( -TABS_SCROLL_STEP );
            }
        }

        let mut tabs_ui = ui.new_child( UiBuilder::new().max_rect( tabs_rect ).layout( Layout::left_to_right( Align::Center ) ) );
        tabs_ui.set_clip_rect( tabs_rect );
        let strip  = ( self.active, self.tabs.len(), self.tabs_overflow );
        let reveal = self.revealed_tab != Some( strip );

        let mut action = None;
        let tabs_area  = ScrollArea::horizontal().id_salt( "tabs" ).scroll_bar_visibility( egui::scroll_area::ScrollBarVisibility::AlwaysHidden );
        let tabs_output = tabs_area.show(
            &mut tabs_ui,
            |ui|
            {
                if let Some( delta ) = scroll
                {
                    ui.scroll_with_delta( Vec2::new( delta, 0.0 ) );
                }
                ui.spacing_mut().item_spacing.x = MAIN_PADDING;
                for ( index, tab ) in self.tabs.iter().enumerate()
                {
                    let close    = |painter: &egui::Painter, center: Pos2, color| self.title_bar.paint_close( painter, center, widgets::ICON_SIZE, color );
                    let response = widgets::tab( ui, palette, file_icon( &tab.name ), &tab.name, self.active == Some( index ), !tab.pinned, close );
                    if reveal && self.active == Some( index )
                    {
                        ui.scroll_to_rect( response.response.rect, None );
                    }

                    if let Some( menu_action ) = tab_menu( &response.response, palette, index, &tab.name )
                    {
                        action = Some( menu_action );
                    }
                    else if response.closed
                    {
                        action = Some( TabAction::Close( index ) );
                    }
                    else if response.double_clicked
                    {
                        action = Some( TabAction::Pin( index ) );
                    }
                    else if response.clicked
                    {
                        action = Some( TabAction::Activate( index ) );
                    }
                }
            }
        );
        self.revealed_tab = Some( strip );

        let overflow = tabs_output.content_size.x > rect.width();
        if overflow != self.tabs_overflow
        {
            self.tabs_overflow = overflow;
            ui.ctx().request_repaint();
        }

        match action
        {
            Some( TabAction::Activate( index ) ) => self.active = Some( index ),
            Some( TabAction::Pin( index ) ) =>
            {
                self.active = Some( index );
                self.tabs[ index ].pinned = true;
            }
            Some( TabAction::Close( index ) ) => self.on_close_tab( index ),
            Some( TabAction::CloseOthers( index ) ) => self.on_close_others( index ),
            Some( TabAction::Export( index, export ) ) => self.on_export_tab( index, export ),
            None => {}
        }

        let mut body_ui = ui.new_child( UiBuilder::new().max_rect( body ) );
        body_ui.set_clip_rect( body );
        let checker = &self.checker;
        let error = match self.active.and_then( |index| self.tabs.get_mut( index ) )
        {
            Some( tab ) => tab.viewer.show( &mut body_ui, palette, checker, &tab.data, &self.settings.viewer ),
            None =>
            {
                let hint = if self.library.archives.is_empty() { t!( "blank.open_folder" ) } else { t!( "blank.select_file" ) };
                viewer::show_blank( &mut body_ui, palette, &hint );
                None
            }
        };
        if error.is_some()
        {
            self.message = error;
        }
    }

    fn status_bar( &self, ui: &mut egui::Ui )
    {
        let palette = self.palette;

        ui.spacing_mut().item_spacing.x = 4.0;
        let text = |ui: &mut egui::Ui, text: &str|
        {
            ui.label( egui::RichText::new( text ).size( theme::STATUS_TEXT_SIZE ).color( palette.text ) );
        };

        let mut segments: Vec<String> = Vec::new();

        match self.active_tab().and_then( |tab| self.library.entry( tab.key ).map( |( archive, _ )| ( tab, archive ) ) )
        {
            Some( ( tab, archive ) ) =>
            {
                segments.push( archive.name.clone() );
                segments.push( tab.name.clone() );
                segments.push( i18n::count( "status.bytes", tab.data.len() ) );
            }
            None if !self.library.archives.is_empty() => segments.push( i18n::count( "status.archives", self.library.archives.len() ) ),
            None => {}
        }
        segments.extend( self.message.clone() );
        for ( index, segment ) in segments.iter().enumerate()
        {
            if index > 0
            {
                widgets::status_separator( ui, palette );
            }
            text( ui, segment );
        }
    }

    fn preferences( &mut self, ctx: &egui::Context )
    {
        let Some( mut page ) = self.preferences else { return };
        let renderer = match &self.render_state
        {
            Some( state ) =>
            {
                let info = state.adapter.get_info();
                format!( "{} ({:?})", info.name, info.backend )
            }
            None => "OpenGL".to_owned(),
        };
        let language  = self.settings.language;
        let game_data = self.settings.game_data.clone();
        let open      = settings::show( ctx, self.palette, &mut self.settings, &mut page, &AboutInfo { renderer } );
        self.preferences = open.then_some( page );
        if self.settings.language != language
        {
            i18n::apply( self.settings.language );
        }
        if self.settings.game_data != game_data
            && let Some( dir ) = self.settings.game_data.clone()
        {
            self.on_open_folder( &dir );
        }
        if ctx.zoom_factor() != self.settings.ui_scale
        {
            ctx.set_zoom_factor( self.settings.ui_scale );
        }
    }

    /// The surface keeps its other parameters; the present mode follows the vsync setting.
    fn apply_vsync( &self, frame: &mut eframe::Frame )
    {
        let Some( mut config ) = frame.wgpu_surface_config() else { return };
        config.present_mode = if self.settings.viewer.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        frame.set_wgpu_surface_config( config );
    }

    /// Without vsync, waits out what is left of the frame the FPS limit allows.
    fn limit_fps( &mut self )
    {
        let viewer = self.settings.viewer;

        if let Some( last ) = self.frame_end.filter( |_| !viewer.vsync )
        {
            let frame = Duration::from_secs_f64( 1.0 / f64::from( viewer.fps_limit.max( 1 ) ) );
            if let Some( rest ) = frame.checked_sub( last.elapsed() )
            {
                std::thread::sleep( rest );
            }
        }
        self.frame_end = Some( Instant::now() );
    }

    /// Applies a theme picked in the settings or, for the system one, changed in the OS.
    fn apply_theme( &mut self, ctx: &egui::Context )
    {
        let palette = self.settings.theme.palette( ctx.system_theme() );
        if palette.theme != self.palette.theme
        {
            theme::apply_palette( ctx, palette );
            self.palette = palette;
            ctx.request_repaint();
        }
    }

    fn window_border( &self, ctx: &egui::Context )
    {
        let expanded = ctx.input( |input| input.viewport().maximized.unwrap_or( false ) || input.viewport().fullscreen.unwrap_or( false ) );
        // Windows draws its own 1 px frame with rounded corners around the borderless window, macOS keeps a native one.
        if expanded || cfg!( any( windows, target_os = "macos" ) )
        {
            return;
        }
        let painter = ctx.layer_painter( egui::LayerId::new( egui::Order::Foreground, egui::Id::unique( "window_border" ) ) );
        // One physical pixel: the design's 1.3 smears over two.
        let width = 1.0 / ctx.pixels_per_point();
        painter.rect_stroke( ctx.viewport_rect(), 0.0, Stroke::new( width, self.palette.window_border ), StrokeKind::Inside );
    }

    fn shortcuts( &mut self, ctx: &egui::Context )
    {
        if ctx.memory( |memory| memory.focused().is_some() )
        {
            return;
        }
        let ( up, down, close, preferences ) = ctx.input(
            |input|
            {
                let command = input.modifiers.command;
                ( input.key_pressed( Key::ArrowUp ), input.key_pressed( Key::ArrowDown ), command && input.key_pressed( Key::W ), command && input.key_pressed( Key::Comma ) )
            }
        );

        if preferences
        {
            self.preferences = Some( Page::General );
        }
        if up || down
        {
            self.on_step_selection( ctx, down );
        }
        if close && let Some( index ) = self.active
        {
            self.on_close_tab( index );
        }
    }
}

/// egui's menu style with the title bar buttons of the design: padding 4 by 2, 4 apart.
fn title_menu_style( style: &mut egui::Style )
{
    egui::containers::menu::menu_style( style );
    style.spacing.button_padding = Vec2::new( 4.0, 2.0 );
    style.spacing.item_spacing.x = 4.0;
}

/// `None` when the dialog is cancelled.
fn save_raw_as( name: &str, data: &[u8] ) -> Option<String>
{
    let target = rfd::FileDialog::new().set_file_name( name ).save_file()?;
    let result = std::fs::write( &target, data ).map_err( |err| err.to_string() );

    Some( saved_message( &target, result ) )
}

/// `None` when the dialog is cancelled.
fn save_png_as( name: &str, export: impl FnOnce( &Path ) -> Result<(), String> ) -> Option<String>
{
    let stem   = name.rsplit_once( '.' ).map_or( name, |( stem, _ )| stem );
    let dialog = rfd::FileDialog::new().add_filter( "PNG", &[ "png" ] ).set_file_name( format!( "{stem}.png" ) );
    let target = dialog.save_file()?;
    let result = export( &target );

    Some( saved_message( &target, result ) )
}

fn saved_message( target: &Path, result: Result<(), String> ) -> String
{
    match result
    {
        Ok( () ) => t!( "status.saved", path = target.display() ).into_owned(),
        Err( err ) => format!( "{}: {err}", target.display() ),
    }
}

/// `None` when the dialog is cancelled. `.gltf` or `.glb` is picked by the extension the user saves with.
/// Meshes convert from `data`; scenes read their actors, meshes and textures from the open archives.
fn save_gltf_as( name: &str, data: &[u8], library: &Library ) -> Option<String>
{
    let stem   = name.rsplit_once( '.' ).map_or( name, |( stem, _ )| stem );
    let dialog = rfd::FileDialog::new()
        .add_filter( "glTF binary", &[ "glb" ] )
        .add_filter( "glTF", &[ "gltf" ] )
        .set_file_name( format!( "{stem}.glb" ) );
    let target = dialog.save_file()?;
    let read   = |file: &str| library.read_by_name( file ).map( |( _, data )| data );
    let png    = |texture: &str|
    {
        let ( path, data ) = library.read_by_name( texture )?;
        let image = texture_viewer::decode( &path, &data ).ok()?;

        texture_viewer::encode_png( &image ).ok()
    };

    let exported = match crate::preview::kind_of( name )
    {
        Kind::Scene => Scene::load( name, read ).map( |scene| morscene::gltf::export( &scene, stem, read, read, png ).gltf ).map_err( |err| err.to_string() ),
        _ => Mesh::parse( data ).map( |mesh| Gltf::from_mesh( &mesh, stem, png ) ).map_err( |err| err.to_string() ),
    };
    let result = exported.and_then( |gltf| gltf::save( &gltf, &target ) );

    Some( saved_message( &target, result ) )
}

/// Raw bytes for every file, plus the conversion its type has.
fn export_items( ui: &mut egui::Ui, name: &str ) -> Option<Export>
{
    let mut chosen = None;

    for export in std::iter::once( Export::Raw ).chain( Export::conversion( name ) )
    {
        if ui.button( t!( export.label() ) ).clicked()
        {
            chosen = Some( export );
        }
    }

    chosen
}

fn archive_menu( response: &egui::Response, palette: &Palette, row: &Row ) -> Option<Action>
{
    let ( RowKind::Archive, Some( node ) ) = ( &row.kind, &row.node ) else { return None };
    let mut action = None;

    response.context_menu(
        |ui|
        {
            widgets::menu_visuals( ui, palette );
            if ui.button( t!( "context.close_archive" ) ).clicked()
            {
                action = Some( Action::CloseArchive( node.archive ) );
            }
        }
    );

    action
}

fn file_menu( response: &egui::Response, palette: &Palette, row: &Row ) -> Option<Action>
{
    let RowKind::File( key, _ ) = row.kind else { return None };
    let mut action = None;

    response.context_menu(
        |ui|
        {
            widgets::menu_visuals( ui, palette );
            if ui.button( t!( "context.open" ) ).clicked()
            {
                action = Some( Action::Open( key, true ) );
            }
            ui.separator();
            if let Some( export ) = export_items( ui, &row.label )
            {
                action = Some( Action::Export( key, export ) );
            }
        }
    );

    action
}

fn tab_menu( response: &egui::Response, palette: &Palette, index: usize, name: &str ) -> Option<TabAction>
{
    let mut action = None;

    response.context_menu(
        |ui|
        {
            widgets::menu_visuals( ui, palette );
            if ui.button( t!( "context.close" ) ).clicked()
            {
                action = Some( TabAction::Close( index ) );
            }
            if ui.button( t!( "context.close_others" ) ).clicked()
            {
                action = Some( TabAction::CloseOthers( index ) );
            }
            ui.separator();
            if let Some( export ) = export_items( ui, name )
            {
                action = Some( TabAction::Export( index, export ) );
            }
        }
    );

    action
}

fn file_icon( name: &str ) -> egui::ImageSource<'static>
{
    match crate::preview::kind_of( name )
    {
        Kind::Texture | Kind::Map => icons::TEXTURE,
        Kind::Font => icons::FONT,
        Kind::Text => icons::TEXT,
        Kind::Scene if crate::preview::has_extension( name, morscene::OUTDOOR_EXTENSION ) => icons::WORLD,
        Kind::Scene => icons::INTERIOR,
        Kind::Model => icons::MESH,
        Kind::Binary if crate::preview::has_extension( name, crate::preview::PATH_EXTENSION ) => icons::PATH,
        Kind::Audio => icons::AUDIO,
        Kind::Binary => icons::BINARY,
    }
}

fn row_action( row: &Row, response: &egui::Response ) -> Option<Action>
{
    match ( &row.kind, &row.node )
    {
        ( RowKind::File( key, _ ), _ ) if response.double_clicked() => Some( Action::Open( *key, true ) ),
        ( RowKind::File( key, _ ), _ ) if response.clicked() => Some( Action::Open( *key, false ) ),
        ( _, Some( node ) ) if response.clicked() => Some( Action::Toggle( node.clone() ) ),
        _ => None,
    }
}

/// Thumbnail card: preview or type icon, file name below in up to two lines.
fn card( ui: &mut egui::Ui, palette: &Palette, label: &str, thumb: Option<&Thumbnail>, selected: bool ) -> egui::Response
{
    let ( rect, response ) = ui.allocate_exact_size( CARD_SIZE, Sense::click() );
    let painter = ui.painter();
    if selected || response.hovered()
    {
        painter.rect_filled( rect, CornerRadius::same( theme::CONTROL_RADIUS ), palette.border );
    }

    let slot = Rect::from_min_size( rect.min + Vec2::splat( 4.0 ), Vec2::splat( THUMB_SIZE ) );

    match thumb
    {
        Some( thumb ) =>
        {
            let size    = thumb.size();
            let fitted  = size * ( THUMB_SIZE / size.x.max( size.y ) );
            let uv      = Rect::from_min_max( Pos2::ZERO, Pos2::new( 1.0, 1.0 ) );
            let preview = egui::epaint::RectShape::filled( Rect::from_center_size( slot.center(), fitted ), CornerRadius::same( 2 ), egui::Color32::WHITE );
            painter.add( preview.with_texture( thumb.id(), uv ) );
        }
        None => widgets::paint_icon( ui, file_icon( label ), Rect::from_center_size( slot.center(), Vec2::splat( 20.0 ) ), palette.text_weak ),
    }

    let font    = FontId::proportional( theme::CAPTION_TEXT_SIZE );
    let mut top = slot.max.y + 4.0;
    for line in widgets::card_lines( painter, label, &font, CARD_SIZE.x - 4.0 )
    {
        let galley = painter.layout_no_wrap( line, font.clone(), palette.text );
        let height = galley.size().y;
        painter.galley( Pos2::new( rect.center().x - ( galley.size().x / 2.0 ), top ), galley, palette.text );
        top += height;
    }

    response.on_hover_text( label )
}

impl eframe::App for ViewerApp
{
    fn save( &mut self, storage: &mut dyn eframe::Storage )
    {
        eframe::set_value( storage, settings::STORAGE_KEY, &self.settings );
    }

    fn ui( &mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame )
    {
        self.apply_vsync( frame );
        let ctx = ui.ctx().clone();
        self.apply_theme( &ctx );

        let palette = self.palette;
        self.shortcuts( &ctx );

        egui::Panel::top( "title_bar" )
            .exact_size( title_bar::HEIGHT )
            .show_separator_line( false )
            .frame( egui::Frame::NONE.fill( palette.window ) )
            .show(
                ui,
                |ui|
                {
                    let mut row = self.title_bar.show( ui );
                    self.menu( &mut row );
                }
            );
        egui::Panel::bottom( "status" )
            .exact_size( STATUS_HEIGHT )
            .show_separator_line( false )
            .frame( egui::Frame::NONE.fill( palette.window ).inner_margin( egui::Margin::symmetric( 8, 0 ) ) )
            .show( ui, |ui| ui.horizontal_centered( |ui| self.status_bar( ui ) ) );
        egui::CentralPanel::default()
            .frame( egui::Frame::NONE.fill( palette.window ).inner_margin( egui::Margin::symmetric( 4, 0 ) ) )
            .show(
                ui,
                |ui|
                {
                    let rect = ui.max_rect();
                    self.sidebar_width = self.sidebar_width.clamp( SIDEBAR_MIN, ( rect.width() * 0.6 ).max( SIDEBAR_MIN ) );
                    let sidebar  = Rect::from_min_size( rect.min, Vec2::new( self.sidebar_width, rect.height() ) );
                    let splitter = Rect::from_min_max( Pos2::new( sidebar.max.x, rect.min.y ), Pos2::new( sidebar.max.x + GUTTER, rect.max.y ) );
                    let main     = Rect::from_min_max( Pos2::new( splitter.max.x, rect.min.y ), rect.max );

                    let drag = ui.interact( splitter, ui.scope_id().with( "splitter" ), Sense::drag() );
                    if drag.hovered() || drag.dragged()
                    {
                        ui.ctx().set_cursor_icon( CursorIcon::ResizeHorizontal );
                    }
                    self.sidebar_width += drag.drag_delta().x;

                    let panel_top = self.sidebar( &mut ui.new_child( UiBuilder::new().max_rect( sidebar ) ) );
                    self.main_view( &mut ui.new_child( UiBuilder::new().max_rect( main ) ), panel_top );
                }
            );

        self.preferences( &ctx );
        self.window_border( &ctx );
        title_bar::resize_edges( &ctx );
        self.limit_fps();
    }
}
