use eframe::egui::emath::GuiRounding;
use eframe::egui::{ self, Color32, ColorImage, Pos2, Rect, ScrollArea, Sense, TextureHandle, TextureOptions };
use mortex::{ Image, Texture };

use rust_i18n::t;
use crate::data::preview::{ self, Channels };
use crate::ui::theme::Palette;
use crate::ui::widgets::{ self, Zoom };
use crate::viewer::Sections;

const ZOOM_MIN:        f32       = 0.05;
const ZOOM_MAX:        f32       = 16.0;
const FIT_MAX:         f32       = 8.0;
const CHECKER_CELL:    f32       = 8.0;
const CHANNEL_LETTERS: [&str; 4] = [ "R", "G", "B", "A" ];
const FACES:           [&str; 6] = [ "+X", "−X", "+Y", "−Y", "+Z", "−Z" ];

pub struct TextureViewer
{
    /// `None` for plain images, which have no mips or faces.
    texture: Option<Texture>,

    face:     u32,
    mip:      u32,
    channels: Channels,
    image:    Image,
    handle:   TextureHandle,
    zoom:     f32,
    fit:      bool,

    /// Until the first frame: shrink large images to the view, keep small ones at 100%.
    initial: bool,
}

impl TextureViewer
{
    pub fn new( ctx: &egui::Context, texture: Texture ) -> Result<Self, String>
    {
        let image      = texture.decode( 0, 0 ).map_err( |err| err.to_string() )?;
        let mut viewer = Self::from_image( ctx, image );
        viewer.texture = Some( texture );

        Ok( viewer )
    }

    pub fn from_image( ctx: &egui::Context, image: Image ) -> Self
    {
        let channels = Channels::ALL;
        let handle   = upload( ctx, &image, channels );

        Self { texture: None, face: 0, mip: 0, channels, image, handle, zoom: 1.0, fit: false, initial: true }
    }

    pub fn show( &mut self, sections: &mut Sections, palette: &Palette, checker: &TextureHandle ) -> Option<String>
    {
        let mut error = None;
        let ctx       = sections.view.ctx().clone();

        let mut channels_changed = false;

        for ( index, letter ) in CHANNEL_LETTERS.into_iter().enumerate()
        {
            let toggle = widgets::channel_toggle( &mut sections.action_left, palette, letter, palette.channels[ index ], &mut self.channels.0[ index ] );
            channels_changed |= toggle.changed();
        }

        // Right-to-left: mip ends up rightmost, the cube face to its left.
        let action       = &mut sections.action_right;
        let mut redecode = false;

        if let Some( texture ) = &self.texture
            && texture.mip_count() > 1
        {
            let response = widgets::dropdown_button( action, palette, &t!( "texture.mip", index = self.mip ) );
            egui::Popup::menu( &response ).show(
                |ui|
                {
                    for mip in 0..texture.mip_count()
                    {
                        let size = t!( "texture.mip_size", index = mip, width = ( texture.width() >> mip ).max( 1 ), height = ( texture.height() >> mip ).max( 1 ) );
                        redecode |= ui.selectable_value( &mut self.mip, mip, size ).changed();
                    }
                }
            );
        }
        if let Some( texture ) = &self.texture
            && texture.is_cubemap()
        {
            let selected = FACES.get( to_usize( self.face ) ).copied().unwrap_or_default();
            let response = widgets::dropdown_button( action, palette, &t!( "texture.face", name = selected ) );
            egui::Popup::menu( &response ).show(
                |ui|
                {
                    for ( face, name ) in ( 0.. ).zip( FACES )
                    {
                        redecode |= ui.selectable_value( &mut self.face, face, name ).changed();
                    }
                }
            );
        }

        if redecode && let Some( texture ) = &self.texture
        {
            match texture.decode( self.face, self.mip )
            {
                Ok( image ) =>
                {
                    self.image = image;
                    channels_changed = true;
                }
                Err( err ) => error = Some( err.to_string() ),
            }
        }
        if channels_changed
        {
            self.handle = upload( &ctx, &self.image, self.channels );
        }

        self.canvas( &mut sections.view, checker );

        sections.footer_text( palette, &format!( "{}×{}", self.image.width, self.image.height ) );
        match widgets::zoom_control( &mut sections.footer_right, palette, self.zoom, ( ZOOM_MIN, ZOOM_MAX ), self.fit )
        {
            Zoom::Unchanged => {}
            Zoom::Set( zoom ) =>
            {
                self.zoom = zoom;
                self.fit = false;
            }
            Zoom::Fit => self.fit = true,
        }

        error
    }

