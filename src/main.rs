//! Usage: mor-viewer [data_dir | archive.vfs | game_file]
//! Unpacked game files open only with the `load_unpacked` setting, see [`settings::Settings`].
//! Without an argument, falls back to the `PATHOLOGIC_DATA` environment variable.

#![ cfg_attr( not( debug_assertions ), windows_subsystem = "windows" ) ]

mod app;
mod data;
mod i18n;
mod render;
mod settings;
mod ui;
mod viewer;

#[ cfg( target_arch = "wasm32" ) ]
mod web;

use std::sync::Arc;

use eframe::egui_wgpu::{ self, wgpu };

rust_i18n::i18n!( "locales", fallback = "en" );

/// Also names the directory eframe keeps the settings in.
#[ cfg( not( target_arch = "wasm32" ) ) ]
const APP_NAME: &str = "Mor Viewer";

/// The one graphics API of each platform; the web's is WebGPU, as the culling needs compute shaders.
#[ cfg( target_os = "macos" ) ]
const BACKENDS: wgpu::Backends = wgpu::Backends::METAL;

#[ cfg( target_arch = "wasm32" ) ]
const BACKENDS: wgpu::Backends = wgpu::Backends::BROWSER_WEBGPU;

#[ cfg( not( any( target_os = "macos", target_arch = "wasm32" ) ) ) ]
const BACKENDS: wgpu::Backends = wgpu::Backends::VULKAN;

/// Ids of the page's elements, see `index.html`.
#[ cfg( target_arch = "wasm32" ) ]
const CANVAS_ID: &str = "mor_viewer";

#[ cfg( target_arch = "wasm32" ) ]
const STATUS_ID: &str = "status";

#[ cfg( target_arch = "wasm32" ) ]
fn main()
{
    use wasm_bindgen::JsCast;

    let document = web_sys::window().and_then( |window| window.document() ).expect( "the app runs in a page" );
    let canvas   = document.get_element_by_id( CANVAS_ID ).and_then( |element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok() ).expect( "the page has the canvas" );
    let options  = eframe::WebOptions { wgpu_options: wgpu_options(), ..Default::default() };

    let task = async move
    {
        let started = eframe::WebRunner::new().start( canvas, options, Box::new( |cc| Ok( Box::new( app::ViewerApp::new( cc, None ) ) ) ) ).await;
        let status  = document.get_element_by_id( STATUS_ID );
        match ( started, status )
        {
            ( Ok( () ), Some( status ) ) => status.remove(),
            ( Err( err ), Some( status ) ) => status.set_text_content( Some( &web::error_text( &err ) ) ),
            ( _, None ) => {}
        }
    };

    wasm_bindgen_futures::spawn_local( task );
}

#[ cfg( not( target_arch = "wasm32" ) ) ]
fn main() -> eframe::Result
{
    let start = std::env::args_os().nth( 1 ).or_else( || std::env::var_os( "PATHOLOGIC_DATA" ) ).map( std::path::PathBuf::from );

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

    let options = eframe::NativeOptions { viewport, wgpu_options: wgpu_options(), ..Default::default() };

    eframe::run_native( APP_NAME, options, Box::new( |cc| Ok( Box::new( app::ViewerApp::new( cc, start ) ) ) ) )
}

/// wgpu on the platform's graphics API, with the device the renderer asks for.
fn wgpu_options() -> egui_wgpu::WgpuConfiguration
{
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = BACKENDS;
    setup.device_descriptor = Arc::new( device_descriptor );

    let options = egui_wgpu::WgpuConfiguration { wgpu_setup: egui_wgpu::WgpuSetup::CreateNew( setup ), ..Default::default() };

    options
}

/// egui's device, plus the indirect draw count that lets the culling skip hidden draws.
fn device_descriptor( adapter: &wgpu::Adapter ) -> wgpu::DeviceDescriptor<'static>
{
    // egui's own limit: a depth buffer the size of a 4K+ display.
    let limits = wgpu::Limits { max_texture_dimension_2d: 8192, ..wgpu::Limits::default() };
    let descriptor = wgpu::DeviceDescriptor
    {
        label:             Some( "mor viewer" ),
        required_features: adapter.features() & wgpu::Features::MULTI_DRAW_INDIRECT_COUNT,
        required_limits:   limits,
        ..Default::default()
    };

    descriptor
}

/// The icon on its tile, rasterized for the window and the taskbar.
#[ cfg( not( target_arch = "wasm32" ) ) ]
fn app_icon() -> Option<eframe::egui::IconData>
{
    let image = egui_extras::image::load_svg_bytes( include_bytes!( "../assets/icons/app-tile.svg" ), &Default::default() ).ok()?;
    let rgba  = image.pixels.iter().flat_map( |pixel| pixel.to_srgba_unmultiplied() ).collect();

    let [ width, height ] = image.size.map( |side| u32::try_from( side ).unwrap_or( 0 ) );

    let icon = eframe::egui::IconData { rgba, width, height };

    Some( icon )
}
