use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui_wgpu::wgpu;
use eframe::egui::{ self, Align, Color32, CornerRadius, FontId, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, UiBuilder, Vec2 };
use serde::{ Deserialize, Serialize };

use rust_i18n::t;
use crate::i18n::{ self, Language };
use crate::theme::{ self, Palette, Theme };
use crate::widgets;

pub const STORAGE_KEY: &str = "settings";

/// eframe's store in its storage directory.
const STORAGE_FILE: &str = "app.ron";

const FPS_LIMITS: std::ops::RangeInclusive<u32> = 30..=480;

const MODAL_SIZE:      Vec2       = Vec2::new( 720.0, 510.0 );
const NAV_WIDTH:       f32        = 186.0;
const NAV_PADDING:     f32        = 18.0;
const NAV_ROW:         f32        = 39.0;
const CONTENT_PADDING: f32        = 28.0;
const ROW_HEIGHT:      f32        = 36.0;
const BACKDROP:        Color32    = Color32::from_rgba_unmultiplied_const( 0, 0, 0, 150 );
const UI_SCALES:       [f32; 7]   = [ 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0 ];
const HEX_WIDTHS:      [usize; 4] = [ 8, 16, 32, 64 ];

/// Steam's default install location of the game.
const STEAM_GAME_DATA: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Pathologic Classic HD\data";

#[ derive( Clone, Copy, PartialEq, Eq, Serialize, Deserialize ) ]
pub enum SidebarLayout
{
    List,
    Grid,
}

impl SidebarLayout
{
    fn key( self ) -> &'static str
    {
        match self
        {
            Self::List => "menu.list",
            Self::Grid => "menu.thumbnails",
        }
    }
}

/// Graphics API the window and the 3D views are drawn with; picked at startup.
#[ derive( Clone, Copy, PartialEq, Eq, Serialize, Deserialize ) ]
pub enum GraphicsBackend
{
    Dx12,
    Vulkan,
    Metal,
}

impl GraphicsBackend
{
    #[ cfg( target_os = "windows" ) ]
    pub const ALL: &[Self] = &[ Self::Dx12, Self::Vulkan ];

    /// Vulkan only runs on macOS through a translation layer.
    #[ cfg( target_os = "macos" ) ]
    pub const ALL: &[Self] = &[ Self::Metal ];

    #[ cfg( not( any( target_os = "windows", target_os = "macos" ) ) ) ]
    pub const ALL: &[Self] = &[ Self::Vulkan ];

    pub fn label( self ) -> &'static str
    {
        match self
        {
            Self::Dx12 => "DirectX 12",
            Self::Vulkan => "Vulkan",
            Self::Metal => "Metal",
        }
    }

    /// The saved choice if this platform has it, else its first.
    pub fn backends( self ) -> wgpu::Backends
    {
        let backend = if Self::ALL.contains( &self ) { self } else { Self::ALL[ 0 ] };

        match backend
        {
            Self::Dx12 => wgpu::Backends::DX12,
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::Metal => wgpu::Backends::METAL,
        }
    }
}

/// How the 3D views draw and what they show over the picture.
#[ derive( Clone, Copy, PartialEq, Serialize, Deserialize ) ]
#[ serde( default ) ]
pub struct ViewerSettings
{
    pub backend: GraphicsBackend,
    pub vsync:   bool,

    /// Frames per second at most when vsync is off.
    pub fps_limit: u32,

    pub show_fps:         bool,
    pub show_coordinates: bool,
    pub show_stats:       bool,
    pub show_orientation: bool,
}

impl Default for ViewerSettings
{
    fn default() -> Self
    {
        let settings = ViewerSettings
        {
            backend:          GraphicsBackend::ALL[ 0 ],
            vsync:            true,
            fps_limit:        144,
            show_fps:         true,
            show_coordinates: true,
            show_stats:       true,
            show_orientation: true,
        };

        settings
    }
}

#[ derive( Clone, Serialize, Deserialize ) ]
#[ serde( default ) ]
pub struct Settings
{
    pub language:          Language,
    pub sidebar:           SidebarLayout,
    pub hex_bytes_per_row: usize,
    pub ui_scale:          f32,
    pub theme:             Theme,

