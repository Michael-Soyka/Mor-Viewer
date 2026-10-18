use std::path::{ Path, PathBuf };

use eframe::egui::emath::GuiRounding;
use eframe::egui::{
    self, Color32, ColorImage, Key, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, TextStyle, TextureHandle, TextureOptions, Vec2,
};
use morft::{ Font, Glyph };
use mortex::{ Image, Texture };
use morvfs::{ Entry, Vfs };

use crate::preview::{ self, Channels, Preview };

const CHECKER_CELL:         f32       = 8.0;
const ZOOM_MIN:             f32       = 0.05;
const ZOOM_MAX:             f32       = 16.0;
const FIT_MAX:              f32       = 8.0;
const FONT_SCALE_MIN:       f32       = 1.0;
const FONT_SCALE_MAX:       f32       = 8.0;
const GLYPH_CELL_MIN_WIDTH: f32       = 40.0;
const MODEL_EXTENSIONS:     [&str; 4] = [ "mesh", "mms", "mma", "lsh" ];

pub struct ViewerApp
{
    archives:           Vec<PathBuf>,
    vfs:                Option<Vfs>,
    entries:            Vec<Entry>,
    filter:             String,
    visible:            Vec<usize>,
    selected:           Option<usize>,
    scroll_to_selected: bool,
    list_offset:        f32,
    list_height:        f32,
    current:            Option<Current>,
    checker:            TextureHandle,
    status:             String,
}

struct Current
{
    path:    String,
    data:    Vec<u8>,
    preview: Preview,
    view:    Option<View>,
}

enum View
{
    Texture( TextureView ),
    Font( FontView ),
}

struct TextureView
{
    face:     u32,
    mip:      u32,
    channels: Channels,
    image:    Image,
    handle:   TextureHandle,
    zoom:     f32,
    fit:      bool,
}

impl TextureView
{
    fn new( ctx: &egui::Context, texture: &Texture ) -> Result<Self, String>
    {
        let image    = texture.decode( 0, 0 ).map_err( |err| err.to_string() )?;
        let channels = Channels::Rgba;
        let handle   = upload( ctx, &image, channels );

        Ok( Self { face: 0, mip: 0, channels, image, handle, zoom: 1.0, fit: true } )
    }

    fn redecode( &mut self, ctx: &egui::Context, texture: &Texture ) -> Result<(), String>
    {
        self.image = texture.decode( self.face, self.mip ).map_err( |err| err.to_string() )?;
        self.handle = upload( ctx, &self.image, self.channels );

        Ok( () )
    }
}

struct FontView
{
    sample: TextureHandle,

    /// In font file order; `None` for empty glyphs.
    glyphs: Vec<Option<TextureHandle>>,

    /// Largest glyph box in font pixels.
    cell: Vec2,

    scale: f32,
}

impl FontView
{
    fn new( ctx: &egui::Context, font: &Font ) -> Self
    {
        let ( width, height, coverage ) = preview::render_line( font, preview::FONT_SAMPLE );
        let sample = upload_coverage( ctx, width, height, &coverage );

        let glyphs = font
            .glyphs()
            .map(
                |( _, glyph )|
                {
                    let width = to_usize( glyph.width );
                    ( width > 0 ).then( || upload_coverage( ctx, width, to_usize( glyph.height ), &glyph.coverage ) )
                }
            )
            .collect();

        let widest = font.glyphs().map( |( _, glyph )| glyph.width.max( glyph.advance ) ).max().unwrap_or( 0 );
        let cell   = Vec2::new( to_f32( to_usize( widest ) ), to_f32( to_usize( font.glyph_height() ) ) );

        Self { sample, glyphs, cell, scale: 2.0 }
    }
}

/// White texture with coverage in alpha, tinted when painted.
fn upload_coverage( ctx: &egui::Context, width: usize, height: usize, coverage: &[u8] ) -> TextureHandle
{
    let pixels = coverage.iter().map( |&alpha| Color32::from_white_alpha( alpha ) ).collect();

    ctx.load_texture( "glyph", ColorImage::new( [ width, height ], pixels ), TextureOptions::NEAREST )
}