    pub fn export_png( &self ) -> Result<Vec<u8>, String>
    {
        let rgba = self.channels.apply( &self.image );

        png( &rgba, self.image.width, self.image.height )
    }

    fn canvas( &mut self, ui: &mut egui::Ui, checker: &TextureHandle )
    {
        let image_size = self.handle.size_vec2();
        let available  = ui.available_size();
        let fitted     = ( available.x / image_size.x ).min( available.y / image_size.y );

        if self.fit
        {
            self.zoom = fitted.clamp( ZOOM_MIN, FIT_MAX );
        }
        else if std::mem::take( &mut self.initial )
        {
            self.zoom = fitted.clamp( ZOOM_MIN, 1.0 );
        }
        let hovered    = ui.rect_contains_pointer( ui.max_rect() );
        let zoom_delta = ui.input( |input| input.zoom_delta() );
        if hovered && zoom_delta != 1.0
        {
            self.zoom = ( self.zoom * zoom_delta ).clamp( ZOOM_MIN, ZOOM_MAX );
            self.fit = false;
        }

        let size = image_size * self.zoom;

        ScrollArea::both().id_salt( "texture" ).auto_shrink( false ).show(
            ui,
            |ui|
            {
                let ( area, _ ) = ui.allocate_exact_size( size.max( ui.available_size() ), Sense::hover() );
                let rect       = Rect::from_center_size( area.center(), size ).round_to_pixels( ui.pixels_per_point() );
                let painter    = ui.painter();
                let checker_uv = Rect::from_min_size( Pos2::ZERO, size / ( CHECKER_CELL * 2.0 ) );
                painter.image( checker.id(), rect, checker_uv, Color32::WHITE );
                let full_uv = Rect::from_min_max( Pos2::ZERO, Pos2::new( 1.0, 1.0 ) );
                painter.image( self.handle.id(), rect, full_uv, Color32::WHITE );
            }
        );
    }
}

fn upload( ctx: &egui::Context, image: &Image, channels: Channels ) -> TextureHandle
{
    let size  = [ to_usize( image.width ), to_usize( image.height ) ];
    let color = ColorImage::from_rgba_unmultiplied( size, &channels.apply( image ) );

    ctx.load_texture( "preview", color, TextureOptions::NEAREST )
}

pub fn to_usize( value: u32 ) -> usize
{
    usize::try_from( value ).unwrap_or( usize::MAX )
}

/// A small RGBA preview for the thumbnail grid: the smallest mip at least `size` wide.
/// Top mip of the first face, or the whole picture for plain image formats.
pub fn decode( path: &str, data: &[u8] ) -> Result<Image, String>
{
    if preview::image_format( path ).is_some()
    {
        return preview::decode_image( path, data );
    }
    let texture = Texture::parse( data ).map_err( |err| err.to_string() )?;

    texture.decode( 0, 0 ).map_err( |err| err.to_string() )
}

pub fn encode_png( image: &Image ) -> Result<Vec<u8>, String>
{
    png( &image.rgba, image.width, image.height )
}

fn png( rgba: &[u8], width: u32, height: u32 ) -> Result<Vec<u8>, String>
{
    let mut bytes = Vec::new();
    let encoder   = image::codecs::png::PngEncoder::new( &mut bytes );
    image::ImageEncoder::write_image( encoder, rgba, width, height, image::ExtendedColorType::Rgba8 ).map_err( |err| err.to_string() )?;

    Ok( bytes )
}

pub fn thumbnail( ctx: &egui::Context, path: &str, data: &[u8], size: u32 ) -> Option<TextureHandle>
{
    let image = if preview::image_format( path ).is_some()
    {
        preview::decode_image( path, data ).ok()?
    }
    else
    {
        let texture = Texture::parse( data ).ok()?;
        let mip     = ( 0..texture.mip_count() ).rev().find( |&mip| ( texture.width() >> mip ) >= size ).unwrap_or( 0 );
        texture.decode( 0, mip ).ok()?
    };
    let color = ColorImage::from_rgba_unmultiplied( [ to_usize( image.width ), to_usize( image.height ) ], &image.rgba );

    Some( ctx.load_texture( "thumbnail", color, TextureOptions::LINEAR ) )
}

pub fn new_checker( ctx: &egui::Context ) -> TextureHandle
{
    let ( light, dark ) = ( Color32::from_gray( 200 ), Color32::from_gray( 150 ) );
    let checker = ColorImage::new( [ 2, 2 ], vec![ light, dark, dark, light ] );

    ctx.load_texture( "checker", checker, TextureOptions::NEAREST_REPEAT )
}