    /// The game's data folder; when set, its archives load at startup.
    pub game_data: Option<PathBuf>,

    pub viewer: ViewerSettings,
}

impl Settings
{
    /// What the last run saved, read before the window exists: the graphics API is chosen with it.
    pub fn saved( app_name: &str ) -> Self
    {
        let path   = eframe::storage_dir( app_name ).map( |dir| dir.join( STORAGE_FILE ) );
        let text   = path.and_then( |path| std::fs::read_to_string( path ).ok() );
        let values = text.and_then( |text| ron::from_str::<HashMap<String, String>>( &text ).ok() );
        let saved  = values.and_then( |values| ron::from_str( values.get( STORAGE_KEY )? ).ok() );

        saved.unwrap_or_default()
    }
}

impl Default for Settings
{
    fn default() -> Self
    {
        let settings = Settings
        {
            language:          Language::system(),
            sidebar:           SidebarLayout::List,
            hex_bytes_per_row: 16,
            ui_scale:          1.0,
            theme:             Theme::Dark,
            game_data:         Some( PathBuf::from( STEAM_GAME_DATA ) ).filter( |dir| dir.is_dir() ),
            viewer:            ViewerSettings::default(),
        };

        settings
    }
}

#[ derive( Clone, Copy, PartialEq, Eq ) ]
pub enum Page
{
    General,
    Game,
    Viewer,
    Theming,
    About,
}

impl Page
{
    fn key( self ) -> &'static str
    {
        match self
        {
            Self::General => "settings.general",
            Self::Game => "settings.game",
            Self::Viewer => "settings.viewer",
            Self::Theming => "settings.theming",
            Self::About => "settings.about",
        }
    }
}

/// Facts for the About page that only the app knows.
pub struct AboutInfo
{
    pub renderer: String,
}

/// Draws the preferences over the window; returns `false` once it should close.
pub fn show( ctx: &egui::Context, palette: &Palette, settings: &mut Settings, page: &mut Page, about: &AboutInfo ) -> bool
{
    let frame = egui::Frame::NONE
        .fill( palette.view )
        .stroke( Stroke::new( 1.0, palette.border ) )
        .corner_radius( CornerRadius::same( theme::PANEL_RADIUS ) );
    let modal = egui::Modal::new( egui::Id::unique( "preferences" ) ).backdrop_color( BACKDROP ).frame( frame ).show(
        ctx,
        |ui|
        {
            let ( rect, _ ) = ui.allocate_exact_size( MODAL_SIZE, Sense::hover() );
            let nav     = Rect::from_min_size( rect.min, Vec2::new( NAV_WIDTH, rect.height() ) );
            let content = Rect::from_min_max( Pos2::new( nav.max.x, rect.min.y ), rect.max );
            ui.painter().rect( content, CornerRadius::same( theme::PANEL_RADIUS ), palette.window, Stroke::new( 1.0, palette.border ), StrokeKind::Inside );

            let mut nav_ui = ui.new_child( UiBuilder::new().max_rect( nav.shrink( NAV_PADDING ) ).layout( Layout::top_down( Align::Min ) ) );
            let back       = navigation( &mut nav_ui, palette, &t!( "settings.back" ), true, false );
            nav_ui.add_space( NAV_PADDING );
            for item in [ Page::General, Page::Game, Page::Viewer, Page::Theming ]
            {
                if navigation( &mut nav_ui, palette, &t!( item.key() ), false, *page == item ).clicked()
                {
                    *page = item;
                }
            }
            let about_row = Rect::from_min_size(
                Pos2::new( nav.min.x + NAV_PADDING, nav.max.y - NAV_PADDING - NAV_ROW ),
                Vec2::new( nav.width() - ( NAV_PADDING * 2.0 ), NAV_ROW ),
            );
            let mut about_ui = ui.new_child( UiBuilder::new().max_rect( about_row ) );
            if navigation( &mut about_ui, palette, &t!( Page::About.key() ), false, *page == Page::About ).clicked()
            {
                *page = Page::About;
            }

            let mut content_ui = ui.new_child( UiBuilder::new().max_rect( content.shrink( CONTENT_PADDING ) ).layout( Layout::top_down( Align::Min ) ) );
            content_ui.label( egui::RichText::new( t!( page.key() ) ).font( theme::bold_font( theme::STATUS_TEXT_SIZE ) ).color( palette.text ) );
            content_ui.add_space( 12.0 );
            match page
            {
                Page::General => general( &mut content_ui, palette, settings ),
                Page::Game => game( &mut content_ui, palette, settings ),
                Page::Viewer => viewer( &mut content_ui, palette, &mut settings.viewer ),
                Page::Theming => theming( &mut content_ui, palette, settings ),
                Page::About => about_page( &mut content_ui, palette, about ),
            }
            back.clicked()
        }
    );
    let close = modal.inner || modal.should_close();

    !close
}

