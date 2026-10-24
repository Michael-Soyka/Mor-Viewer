//! Usage: mor-viewer [data_dir | archive.vfs]
//! Without an argument, falls back to the `PATHOLOGIC_DATA` environment variable.

#![ cfg_attr( not( debug_assertions ), windows_subsystem = "windows" ) ]

mod app;
mod audio_viewer;
mod font_viewer;
mod gltf;
mod hex_viewer;
mod i18n;
mod icons;
mod library;
mod map_viewer;
mod model;
mod model_viewer;
mod scene;
mod scene_viewer;
mod preview;
mod settings;
mod texture_viewer;
mod theme;
mod title_bar;
mod viewer;
mod widgets;
mod xml_viewer;

use std::path::PathBuf;

rust_i18n::i18n!( "locales", fallback = "en" );

fn main() -> eframe::Result
{
    let start = std::env::args_os().nth( 1 ).or_else( || std::env::var_os( "PATHOLOGIC_DATA" ) ).map( PathBuf::from );

    let viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size( [ 1280.0, 800.0 ] )
        .with_min_inner_size( [ 640.0, 400.0 ] );
    let viewport = match app_icon()
    {
        Some( icon ) => viewport.with_icon( icon ),
        None => viewport,
    };

    #[ cfg( target_os = "macos" ) ]
    let viewport = viewport.with_fullsize_content_view( true ).with_titlebar_shown( false ).with_title_shown( false );

    #[ cfg( not( target_os = "macos" ) ) ]
    let viewport = viewport.with_decorations( false );

    let options = eframe::NativeOptions { viewport, ..Default::default() };

    eframe::run_native( "Mor Viewer", options, Box::new( |cc| Ok( Box::new( app::ViewerApp::new( cc, start ) ) ) ) )
}

/// The title bar icon, rasterized for the window and the taskbar.
fn app_icon() -> Option<eframe::egui::IconData>
{
    let image = egui_extras::image::load_svg_bytes( include_bytes!( "../assets/icons/app.svg" ), &Default::default() ).ok()?;
    let rgba  = image.pixels.iter().flat_map( |pixel| pixel.to_srgba_unmultiplied() ).collect();

    let [ width, height ] = image.size.map( |side| u32::try_from( side ).unwrap_or( 0 ) );

    let icon = eframe::egui::IconData { rgba, width, height };

    Some( icon )
}
