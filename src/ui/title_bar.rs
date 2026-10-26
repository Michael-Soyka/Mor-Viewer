use eframe::egui::{
    self, emath::GuiRounding, Color32, CursorIcon, FontData, FontFamily, FontId, FontInsert, FontPriority, InsertFontFamily, Pos2, Rect,
    ResizeDirection, Sense, Stroke, StrokeKind, Vec2, ViewportCommand,
};

use crate::ui::theme;

pub const HEIGHT: f32 = 25.0;

/// Windows 11 caption buttons: 46 px wide, flush with the window edge, full bar height.
const BUTTON_WIDTH: f32 = 46.0;

const BAR_PADDING:   f32     = 4.0;
const TITLE_ICON:    f32     = 13.0;
const TITLE_GAP:     f32     = 4.0;
const TITLE:         &str    = "Mor Viewer";
const GLYPH_SIZE:    f32     = 10.0;
const RESIZE_BORDER: f32     = 5.0;
const CLOSE_HOVER:   Color32 = Color32::from_rgb( 0xC4, 0x2B, 0x1C );
const CLOSE_PRESS:   Color32 = Color32::from_rgb( 0xB2, 0x27, 0x1A );
const GLYPH_FONT:    &str    = "caption-glyphs";

// macOS keeps native traffic lights; the title bar only reserves room for them.
const CAPTION_BUTTONS: bool = cfg!( not( target_os = "macos" ) );
const LEADING_INSET:   f32  = if CAPTION_BUTTONS { BAR_PADDING } else { 72.0 };

#[ derive( Clone, Copy ) ]
enum Caption
{
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl Caption
{
    fn glyph( self ) -> char
    {
        match self
        {
            Self::Minimize => '\u{E921}',
            Self::Maximize => '\u{E922}',
            Self::Restore => '\u{E923}',
            Self::Close => '\u{E8BB}',
        }
    }

    fn command( self ) -> ViewportCommand
    {
        match self
        {
            Self::Minimize => ViewportCommand::Minimized( true ),
            Self::Maximize => ViewportCommand::Maximized( true ),
            Self::Restore => ViewportCommand::Maximized( false ),
            Self::Close => ViewportCommand::Close,
        }
    }
}

pub struct TitleBar
{
    glyphs: Option<FontFamily>,
}

impl TitleBar
{
    pub fn new( ctx: &egui::Context ) -> Self
    {
        let glyphs = load_glyph_font().map(
            |data|
            {
                let family = FontFamily::Name( GLYPH_FONT.into() );
                let insert = InsertFontFamily { family: family.clone(), priority: FontPriority::Highest };
                ctx.add_font( FontInsert::new( GLYPH_FONT, FontData::from_owned( data ), vec![ insert ] ) );
                family
            }
        );

        Self { glyphs }
    }

    /// Draws caption buttons, the drag area and the centered title; returns a row for the menu.
    pub fn show( &self, ui: &mut egui::Ui ) -> egui::Ui
    {
        let rect      = ui.max_rect();
        let maximized = ui.input( |i| i.viewport().maximized.unwrap_or( false ) );
        let captions = [ Caption::Close, if maximized { Caption::Restore } else { Caption::Maximize }, Caption::Minimize ];
        let buttons: &[Caption] = if CAPTION_BUTTONS { &captions } else { &[] };

        let mut right = rect.max.x;

        for &caption in buttons
        {
            let button = Rect::from_min_max( Pos2::new( right - BUTTON_WIDTH, rect.min.y ), Pos2::new( right, rect.max.y ) );

            right -= BUTTON_WIDTH;
            if self.caption_button( ui, button, caption ).clicked()
            {
                ui.ctx().send_viewport_cmd( caption.command() );
            }
        }

        let drag_rect = Rect::from_min_max( rect.min, Pos2::new( right, rect.max.y ) );
        let drag      = ui.interact( drag_rect, ui.scope_id().with( "drag" ), Sense::click_and_drag() );
        if drag.double_clicked()
        {
            ui.ctx().send_viewport_cmd( ViewportCommand::Maximized( !maximized ) );
        }
        else if drag.drag_started()
        {
            ui.ctx().send_viewport_cmd( ViewportCommand::StartDrag );
        }

        paint_title( ui, rect );

        let content = Rect::from_min_max( Pos2::new( rect.min.x + LEADING_INSET, rect.min.y ), Pos2::new( right, rect.max.y ) );
        let layout  = egui::Layout::left_to_right( egui::Align::Center );
        let mut row = ui.new_child( egui::UiBuilder::new().max_rect( content ).layout( layout ) );
        row.spacing_mut().item_spacing.x = BAR_PADDING;

        row
    }

    fn caption_button( &self, ui: &egui::Ui, rect: Rect, caption: Caption ) -> egui::Response
    {
        let response = ui.interact( rect, ui.scope_id().with( caption.glyph() ), Sense::click() );
        let visuals  = ui.visuals();
        let focused  = ui.input( |i| i.viewport().focused.unwrap_or( true ) );
        let pressed  = response.is_pointer_button_down_on();
        let idle     = if focused { visuals.strong_text_color() } else { visuals.weak_text_color() };

        let ( fill, color ) = match caption
        {
            Caption::Close if pressed => ( CLOSE_PRESS, Color32::WHITE ),
            Caption::Close if response.hovered() => ( CLOSE_HOVER, Color32::WHITE ),
            // The theme's hover overlay, a third lighter while pressed.
            _ if pressed => ( visuals.widgets.hovered.weak_bg_fill.gamma_multiply( 0.67 ), idle ),
            _ if response.hovered() => ( visuals.widgets.hovered.weak_bg_fill, visuals.strong_text_color() ),
            _ => ( Color32::TRANSPARENT, idle ),
        };

        ui.painter().rect_filled( rect, 0.0, fill );
        self.paint_glyph( ui.painter(), rect.center(), caption, GLYPH_SIZE, color );

        response
    }

