use std::sync::Arc;

use eframe::egui::{ self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Stroke, TextStyle };
use serde::{ Deserialize, Serialize };

pub const LIGHT_FAMILY: &str = "light";
pub const BOLD_FAMILY:  &str = "bold";

// The design uses 10 px; one more keeps JetBrains Mono legible without hinting.
pub const TEXT_SIZE:         f32 = 11.0;
pub const STATUS_TEXT_SIZE:  f32 = 12.0;
pub const CAPTION_TEXT_SIZE: f32 = 8.0;
pub const HEX_TEXT_SIZE:     f32 = 11.0;
pub const HEADING_TEXT_SIZE: f32 = 14.0;

/// JetBrains Mono reserves tall ascent for accents, so lowercase sits about 0.085 em below the line center;
/// this lifts it halfway between the lowercase and the capitals.
const GLYPH_LIFT: f32 = -0.07;

pub const PANEL_RADIUS:   u8 = 6;
pub const CONTROL_RADIUS: u8 = 4;

#[ derive( Clone, Copy, PartialEq, Eq, Serialize, Deserialize ) ]
pub enum Theme
{
    Dark,
    Light,

    /// Follows the OS, dark when it does not say.
    System,
}

impl Theme
{
    pub const ALL: [Self; 3] = [ Self::Dark, Self::Light, Self::System ];

    pub fn key( self ) -> &'static str
    {
        match self
        {
            Self::Dark => "settings.dark",
            Self::Light => "settings.light",
            Self::System => "settings.system_theme",
        }
    }

    pub fn palette( self, system: Option<egui::Theme> ) -> &'static Palette
    {
        match ( self, system )
        {
            ( Self::Light, _ ) | ( Self::System, Some( egui::Theme::Light ) ) => &LIGHT,
            _ => &DARK,
        }
    }
}

pub struct Palette
{
    /// egui's base visuals the palette is applied over.
    pub theme: egui::Theme,

    pub window:            Color32,
    pub view:              Color32,
    pub border:            Color32,
    pub window_border:     Color32,
    pub text:              Color32,
    pub text_weak:         Color32,
    pub menu_text:         Color32,
    pub menu_text_hovered: Color32,
    pub hovered:           Color32,
    pub rail:              Color32,

    /// Translucent backing of the controls floating over a viewer.
    pub plate: Color32,

    pub channels:          [Color32; 4],
    pub syntax:            Syntax,
}

/// Code colors, after VS Code Dark+.
pub struct Syntax
{
    pub line_number: Color32,
    pub punctuation: Color32,
    pub tag:         Color32,
    pub attribute:   Color32,
    pub value:       Color32,
    pub comment:     Color32,
    pub text:        Color32,
}

pub static DARK: Palette = Palette
{
    theme:             egui::Theme::Dark,
    window:            Color32::from_rgb( 0x19, 0x1A, 0x1B ),
    view:              Color32::from_rgb( 0x12, 0x13, 0x14 ),
    border:            Color32::from_rgb( 0x2A, 0x2B, 0x2C ),
    window_border:     Color32::from_rgba_unmultiplied_const( 117, 117, 117, 102 ),
    text:              Color32::WHITE,
    text_weak:         Color32::from_rgb( 0x4A, 0x4B, 0x4B ),
    menu_text:         Color32::from_rgb( 0x80, 0x80, 0x80 ),
    menu_text_hovered: Color32::from_rgb( 0xD5, 0xD5, 0xD5 ),
    hovered:           Color32::from_rgba_unmultiplied_const( 255, 255, 255, 15 ),
    rail:              Color32::from_rgba_unmultiplied_const( 217, 217, 217, 138 ),
    plate:             Color32::from_rgba_unmultiplied_const( 0, 0, 0, 51 ),
    channels:
    [
        Color32::from_rgb( 0xE9, 0x5C, 0x5C ),
        Color32::from_rgb( 0x5C, 0xE9, 0x5C ),
        Color32::from_rgb( 0x5C, 0xA5, 0xE9 ),
        Color32::from_rgb( 0xD5, 0xD5, 0xD5 ),
    ],
    syntax: Syntax
    {
        line_number: Color32::from_rgb( 0x6E, 0x76, 0x81 ),
        punctuation: Color32::from_rgb( 0x80, 0x80, 0x80 ),
        tag:         Color32::from_rgb( 0x56, 0x9C, 0xD6 ),
        attribute:   Color32::from_rgb( 0x9C, 0xDC, 0xFE ),
        value:       Color32::from_rgb( 0xCE, 0x91, 0x78 ),
        comment:     Color32::from_rgb( 0x6A, 0x99, 0x55 ),
        text:        Color32::from_rgb( 0xD4, 0xD4, 0xD4 ),
    },
};