fn navigation( ui: &mut egui::Ui, palette: &Palette, label: &str, back: bool, selected: bool ) -> egui::Response
{
    let ( rect, response ) = ui.allocate_exact_size( Vec2::new( ui.available_width(), NAV_ROW ), Sense::click() );
    let painter = ui.painter();
    if selected || response.hovered()
    {
        painter.rect_filled( rect, CornerRadius::same( theme::CONTROL_RADIUS ), palette.hovered );
    }

    let mut x = rect.min.x + 12.0;

    if back
    {
        let chevron = Rect::from_center_size( Pos2::new( x + ( widgets::ICON_SIZE / 2.0 ) - 6.0, rect.center().y ), Vec2::splat( widgets::ICON_SIZE ) );
        widgets::paint_chevron_left( ui, chevron, palette.text );
        x += widgets::ICON_SIZE + 2.0;
    }
    let color = if selected || back { palette.text } else { palette.menu_text_hovered };
    ui.painter().text( Pos2::new( x, rect.center().y ), egui::Align2::LEFT_CENTER, label, FontId::proportional( theme::STATUS_TEXT_SIZE ), color );

    response
}

/// Setting row: label on the left, control on the right.
fn row( ui: &mut egui::Ui, palette: &Palette, label: &str, control: impl FnOnce( &mut egui::Ui ) )
{
    let ( rect, _ ) = ui.allocate_exact_size( Vec2::new( ui.available_width(), ROW_HEIGHT ), Sense::hover() );
    ui.painter().text( Pos2::new( rect.min.x, rect.center().y ), egui::Align2::LEFT_CENTER, label, FontId::proportional( theme::TEXT_SIZE ), palette.text );
    ui.painter().hline( rect.x_range(), rect.max.y, Stroke::new( 1.0, palette.border ) );
    let mut right = ui.new_child( UiBuilder::new().max_rect( rect ).layout( Layout::right_to_left( Align::Center ) ) );
    control( &mut right );
}

fn choice<T: Copy + PartialEq>( ui: &mut egui::Ui, palette: &Palette, value: &mut T, options: &[T], label: impl Fn( T ) -> String )
{
    let response = widgets::dropdown_button( ui, palette, &label( *value ) );
    egui::Popup::menu( &response ).show(
        |ui|
        {
            for &option in options
            {
                if ui.selectable_label( option == *value, label( option ) ).clicked()
                {
                    *value = option;
                }
            }
        }
    );
}

fn general( ui: &mut egui::Ui, palette: &Palette, settings: &mut Settings )
{
    row(
        ui,
        palette,
        &t!( "settings.language" ),
        |ui|
        {
            choice( ui, palette, &mut settings.language, &Language::ALL, |language| language.label().to_owned() );
        }
    );
    row(
        ui,
        palette,
        &t!( "settings.sidebar" ),
        |ui|
        {
            choice( ui, palette, &mut settings.sidebar, &[ SidebarLayout::List, SidebarLayout::Grid ], |layout| t!( layout.key() ).into_owned() );
        }
    );
    row(
        ui,
        palette,
        &t!( "settings.hex_bytes" ),
        |ui|
        {
            choice( ui, palette, &mut settings.hex_bytes_per_row, &HEX_WIDTHS, |width| i18n::count( "hex.bytes", width ) );
        }
    );
}

