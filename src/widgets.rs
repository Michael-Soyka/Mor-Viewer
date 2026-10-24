use eframe::egui::{ self, Align2, Color32, CornerRadius, FontId, Image, ImageSource, Pos2, Rect, Response, Sense, Stroke, Vec2 };

use rust_i18n::t;
use crate::icons;
use crate::theme::{ self, Palette };

pub const ICON_SIZE:  f32 = 10.0;
pub const ROW_HEIGHT: f32 = 23.0;

/// Space between tree rows.
pub const ROW_GAP: f32 = 2.0;

pub const BAR_HEIGHT: f32 = 32.0;

const TAB_HEIGHT:    f32 = 21.0;
const SLIDER_WIDTH:  f32 = 124.0;
const SLIDER_HEIGHT: f32 = 14.0;
const ROW_PADDING:   f32 = 5.0;
const ROW_ICON_GAP:  f32 = 8.0;
const BADGE_GAP:     f32 = 4.0;
const TAB_GAP:       f32 = 4.0;

/// Search field height inside its border.
const SEARCH_HEIGHT: f32 = 21.0;

const SEARCH_ICON: f32 = 13.0;

pub const ZOOM_PRESETS: [f32; 9] = [ 0.25, 0.5, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 16.0 ];

/// egui lays out in f32; there is no lossless conversion from usize.
pub fn to_f32( value: usize ) -> f32
{
    value as f32
}

/// Row index from a non-negative layout coordinate; Rust has no `TryFrom<f32>` for integers.
pub fn from_f32( value: f32 ) -> usize
{
    value as usize
}