fn upload( ctx: &egui::Context, image: &Image, channels: Channels ) -> TextureHandle
{
    let size  = [ to_usize( image.width ), to_usize( image.height ) ];
    let color = ColorImage::from_rgba_unmultiplied( size, &channels.apply( image ) );

    ctx.load_texture( "preview", color, TextureOptions::NEAREST )
}

fn to_usize( value: u32 ) -> usize
{
    usize::try_from( value ).unwrap_or( usize::MAX )
}

/// egui lays out in f32; there is no lossless conversion from usize.
fn to_f32( value: usize ) -> f32
{
    value as f32
}

fn file_name( path: &Path ) -> String
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

impl ViewerApp
{
    pub fn new( cc: &eframe::CreationContext<'_>, start: Option<PathBuf> ) -> Self
    {
        let ( light, dark ) = ( Color32::from_gray( 200 ), Color32::from_gray( 150 ) );
        let checker = ColorImage::new( [ 2, 2 ], vec![ light, dark, dark, light ] );

        let mut app = Self
        {
            archives:           Vec::new(),
            vfs:                None,
            entries:            Vec::new(),
            filter:             String::new(),
            visible:            Vec::new(),
            selected:           None,
            scroll_to_selected: false,
            list_offset:        0.0,
            list_height:        0.0,
            current:            None,
            checker:            cc.egui_ctx.load_texture( "checker", checker, TextureOptions::NEAREST_REPEAT ),
            status:             "Откройте папку data игры или архив .vfs".to_owned(),
        };

        if let Some( path ) = start
        {
            if path.is_dir()
            {
                app.on_open_folder( &path );
            }
            else
            {
                app.on_open_archive( &path );
            }
        }

        app
    }

    fn on_open_folder( &mut self, dir: &Path )
    {
        match list_archives( dir )
        {
            Ok( archives ) if archives.is_empty() => self.status = format!( "В {} нет архивов .vfs", dir.display() ),
            Ok( archives ) =>
            {
                let first = archives
                    .iter()
                    .find( |path| file_name( path ).eq_ignore_ascii_case( "Textures.vfs" ) )
                    .or( archives.first() )
                    .cloned();

                self.archives = archives;
                if let Some( path ) = first
                {
                    self.on_open_archive( &path );
                }
            }
            Err( err ) => self.status = format!( "{}: {err}", dir.display() ),
        }
    }

    fn on_open_archive( &mut self, path: &Path )
    {
        let vfs = match Vfs::open( path )
        {
            Ok( vfs ) => vfs,
            Err( err ) =>
            {
                self.status = format!( "{}: {err}", path.display() );
                return;
            }
        };

        if !self.archives.iter().any( |known| known == path )
        {
            self.archives = path.parent().and_then( |dir| list_archives( dir ).ok() ).unwrap_or_else( || vec![ path.to_owned() ] );
        }
        self.status = format!( "{}: {} файлов", file_name( path ), vfs.len() );
        self.entries = vfs.entries().cloned().collect();
        self.vfs = Some( vfs );
        self.selected = None;
        self.current = None;
        self.list_offset = 0.0;
        self.on_filter_change();
    }

    fn on_filter_change( &mut self )
    {
        let needle = self.filter.to_lowercase();

        self.visible = ( 0..self.entries.len() )
            .filter( |&index| needle.is_empty() || self.entries[ index ].path.to_lowercase().contains( &needle ) )
            .collect();
    }

    fn on_select( &mut self, ctx: &egui::Context, index: usize )
    {
        self.selected = Some( index );
        let ( Some( vfs ), Some( entry ) ) = ( &self.vfs, self.entries.get( index ) ) else { return };
        let path = entry.path.clone();

        let data = match vfs.read_entry( entry )
        {
            Ok( data ) => data,
            Err( err ) =>
            {
                self.current = None;
                self.status = format!( "{path}: {err}" );
                return;
            }
        };
        let loaded = Preview::new( &path, &data ).and_then(
            |preview|
            {
                let view = match &preview
                {
                    Preview::Texture( texture ) => Some( View::Texture( TextureView::new( ctx, texture )? ) ),
                    Preview::Font( font ) => Some( View::Font( FontView::new( ctx, font ) ) ),
                    Preview::Text( _ ) | Preview::Binary => None,
                };

                Ok( ( preview, view ) )
            }
        );
        match loaded
        {
            Ok( ( preview, view ) ) =>
            {
                self.status = format!( "{path}: {} байт", data.len() );
                self.current = Some( Current { path, data, preview, view } );
            }
            Err( err ) =>
            {
                self.status = format!( "{path}: {err}" );
                self.current = Some( Current { path, data, preview: Preview::Binary, view: None } );
            }
        }
    }

    fn on_step_selection( &mut self, ctx: &egui::Context, forward: bool )
    {
        let position = self.selected.and_then( |index| self.visible.iter().position( |&visible| visible == index ) );
        let next = match ( position, forward )
        {
            ( None, _ ) => 0,
            ( Some( position ), true ) => ( position + 1 ).min( self.visible.len().saturating_sub( 1 ) ),
            ( Some( position ), false ) => position.saturating_sub( 1 ),
        };

        if let Some( &index ) = self.visible.get( next )
            && Some( index ) != self.selected
        {
            self.scroll_to_selected = true;
            self.on_select( ctx, index );
        }
    }

    fn on_extract( &mut self )
    {
        let Some( current ) = &self.current else { return };
        let name = current.path.rsplit( '/' ).next().unwrap_or_default();
        let Some( target ) = rfd::FileDialog::new().set_file_name( name ).save_file() else { return };

        self.status = match std::fs::write( &target, &current.data )
        {
            Ok( () ) => format!( "Сохранено: {}", target.display() ),
            Err( err ) => format!( "{}: {err}", target.display() ),
        };
    }

    fn on_save_png( &mut self )
    {
        let Some( current ) = &self.current else { return };
        let Some( View::Texture( view ) ) = &current.view else { return };
        let stem   = current.path.rsplit( '/' ).next().unwrap_or_default().trim_end_matches( ".tex" );
        let dialog = rfd::FileDialog::new().add_filter( "PNG", &[ "png" ] ).set_file_name( format!( "{stem}.png" ) );
        let Some( target ) = dialog.save_file() else { return };
        let rgba  = view.channels.apply( &view.image );
        let saved = image::save_buffer( &target, &rgba, view.image.width, view.image.height, image::ExtendedColorType::Rgba8 );

        self.status = match saved
        {
            Ok( () ) => format!( "Сохранено: {}", target.display() ),
            Err( err ) => format!( "{}: {err}", target.display() ),
        };
    }

    fn toolbar( &mut self, ui: &mut egui::Ui )
    {
        ui.horizontal(
            |ui|
            {
                if ui.button( "Папка data…" ).clicked()
                    && let Some( dir ) = rfd::FileDialog::new().pick_folder()
                {
                    self.on_open_folder( &dir );
                }
                if ui.button( "Архив .vfs…" ).clicked()
                    && let Some( path ) = rfd::FileDialog::new().add_filter( "VFS", &[ morvfs::EXTENSION ] ).pick_file()
                {
                    self.on_open_archive( &path );
                }

                let current       = self.vfs.as_ref().map( |vfs| vfs.path().to_owned() );
                let mut chosen    = None;
                let selected_text = current.as_deref().map( file_name ).unwrap_or_else( || "—".to_owned() );

                egui::ComboBox::from_id_salt( "archive" ).selected_text( selected_text ).width( 180.0 ).show_ui(
                    ui,
                    |ui|
                    {
                        for archive in &self.archives
                        {
                            let is_current = current.as_deref() == Some( archive.as_path() );
                            if ui.selectable_label( is_current, file_name( archive ) ).clicked() && !is_current
                            {
                                chosen = Some( archive.clone() );
                            }
                        }
                    }
                );
                if let Some( path ) = chosen
                {
                    self.on_open_archive( &path );
                }

                ui.separator();
                if ui.add_enabled( self.current.is_some(), egui::Button::new( "Извлечь файл…" ) ).clicked()
                {
                    self.on_extract();
                }
            }
        );
    }

    fn file_list( &mut self, ui: &mut egui::Ui )
    {
        let filter_id = ui.make_persistent_id( "filter" );
        let filter    = TextEdit::singleline( &mut self.filter ).id( filter_id ).hint_text( "Фильтр, например .tex" ).desired_width( f32::INFINITY );
        if ui.add( filter ).changed()
        {
            self.on_filter_change();
        }
        ui.label( format!( "{} из {}", self.visible.len(), self.entries.len() ) );
        ui.separator();

        let focus = ui.ctx().memory( |memory| memory.focused() );
        if focus.is_none_or( |id| id == filter_id )
        {
            let ( down, up ) = ui.input( |input| ( input.key_pressed( Key::ArrowDown ), input.key_pressed( Key::ArrowUp ) ) );
            if down || up
            {
                self.on_step_selection( ui.ctx(), down );
            }
        }

        let row_height = ui.spacing().interact_size.y;
        let pitch      = row_height + ui.spacing().item_spacing.y;
        let mut area   = ScrollArea::vertical().auto_shrink( false );

        if std::mem::take( &mut self.scroll_to_selected )
            && let Some( position ) = self.selected.and_then( |index| self.visible.iter().position( |&visible| visible == index ) )
        {
            let top = to_f32( position ) * pitch;
            if top < self.list_offset
            {
                area = area.vertical_scroll_offset( top );
            }
            else if top + pitch > self.list_offset + self.list_height
            {
                area = area.vertical_scroll_offset( top + pitch - self.list_height );
            }
        }

        let mut clicked = None;

        let output = area.show_rows(
            ui,
            row_height,
            self.visible.len(),
            |ui, rows|
            {
                for &index in &self.visible[ rows ]
                {
                    let entry = &self.entries[ index ];
                    let label = ui.selectable_label( self.selected == Some( index ), &entry.path );
                    if label.on_hover_text( format!( "{} байт", entry.size ) ).clicked()
                    {
                        clicked = Some( index );
                    }
                }
            }
        );
        self.list_offset = output.state.clamped_offset().y;
        self.list_height = output.inner_rect.height();
        if let Some( index ) = clicked
        {
            self.on_select( ui.ctx(), index );
        }
    }

    fn content( &mut self, ui: &mut egui::Ui )
    {
        let Some( current ) = &mut self.current else
        {
            ui.centered_and_justified( |ui| ui.label( "Выберите файл слева" ) );
            return;
        };

        let mut save_png = false;
        let mut error    = None;

        match ( &current.preview, &mut current.view )
        {
            ( Preview::Texture( texture ), Some( View::Texture( view ) ) ) =>
            {
                let controls = texture_controls( ui, texture, view, &current.data );
                save_png = controls.save_png;
                if controls.redecode
                {
                    error = view.redecode( ui.ctx(), texture ).err();
                }
                texture_canvas( ui, view, &self.checker );
            }
            ( Preview::Font( font ), Some( View::Font( view ) ) ) => font_view( ui, font, view, current.data.len() ),
            ( Preview::Text( text ), _ ) =>
            {
                ScrollArea::both().auto_shrink( false ).show(
                    ui,
                    |ui|
                    {
                        ui.add( TextEdit::multiline( &mut text.as_str() ).code_editor().desired_width( f32::INFINITY ) );
                    }
                );
            }
            ( Preview::Texture( _ ) | Preview::Font( _ ) | Preview::Binary, _ ) =>
            {
                if MODEL_EXTENSIONS.iter().any( |ext| preview::has_extension( &current.path, ext ) )
                {
                    ui.label( RichText::new( "Формат моделей пока не поддерживается парсерами — показан hex." ).weak() );
                }
                hex_view( ui, &current.data );
            }
        }
        if let Some( err ) = error
        {
            self.status = err;
        }
        if save_png
        {
            self.on_save_png();
        }
    }
}

struct ControlsResult
{
    redecode: bool,
    save_png: bool,
}

fn texture_controls( ui: &mut egui::Ui, texture: &Texture, view: &mut TextureView, data: &[u8] ) -> ControlsResult
{
    let mut result = ControlsResult { redecode: false, save_png: false };

    let info = format!(
        "{:?} · {}×{} · мипов: {} · граней: {} · {} байт",
        texture.format(),
        texture.width(),
        texture.height(),
        texture.mip_count(),
        texture.face_count(),
        data.len(),
    );
    ui.label( info );
    ui.horizontal(
        |ui|
        {
            if texture.is_cubemap()
            {
                const FACES: [&str; 6] = [ "+X", "−X", "+Y", "−Y", "+Z", "−Z" ];

                ui.label( "Грань" );
                for ( face, name ) in ( 0.. ).zip( FACES )
                {
                    result.redecode |= ui.selectable_value( &mut view.face, face, name ).changed();
                }
                ui.separator();
            }
            if texture.mip_count() > 1
            {
                ui.label( "Мип" );
                let mip = egui::Slider::new( &mut view.mip, 0..=texture.mip_count() - 1 );
                result.redecode |= ui.add( mip ).changed();
                ui.label( format!( "{}×{}", view.image.width, view.image.height ) );
                ui.separator();
            }
            for channels in Channels::ALL
            {
                result.redecode |= ui.selectable_value( &mut view.channels, channels, channels.label() ).changed();
            }
            ui.separator();
            let zoom = egui::Slider::new( &mut view.zoom, ZOOM_MIN..=ZOOM_MAX ).logarithmic( true ).text( "масштаб" );
            if ui.add( zoom ).changed()
            {
                view.fit = false;
            }
            if ui.button( "Вписать" ).clicked()
            {
                view.fit = true;
            }
            if ui.button( "1:1" ).clicked()
            {
                view.fit = false;
                view.zoom = 1.0;
            }
            ui.separator();
            result.save_png = ui.button( "Сохранить PNG…" ).clicked();
        }
    );
    ui.separator();

    result
}

fn texture_canvas( ui: &mut egui::Ui, view: &mut TextureView, checker: &TextureHandle )
{
    let image_size = view.handle.size_vec2();

    if view.fit
    {
        let available = ui.available_size() - Vec2::splat( ui.spacing().scroll.bar_width * 2.0 );
        view.zoom = ( available.x / image_size.x ).min( available.y / image_size.y ).clamp( ZOOM_MIN, FIT_MAX );
    }

    let hovered    = ui.rect_contains_pointer( ui.available_rect_before_wrap() );
    let zoom_delta = ui.input( |input| input.zoom_delta() );
    if hovered && zoom_delta != 1.0
    {
        view.zoom = ( view.zoom * zoom_delta ).clamp( ZOOM_MIN, ZOOM_MAX );
        view.fit = false;
    }

    let size = image_size * view.zoom;

    ScrollArea::both().auto_shrink( false ).show(
        ui,
        |ui|
        {
            let ( rect, _ ) = ui.allocate_exact_size( size, Sense::hover() );
            let painter    = ui.painter();
            let checker_uv = Rect::from_min_size( Pos2::ZERO, size / ( CHECKER_CELL * 2.0 ) );
            painter.image( checker.id(), rect, checker_uv, Color32::WHITE );
            let full_uv = Rect::from_min_max( Pos2::ZERO, Pos2::new( 1.0, 1.0 ) );
            painter.image( view.handle.id(), rect, full_uv, Color32::WHITE );
        }
    );
}

fn font_view( ui: &mut egui::Ui, font: &Font, view: &mut FontView, size: usize )
{
    let info = format!(
        "глифов: {} · пар кернинга: {} · высота строки: {} · высота глифа: {} · {size} байт",
        font.glyphs().len(),
        font.kerning_pairs().len(),
        font.line_height(),
        font.glyph_height(),
    );
    ui.label( info );
    ui.horizontal(
        |ui|
        {
            let scale = egui::Slider::new( &mut view.scale, FONT_SCALE_MIN..=FONT_SCALE_MAX ).step_by( 1.0 ).text( "масштаб" );
            ui.add( scale );
        }
    );
    ui.separator();

    let tint    = ui.visuals().strong_text_color();
    let full_uv = Rect::from_min_max( Pos2::ZERO, Pos2::new( 1.0, 1.0 ) );

    ScrollArea::vertical().auto_shrink( false ).show(
        ui,
        |ui|
        {
            let ( rect, _ ) = ui.allocate_exact_size( view.sample.size_vec2() * view.scale, Sense::hover() );
            ui.painter().image( view.sample.id(), rect, full_uv, tint );
            ui.separator();
            ui.horizontal_wrapped(
                |ui|
                {
                    for ( ( ch, glyph ), handle ) in font.glyphs().zip( &view.glyphs )
                    {
                        glyph_cell( ui, ch, glyph, handle.as_ref(), view, tint );
                    }
                }
            );
        }
    );
}

fn glyph_cell( ui: &mut egui::Ui, ch: char, glyph: &Glyph, handle: Option<&TextureHandle>, view: &FontView, tint: Color32 )
{
    let code = u32::from( ch );

    let response = ui.vertical(
        |ui|
        {
            let mut size = view.cell * view.scale;
            size.x = size.x.max( GLYPH_CELL_MIN_WIDTH );
            let ( rect, _ ) = ui.allocate_exact_size( size, Sense::hover() );
            let painter = ui.painter();
            painter.rect_stroke( rect, 0.0, Stroke::new( 1.0, ui.visuals().weak_text_color() ), StrokeKind::Inside );
            if let Some( handle ) = handle
            {
                let glyph_rect = Rect::from_center_size( rect.center(), handle.size_vec2() * view.scale );
                let full_uv    = Rect::from_min_max( Pos2::ZERO, Pos2::new( 1.0, 1.0 ) );
                painter.image( handle.id(), glyph_rect.round_to_pixels( painter.pixels_per_point() ), full_uv, tint );
            }
            ui.label( RichText::new( format!( "{code:04X}" ) ).monospace().small() );
        }
    );

    let hint = format!(
        "{ch:?} U+{code:04X} · {}×{} · смещение {}, {} · шаг {}",
        glyph.width, glyph.height, glyph.offset_x, glyph.offset_y, glyph.advance,
    );
    response.response.on_hover_text( hint );
}

fn hex_view( ui: &mut egui::Ui, data: &[u8] )
{
    let row_height = ui.text_style_height( &TextStyle::Monospace );
    let rows       = data.len().div_ceil( preview::HEX_ROW );

    ScrollArea::both().auto_shrink( false ).show_rows(
        ui,
        row_height,
        rows,
        |ui, range|
        {
            for row in range
            {
                ui.label( RichText::new( preview::hex_row( data, row ) ).monospace() );
            }
        }
    );
}

impl eframe::App for ViewerApp
{
    fn ui( &mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame )
    {
        egui::Panel::top( "toolbar" ).show( ui, |ui| self.toolbar( ui ) );
        egui::Panel::bottom( "status" ).show( ui, |ui| ui.label( &self.status ) );
        egui::Panel::left( "files" ).resizable( true ).default_size( 320.0 ).show( ui, |ui| self.file_list( ui ) );
        egui::CentralPanel::default().show( ui, |ui| self.content( ui ) );
    }
}