/// Not in the design; mirrors the dark palette, code colors after VS Code Light+.
pub static LIGHT: Palette = Palette
{
    theme:             egui::Theme::Light,
    window:            Color32::from_rgb( 0xF3, 0xF3, 0xF3 ),
    view:              Color32::from_rgb( 0xFF, 0xFF, 0xFF ),
    border:            Color32::from_rgb( 0xC4, 0xC4, 0xC4 ),
    window_border:     Color32::from_rgba_unmultiplied_const( 0, 0, 0, 110 ),
    text:              Color32::from_rgb( 0x14, 0x14, 0x14 ),
    text_weak:         Color32::from_rgb( 0x80, 0x80, 0x80 ),
    menu_text:         Color32::from_rgb( 0x5C, 0x5C, 0x5C ),
    menu_text_hovered: Color32::from_rgb( 0x14, 0x14, 0x14 ),
    hovered:           Color32::from_rgba_unmultiplied_const( 0, 0, 0, 22 ),
    rail:              Color32::from_rgba_unmultiplied_const( 0, 0, 0, 110 ),
    plate:             Color32::from_rgba_unmultiplied_const( 0, 0, 0, 18 ),
    channels:
    [
        Color32::from_rgb( 0xC8, 0x3C, 0x3C ),
        Color32::from_rgb( 0x2E, 0x9E, 0x3E ),
        Color32::from_rgb( 0x2F, 0x7F, 0xD0 ),
        Color32::from_rgb( 0x4A, 0x4A, 0x4A ),
    ],
    syntax: Syntax
    {
        line_number: Color32::from_rgb( 0x23, 0x78, 0x93 ),
        punctuation: Color32::from_rgb( 0x80, 0x00, 0x00 ),
        tag:         Color32::from_rgb( 0x80, 0x00, 0x00 ),
        attribute:   Color32::from_rgb( 0xE5, 0x00, 0x00 ),
        value:       Color32::from_rgb( 0x00, 0x00, 0xFF ),
        comment:     Color32::from_rgb( 0x00, 0x80, 0x00 ),
        text:        Color32::from_rgb( 0x00, 0x00, 0x00 ),
    },
};

pub fn light_font( size: f32 ) -> FontId
{
    FontId::new( size, FontFamily::Name( LIGHT_FAMILY.into() ) )
}

pub fn bold_font( size: f32 ) -> FontId
{
    FontId::new( size, FontFamily::Name( BOLD_FAMILY.into() ) )
}

pub fn apply_palette( ctx: &egui::Context, palette: &Palette )
{
    ctx.set_fonts( fonts( palette.theme ) );
    ctx.set_theme( palette.theme );
    ctx.style_mut_of( palette.theme, |style| apply( style, palette ) );
}