    /// The caption close cross, reused for viewer tabs.
    pub fn paint_close( &self, painter: &egui::Painter, center: Pos2, size: f32, color: Color32 )
    {
        self.paint_glyph( painter, center, Caption::Close, size, color );
    }

    fn paint_glyph( &self, painter: &egui::Painter, center: Pos2, caption: Caption, size: f32, color: Color32 )
    {
        match &self.glyphs
        {
            Some( family ) =>
            {
                let galley = painter.layout_no_wrap( caption.glyph().to_string(), FontId::new( size, family.clone() ), color );
                let pos    = ( center - ( galley.size() / 2.0 ) ).round_to_pixels( painter.pixels_per_point() );
                painter.galley( pos, galley, color );
            }
            None => paint_caption( painter, center, caption, size, color ),
        }
    }
}

/// App icon and name in the middle of the bar.
fn paint_title( ui: &egui::Ui, rect: Rect )
{
    let color  = ui.visuals().strong_text_color();
    let galley = ui.painter().layout_no_wrap( TITLE.to_owned(), FontId::proportional( theme::TEXT_SIZE ), color );
    let width  = TITLE_ICON + TITLE_GAP + galley.size().x;
    let icon   = Rect::from_min_size( Pos2::new( rect.center().x - ( width / 2.0 ), rect.center().y - ( TITLE_ICON / 2.0 ) ), Vec2::splat( TITLE_ICON ) );
    egui::Image::new( crate::ui::icons::APP ).tint( color ).paint_at( ui, icon );
    let text = Pos2::new( icon.max.x + TITLE_GAP, rect.center().y - ( galley.size().y / 2.0 ) );
    ui.painter().galley( text, galley, color );
}

/// Lets a frameless window be resized by its edges. Call after all panels so the cursor wins.
pub fn resize_edges( ctx: &egui::Context )
{
    if !CAPTION_BUTTONS
    {
        return;
    }
    let ( expanded, pos, pressed, rect ) = ctx.input(
        |i|
        {
            let viewport = i.viewport();
            let expanded = viewport.maximized.unwrap_or( false ) || viewport.fullscreen.unwrap_or( false );
            ( expanded, i.pointer.hover_pos(), i.pointer.primary_pressed(), i.viewport_rect() )
        }
    );
    let Some( pos ) = pos else { return };

    if expanded
    {
        return;
    }

    let north = pos.y < rect.min.y + RESIZE_BORDER;
    let south = pos.y > rect.max.y - RESIZE_BORDER;
    let west  = pos.x < rect.min.x + RESIZE_BORDER;
    let east  = pos.x > rect.max.x - RESIZE_BORDER;
    let ( direction, cursor ) = match ( north, south, west, east )
    {
        ( true, _, true, _ ) => ( ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest ),
        ( true, _, _, true ) => ( ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast ),
        ( _, true, true, _ ) => ( ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest ),
        ( _, true, _, true ) => ( ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast ),
        ( true, .. ) => ( ResizeDirection::North, CursorIcon::ResizeNorth ),
        ( _, true, .. ) => ( ResizeDirection::South, CursorIcon::ResizeSouth ),
        ( _, _, true, _ ) => ( ResizeDirection::West, CursorIcon::ResizeWest ),
        ( .., true ) => ( ResizeDirection::East, CursorIcon::ResizeEast ),
        _ => return,
    };
    ctx.set_cursor_icon( cursor );
    if pressed
    {
        ctx.send_viewport_cmd( ViewportCommand::BeginResize( direction ) );
    }
}

#[ cfg( windows ) ]
fn load_glyph_font() -> Option<Vec<u8>>
{
    let fonts = std::path::PathBuf::from( std::env::var_os( "WINDIR" )? ).join( "Fonts" );

    // Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on Windows 10.
    [ "SegoeIcons.ttf", "segmdl2.ttf" ].iter().find_map( |name| std::fs::read( fonts.join( name ) ).ok() )
}

#[ cfg( not( windows ) ) ]
fn load_glyph_font() -> Option<Vec<u8>>
{
    None
}

fn paint_caption( painter: &egui::Painter, center: Pos2, caption: Caption, size: f32, color: Color32 )
{
    let stroke = Stroke::new( 1.0, color );
    let half   = size / 2.0;

    match caption
    {
        Caption::Minimize =>
        {
            painter.hline( ( center.x - half )..=( center.x + half ), center.y, stroke );
        }
        Caption::Maximize =>
        {
            painter.rect_stroke( Rect::from_center_size( center, Vec2::splat( size ) ), 0.0, stroke, StrokeKind::Inside );
        }
        Caption::Restore =>
        {
            let offset = 2.0;
            let front  = Rect::from_min_size( Pos2::new( center.x - half, center.y - half + offset ), Vec2::splat( size - offset ) );
            painter.rect_stroke( front, 0.0, stroke, StrokeKind::Inside );
            painter.hline( ( center.x - half + offset )..=( center.x + half ), center.y - half, stroke );
            painter.vline( center.x + half, ( center.y - half )..=( center.y + half - offset ), stroke );
        }
        Caption::Close =>
        {
            painter.line_segment( [ center - Vec2::splat( half ), center + Vec2::splat( half ) ], stroke );
            painter.line_segment( [ center + Vec2::new( -half, half ), center + Vec2::new( half, -half ) ], stroke );
        }
    }
}