pub fn paint_icon( ui: &egui::Ui, source: ImageSource<'static>, rect: Rect, tint: Color32 )
{
    Image::new( source ).tint( tint ).paint_at( ui, rect );
}

/// Chevron pointing down when `expanded`, right otherwise.
pub fn paint_chevron( ui: &egui::Ui, rect: Rect, expanded: bool, tint: Color32 )
{
    let angle = if expanded { 0.0 } else { -std::f32::consts::FRAC_PI_2 };
    Image::new( icons::CHEVRON ).tint( tint ).rotate( angle, Vec2::splat( 0.5 ) ).paint_at( ui, rect );
}

/// Chevron pointing left, for "back" links.
pub fn paint_chevron_left( ui: &egui::Ui, rect: Rect, tint: Color32 )
{
    Image::new( icons::CHEVRON ).tint( tint ).rotate( std::f32::consts::FRAC_PI_2, Vec2::splat( 0.5 ) ).paint_at( ui, rect );
}

pub enum RowIcon
{
    Chevron( bool ),
    Image( ImageSource<'static> ),
}

/// Tree row: chevron or icon, then label; filled on hover or when selected.
/// Chevron rows also carry an archive badge at the top level and a folder one below it.
/// Top-level rows (archives) use the regular face, nested ones the light one.
pub fn tree_row( ui: &mut egui::Ui, palette: &Palette, indent: f32, icon: RowIcon, label: &str, top_level: bool, selected: bool ) -> Response
{
    let ( rect, response ) = ui.allocate_exact_size( Vec2::new( ui.available_width(), ROW_HEIGHT ), Sense::click() );
    if !ui.is_rect_visible( rect )
    {
        return response;
    }
    if selected || response.hovered()
    {
        ui.painter().rect_filled( rect, CornerRadius::same( theme::PANEL_RADIUS ), palette.border );
    }

    let icon_rect = Rect::from_center_size( Pos2::new( rect.min.x + indent + ( ICON_SIZE / 2.0 ), rect.center().y ), Vec2::splat( ICON_SIZE ) );

    let chevron = matches!( icon, RowIcon::Chevron( _ ) );
    match icon
    {
        RowIcon::Chevron( expanded ) => paint_chevron( ui, icon_rect, expanded, palette.text ),
        RowIcon::Image( source ) => paint_icon( ui, source, icon_rect, palette.text ),
    }
    let mut text_x = icon_rect.max.x + ROW_ICON_GAP;
    if chevron
    {
        let source = if top_level { icons::ARCHIVE } else { icons::FOLDER };
        let badge  = Rect::from_min_size( Pos2::new( text_x, rect.center().y - ( ICON_SIZE / 2.0 ) ), Vec2::splat( ICON_SIZE ) );
        paint_icon( ui, source, badge, palette.text );
        text_x = badge.max.x + BADGE_GAP;
    }
    let text_pos = Pos2::new( text_x, rect.center().y );
    let clip     = Rect::from_min_max( rect.min, Pos2::new( rect.max.x - ROW_PADDING, rect.max.y ) );

    let font = if top_level { FontId::proportional( theme::TEXT_SIZE ) } else { theme::light_font( theme::TEXT_SIZE ) };
    ui.painter().with_clip_rect( clip ).text( text_pos, Align2::LEFT_CENTER, label, font, palette.text );

    response
}

pub fn row_indent( depth: usize ) -> f32
{
    ROW_PADDING + ( 10.0 * to_f32( depth ) )
}

pub struct TabResponse
{
    pub clicked:        bool,
    pub double_clicked: bool,
    pub closed:         bool,
    pub response:       Response,
}

/// Viewer tab: file kind icon, label, close cross; the inactive one is dimmed, a preview one is italic.
pub fn tab( ui: &mut egui::Ui, palette: &Palette, icon: ImageSource<'static>, label: &str, current: bool, preview: bool, close: impl Fn( &egui::Painter, Pos2, Color32 ) ) -> TabResponse
{
    let font = if current { FontId::proportional( theme::TEXT_SIZE ) } else { theme::light_font( theme::TEXT_SIZE ) };
    let color = if current { palette.text } else { palette.text_weak };
    let format  = egui::TextFormat { font_id: font, color, italics: preview, ..Default::default() };
    let mut job = egui::text::LayoutJob::single_section( label.to_owned(), format );
    job.wrap.max_rows = 1;
    let galley = ui.painter().layout_job( job );

    let padding = Vec2::new( 6.0, 4.0 );
    let size    = Vec2::new( padding.x + ICON_SIZE + TAB_GAP + galley.size().x + TAB_GAP + ICON_SIZE + padding.x, TAB_HEIGHT );
    let ( rect, response ) = ui.allocate_exact_size( size, Sense::click() );
    let close_rect     = Rect::from_center_size( Pos2::new( rect.max.x - padding.x - ( ICON_SIZE / 2.0 ), rect.center().y ), Vec2::splat( ICON_SIZE + 4.0 ) );
    let close_response = ui.interact( close_rect, response.id.with( "close" ), Sense::click() );

    let painter = ui.painter();

    if current
    {
        painter.rect_filled( rect, CornerRadius::same( theme::CONTROL_RADIUS ), palette.border );
    }
    else if response.hovered()
    {
        painter.rect_filled( rect, CornerRadius::same( theme::CONTROL_RADIUS ), palette.hovered );
    }
    let icon_rect = Rect::from_min_size( Pos2::new( rect.min.x + padding.x, rect.center().y - ( ICON_SIZE / 2.0 ) ), Vec2::splat( ICON_SIZE ) );
    paint_icon( ui, icon, icon_rect, palette.text );
    let text_pos = Pos2::new( icon_rect.max.x + TAB_GAP, rect.center().y - ( galley.size().y / 2.0 ) );
    painter.galley( text_pos, galley, color );
    if close_response.hovered()
    {
        painter.rect_filled( close_rect, CornerRadius::same( 2 ), palette.hovered );
    }
    let cross = if current || close_response.hovered() { palette.text } else { palette.text_weak };
    close( painter, close_rect.center(), cross );

    let middle = response.clicked_by( egui::PointerButton::Middle );
    let result = TabResponse
    {
        clicked:        response.clicked() && !close_response.clicked(),
        double_clicked: response.double_clicked(),
        closed:         close_response.clicked() || middle,
        response,
    };

    result
}

/// R/G/B/A switch: filled when on, faded letter when off.
pub fn channel_toggle( ui: &mut egui::Ui, palette: &Palette, letter: &str, color: Color32, on: &mut bool ) -> Response
{
    let font   = theme::bold_font( theme::STATUS_TEXT_SIZE );
    let galley = ui.painter().layout_no_wrap( letter.to_owned(), font, color );
    let size   = Vec2::new( galley.size().x + 12.0, 20.0 );
    let ( rect, mut response ) = ui.allocate_exact_size( size, Sense::click() );
    if response.clicked()
    {
        *on = !*on;
        response.mark_changed();
    }

    let painter = ui.painter();

    if *on || response.hovered()
    {
        painter.rect_filled( rect, CornerRadius::same( theme::CONTROL_RADIUS ), palette.hovered );
    }
    let tint = if *on { color } else { color.gamma_multiply( 0.35 ) };
    painter.galley_with_override_text_color( rect.center() - ( galley.size() / 2.0 ), galley, tint );

    response
}

/// Tab strip scroll button: filled square with a chevron.
pub fn scroll_button( ui: &mut egui::Ui, palette: &Palette, rect: Rect, left: bool ) -> Response
{
    let response = ui.interact( rect, ui.scope_id().with( ( "tabs_scroll", left ) ), Sense::click() );
    let fill     = if response.hovered() { palette.hovered } else { palette.border };
    let chevron  = Rect::from_center_size( rect.center(), Vec2::splat( ICON_SIZE ) );

    ui.painter().rect_filled( rect, CornerRadius::same( theme::CONTROL_RADIUS ), fill );
    if left
    {
        paint_chevron_left( ui, chevron, palette.text );
    }
    else
    {
        paint_chevron( ui, chevron, false, palette.text );
    }

    response
}

/// Square icon button for the viewer action bar.
pub fn icon_toggle( ui: &mut egui::Ui, palette: &Palette, source: ImageSource<'static>, on: bool ) -> Response
{
    let ( rect, response ) = ui.allocate_exact_size( Vec2::splat( 22.0 ), Sense::click() );
    if on || response.hovered()
    {
        ui.painter().rect_filled( rect, CornerRadius::same( theme::CONTROL_RADIUS ), palette.hovered );
    }
    let tint = if on { palette.text } else { palette.menu_text };
    paint_icon( ui, source, Rect::from_center_size( rect.center(), Vec2::splat( ICON_SIZE + 4.0 ) ), tint );

    response
}

pub enum Zoom
{
    Unchanged,
    Set( f32 ),
    Fit,
}

/// Zoom drop-down with presets plus a logarithmic slider; expects a right-to-left layout.
pub fn zoom_control( ui: &mut egui::Ui, palette: &Palette, zoom: f32, range: (f32, f32), fit: bool ) -> Zoom
{
    let mut result = Zoom::Unchanged;

    if let Some( value ) = zoom_slider( ui, palette, zoom, range )
    {
        result = Zoom::Set( value );
    }
    let response = dropdown_button( ui, palette, &format!( "{:.0}%", zoom * 100.0 ) );
    egui::Popup::menu( &response ).show(
        |ui|
        {
            if fit_available( range ) && ui.selectable_label( fit, t!( "zoom.fit" ) ).clicked()
            {
                result = Zoom::Fit;
            }
            for preset in ZOOM_PRESETS.into_iter().filter( |preset| ( range.0..=range.1 ).contains( preset ) )
            {
                let text = format!( "{:.0}%", preset * 100.0 );
                if ui.selectable_label( !fit && ( zoom - preset ).abs() < 1e-3, text ).clicked()
                {
                    result = Zoom::Set( preset );
                }
            }
        }
    );

    result
}

/// Filled button with a label and a down chevron, as the footer drop-downs in the design.
pub fn dropdown_button( ui: &mut egui::Ui, palette: &Palette, label: &str ) -> Response
{
    let font   = FontId::proportional( theme::TEXT_SIZE );
    let galley = ui.painter().layout_no_wrap( label.to_owned(), font, palette.text );
    let size   = Vec2::new( 6.0 + galley.size().x + 4.0 + ICON_SIZE + 6.0, TAB_HEIGHT );
    let ( rect, response ) = ui.allocate_exact_size( size, Sense::click() );

    let painter = ui.painter();
    painter.rect_filled( rect, CornerRadius::same( theme::CONTROL_RADIUS ), palette.border );
    painter.galley( Pos2::new( rect.min.x + 6.0, rect.center().y - ( galley.size().y / 2.0 ) ), galley, palette.text );
    let chevron = Rect::from_center_size( Pos2::new( rect.max.x - 6.0 - ( ICON_SIZE / 2.0 ), rect.center().y ), Vec2::splat( ICON_SIZE ) );
    paint_chevron( ui, chevron, true, palette.text );

    response
}

fn fit_available( range: (f32, f32) ) -> bool
{
    range.0 < 1.0
}

/// 128 px logarithmic slider; returns the new value while dragged or clicked.
/// Logarithmic, so every zoom step takes the same distance.
fn zoom_slider( ui: &mut egui::Ui, palette: &Palette, value: f32, ( min, max ): (f32, f32) ) -> Option<f32>
{
    let ( low, high ) = ( min.ln(), max.ln() );

    let t = ( ( value.ln() - low ) / ( high - low ) ).clamp( 0.0, 1.0 );

    rail( ui, palette, t ).map( |t| ( low + ( t * ( high - low ) ) ).exp() )
}

pub fn slider( ui: &mut egui::Ui, palette: &Palette, value: f32, ( min, max ): (f32, f32) ) -> Option<f32>
{
    let t = ( ( value - min ) / ( max - min ) ).clamp( 0.0, 1.0 );

    rail( ui, palette, t ).map( |t| min + ( t * ( max - min ) ) )
}

/// Rail with a round thumb over `0..=1`; the new position while dragged.
fn rail( ui: &mut egui::Ui, palette: &Palette, t: f32 ) -> Option<f32>
{
    let ( rect, response ) = ui.allocate_exact_size( Vec2::new( SLIDER_WIDTH, SLIDER_HEIGHT ), Sense::click_and_drag() );
    let thumb_radius = 6.0;
    let rail         = rect.shrink2( Vec2::new( thumb_radius, 0.0 ) );

    let result = response.interact_pointer_pos().map( |pointer| ( ( pointer.x - rail.min.x ) / rail.width() ).clamp( 0.0, 1.0 ) );

    let t       = result.unwrap_or( t );
    let x       = rail.min.x + ( t * rail.width() );
    let y       = rect.center().y;
    let painter = ui.painter();

    let line = |from: f32, to: f32, color: Color32|
    {
        painter.rect_filled( Rect::from_min_max( Pos2::new( from, y - 2.0 ), Pos2::new( to, y + 2.0 ) ), CornerRadius::same( 2 ), color );
    };
    line( rail.min.x, rail.max.x, palette.rail );
    line( rail.min.x, x, palette.text );
    let thumb = if response.hovered() || response.dragged() { palette.text } else { palette.menu_text_hovered };
    painter.circle( Pos2::new( x, y ), thumb_radius, thumb, Stroke::new( 1.0, palette.view ) );

    result
}

/// Search field with a border, as at the top of the sidebar.
pub fn search( ui: &mut egui::Ui, palette: &Palette, text: &mut String, hint: &str ) -> Response
{
    let frame = egui::Frame::NONE
        .fill( palette.window )
        .stroke( Stroke::new( 1.0, palette.border ) )
        .corner_radius( CornerRadius::same( theme::PANEL_RADIUS ) )
        .inner_margin( egui::Margin::symmetric( 4, 0 ) );
    let shown = frame.show( ui, |ui| ui.horizontal( |ui| search_row( ui, palette, text, hint ) ).inner );

    shown.inner
}

/// Magnifier, then the text field filling the rest of the row.
fn search_row( ui: &mut egui::Ui, palette: &Palette, text: &mut String, hint: &str ) -> Response
{
    ui.spacing_mut().item_spacing.x = BADGE_GAP;
    let ( icon_rect, _ ) = ui.allocate_exact_size( Vec2::new( SEARCH_ICON, SEARCH_HEIGHT ), Sense::hover() );
    paint_icon( ui, icons::SEARCH, Rect::from_center_size( icon_rect.center(), Vec2::splat( SEARCH_ICON ) ), palette.text.gamma_multiply( 0.75 ) );

    let size = Vec2::new( ui.available_width(), SEARCH_HEIGHT );
    let hint = egui::RichText::new( hint ).font( theme::light_font( theme::TEXT_SIZE ) ).color( palette.text_weak );
    let edit = egui::TextEdit::singleline( text )
        .frame( egui::Frame::NONE )
        .margin( egui::Margin::ZERO )
        .font( FontId::proportional( theme::TEXT_SIZE ) )
        .hint_text( hint )
        .align( Align2::LEFT_CENTER )
        .desired_width( f32::INFINITY );

    ui.add_sized( size, edit )
}

/// File name in at most two lines: breaks after `_` or `.`, then elides the middle keeping the end.
pub fn card_lines( painter: &egui::Painter, name: &str, font: &FontId, max_width: f32 ) -> Vec<String>
{
    let width = |text: &str| painter.layout_no_wrap( text.to_owned(), font.clone(), Color32::WHITE ).size().x;
    if width( name ) <= max_width
    {
        let lines = vec![ name.to_owned() ];
        return lines;
    }

    let breaks: Vec<usize> = name.match_indices( [ '_', '.' ] ).map( |( index, _ )| index + 1 ).filter( |&end| end < name.len() ).collect();
    let first_end = breaks.iter().rev().copied().find( |&end| width( &name[ ..end ] ) <= max_width ).unwrap_or_else(
        ||
        {
            let mut end = 0;
            for ( index, ch ) in name.char_indices()
            {
                if width( &name[ ..index + ch.len_utf8() ] ) > max_width
                {
                    break;
                }
                end = index + ch.len_utf8();
            }
            end.max( name.chars().next().map_or( 0, char::len_utf8 ) )
        }
    );

    let ( first, rest ) = name.split_at( first_end );
    let second = if width( rest ) <= max_width
    {
        rest.to_owned()
    }
    else
    {
        let mut tail_start = rest.len();
        for ( index, _ ) in rest.char_indices().rev()
        {
            if width( &format!( "…{}", &rest[ index.. ] ) ) > max_width
            {
                break;
            }
            tail_start = index;
        }
        format!( "…{}", &rest[ tail_start.. ] )
    };
    let lines = vec![ first.to_owned(), second ];

    lines
}

/// Vertical separator between status bar segments.
/// Rounded bar 1 by 10 in a 3 by 12 slot, as the design's row spacer.
pub fn status_separator( ui: &mut egui::Ui, palette: &Palette )
{
    let ( rect, _ ) = ui.allocate_exact_size( Vec2::new( 3.0, 12.0 ), Sense::hover() );
    ui.painter().rect_filled( Rect::from_center_size( rect.center(), Vec2::new( 1.0, 10.0 ) ), CornerRadius::same( 1 ), palette.text );
}

/// Grey menu labels that light up on hover, as the design's context buttons.
pub fn menu_visuals( ui: &mut egui::Ui, palette: &Palette )
{
    let visuals = ui.visuals_mut();
    visuals.override_text_color = None;
    visuals.widgets.inactive.fg_stroke.color = palette.menu_text;
    visuals.widgets.hovered.fg_stroke.color = palette.menu_text_hovered;
    visuals.widgets.active.fg_stroke.color = palette.menu_text_hovered;
    visuals.widgets.open.fg_stroke.color = palette.menu_text_hovered;
    visuals.widgets.open.weak_bg_fill = palette.hovered;
}