/// Dark text on light fades more, so the light theme draws its thin text in the regular face.
fn fonts( theme: egui::Theme ) -> FontDefinitions
{
    let mut fonts = FontDefinitions::default();
    let faces =
    [
        ( "jetbrains-mono", &include_bytes!( "../../assets/fonts/JetBrainsMono-Regular.ttf" )[ .. ] ),
        ( "jetbrains-mono-light", &include_bytes!( "../../assets/fonts/JetBrainsMono-Light.ttf" )[ .. ] ),
        ( "jetbrains-mono-bold", &include_bytes!( "../../assets/fonts/JetBrainsMono-Bold.ttf" )[ .. ] ),
    ];
    for ( name, data ) in faces
    {
        let tweak = FontTweak { y_offset_factor: GLYPH_LIFT, ..Default::default() };
        fonts.font_data.insert( name.to_owned(), Arc::new( FontData::from_static( data ).tweak( tweak ) ) );
    }

    // Keep egui's bundled fonts as fallbacks for symbols the UI fonts lack.
    let fallbacks = fonts.families.get( &FontFamily::Proportional ).cloned().unwrap_or_default();
    let family = |primary: &str|
    {
        let mut list = vec![ primary.to_owned() ];
        list.extend( fallbacks.iter().cloned() );
        list
    };
    // The whole UI is monospaced, so both generic families share the face.
    fonts.families.insert( FontFamily::Proportional, family( "jetbrains-mono" ) );
    fonts.families.insert( FontFamily::Monospace, family( "jetbrains-mono" ) );
    let thin = if theme == egui::Theme::Light { "jetbrains-mono" } else { "jetbrains-mono-light" };
    fonts.families.insert( FontFamily::Name( LIGHT_FAMILY.into() ), family( thin ) );
    fonts.families.insert( FontFamily::Name( BOLD_FAMILY.into() ), family( "jetbrains-mono-bold" ) );

    fonts
}

fn apply( style: &mut egui::Style, palette: &Palette )
{
    style.text_styles =
    [
        ( TextStyle::Small, FontId::proportional( CAPTION_TEXT_SIZE ) ),
        ( TextStyle::Body, FontId::proportional( TEXT_SIZE ) ),
        ( TextStyle::Button, FontId::proportional( TEXT_SIZE ) ),
        ( TextStyle::Heading, FontId::proportional( STATUS_TEXT_SIZE ) ),
        ( TextStyle::Monospace, FontId::monospace( HEX_TEXT_SIZE ) ),
    ]
    .into();

    let spacing = &mut style.spacing;
    spacing.item_spacing = egui::vec2( 4.0, 4.0 );
    spacing.button_padding = egui::vec2( 4.0, 2.0 );
    spacing.menu_margin = egui::Margin::same( 4 );
    spacing.interact_size.y = 18.0;

    let visuals = &mut style.visuals;
    // Binning shifts glyphs by fractions of a pixel, which blurs small text.
    visuals.text_options.subpixel_binning = false;
    if palette.theme == egui::Theme::Light
    {
        // egui leaves light-mode coverage as is, which washes out small dark text.
        visuals.text_options.color_transfer_function = egui::epaint::FontColorTransferFunction::Gamma( 0.75 );
    }
    visuals.override_text_color = Some( palette.text );
    visuals.panel_fill = palette.window;
    visuals.window_fill = palette.window;
    visuals.extreme_bg_color = palette.window;
    visuals.faint_bg_color = palette.view;
    visuals.window_stroke = Stroke::new( 1.0, palette.border );
    visuals.window_corner_radius = CornerRadius::same( PANEL_RADIUS );
    visuals.menu_corner_radius = CornerRadius::same( PANEL_RADIUS );
    visuals.selection.bg_fill = palette.border;
    visuals.selection.stroke = Stroke::new( 1.0, palette.text );

    let radius  = CornerRadius::same( CONTROL_RADIUS );
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_stroke = Stroke::new( 1.0, palette.border );
    widgets.noninteractive.fg_stroke = Stroke::new( 1.0, palette.text );
    let states =
    [
        ( &mut widgets.inactive, Color32::TRANSPARENT ),
        ( &mut widgets.hovered, palette.hovered ),
        ( &mut widgets.active, palette.border ),
        ( &mut widgets.open, palette.border ),
    ];
    for ( state, fill ) in states
    {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new( 1.0, palette.text );
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
}