fn game( ui: &mut egui::Ui, palette: &Palette, settings: &mut Settings )
{
    row(
        ui,
        palette,
        &t!( "settings.data_folder" ),
        |ui|
        {
            // Right to left: browse ends up rightmost.
            if ui.add( egui::Button::new( t!( "settings.browse" ) ).fill( palette.border ) ).clicked()
                && let Some( dir ) = rfd::FileDialog::new().pick_folder()
            {
                settings.game_data = Some( dir );
            }
            if settings.game_data.is_some() && ui.add( egui::Button::new( t!( "settings.clear" ) ).fill( palette.border ) ).clicked()
            {
                settings.game_data = None;
            }
        }
    );

    let ( path, color ) = match &settings.game_data
    {
        Some( dir ) => ( dir.display().to_string(), palette.menu_text_hovered ),
        None => ( t!( "settings.data_folder_hint" ).into_owned(), palette.menu_text ),
    };
    ui.add_space( 6.0 );
    ui.label( egui::RichText::new( path ).size( theme::TEXT_SIZE ).color( color ) );
}

fn viewer( ui: &mut egui::Ui, palette: &Palette, settings: &mut ViewerSettings )
{
    row(
        ui,
        palette,
        &t!( "settings.backend" ),
        |ui|
        {
            choice( ui, palette, &mut settings.backend, GraphicsBackend::ALL, |backend| backend.label().to_owned() );
        }
    );
    ui.add_space( 6.0 );
    ui.label( egui::RichText::new( t!( "settings.backend_hint" ) ).size( theme::TEXT_SIZE ).color( palette.menu_text ) );
    ui.add_space( 6.0 );

    switch( ui, palette, &t!( "settings.vsync" ), &mut settings.vsync );
    if !settings.vsync
    {
        row(
            ui,
            palette,
            &t!( "settings.fps_limit" ),
            |ui|
            {
                ui.add( egui::Slider::new( &mut settings.fps_limit, FPS_LIMITS ) );
            }
        );
    }
    switch( ui, palette, &t!( "settings.show_fps" ), &mut settings.show_fps );
    switch( ui, palette, &t!( "settings.show_coordinates" ), &mut settings.show_coordinates );
    switch( ui, palette, &t!( "settings.show_stats" ), &mut settings.show_stats );
    switch( ui, palette, &t!( "settings.show_orientation" ), &mut settings.show_orientation );
}

/// Setting row with a checkbox on the right.
fn switch( ui: &mut egui::Ui, palette: &Palette, label: &str, value: &mut bool )
{
    row(
        ui,
        palette,
        label,
        |ui|
        {
            ui.checkbox( value, "" );
        }
    );
}

fn theming( ui: &mut egui::Ui, palette: &Palette, settings: &mut Settings )
{
    row(
        ui,
        palette,
        &t!( "settings.theme" ),
        |ui|
        {
            choice( ui, palette, &mut settings.theme, &Theme::ALL, |theme| t!( theme.key() ).into_owned() );
        }
    );
    row(
        ui,
        palette,
        &t!( "settings.scale" ),
        |ui|
        {
            choice( ui, palette, &mut settings.ui_scale, &UI_SCALES, |scale| format!( "{:.0}%", scale * 100.0 ) );
        }
    );
}

fn about_page( ui: &mut egui::Ui, palette: &Palette, about: &AboutInfo )
{
    let line = |ui: &mut egui::Ui, text: &str, color: Color32|
    {
        ui.label( egui::RichText::new( text ).size( theme::TEXT_SIZE ).color( color ) );
    };
    line( ui, &format!( "Mor Viewer {}.{} ({})", env!( "CARGO_PKG_VERSION" ), env!( "MOR_BUILD" ), env!( "MOR_COMMIT" ) ), palette.text );
    line( ui, &t!( "settings.description" ), palette.menu_text_hovered );
    ui.add_space( 12.0 );
    line( ui, &t!( "settings.renderer", name = about.renderer ), palette.menu_text_hovered );
    line( ui, &t!( "settings.font" ), palette.menu_text_hovered );
}
