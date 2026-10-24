use std::path::PathBuf;

use eframe::egui::{ self, Align, Color32, CornerRadius, FontId, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, UiBuilder, Vec2 };
use serde::{ Deserialize, Serialize };

use rust_i18n::t;
use crate::i18n::{ self, Language };
use crate::theme::{ self, Palette, Theme };
use crate::widgets;

pub const STORAGE_KEY: &str = "settings";

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
        };

        settings
    }
}

#[ derive( Clone, Copy, PartialEq, Eq ) ]
pub enum Page
{
    General,
    Game,
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
            for item in [ Page::General, Page::Game, Page::Theming ]
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
