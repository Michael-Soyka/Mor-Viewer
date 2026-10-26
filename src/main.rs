//! Usage: mor-viewer [data_dir | archive.vfs]
//! Without an argument, falls back to the `PATHOLOGIC_DATA` environment variable.

#![ cfg_attr( not( debug_assertions ), windows_subsystem = "windows" ) ]

mod app;
mod data;
mod i18n;
mod render;
mod settings;
mod ui;
mod viewer;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui_wgpu::{ self, wgpu };

rust_i18n::i18n!( "locales", fallback = "en" );

/// Also names the directory eframe keeps the settings in.
const APP_NAME: &str = "Mor Viewer";

/// The one graphics API of each platform.
#[ cfg( target_os = "macos" ) ]
const BACKENDS: wgpu::Backends = wgpu::Backends::METAL;

#[ cfg( not( target_os = "macos" ) ) ]
const BACKENDS: wgpu::Backends = wgpu::Backends::VULKAN;

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

    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = BACKENDS;
    setup.device_descriptor = Arc::new( device_descriptor );

    let wgpu_options = egui_wgpu::WgpuConfiguration { wgpu_setup: egui_wgpu::WgpuSetup::CreateNew( setup ), ..Default::default() };
    let options      = eframe::NativeOptions { viewport, wgpu_options, ..Default::default() };

    eframe::run_native( APP_NAME, options, Box::new( |cc| Ok( Box::new( app::ViewerApp::new( cc, start ) ) ) ) )
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

/// The title bar icon, rasterized for the window and the taskbar.
fn app_icon() -> Option<eframe::egui::IconData>
{
    let image = egui_extras::image::load_svg_bytes( include_bytes!( "../assets/icons/app.svg" ), &Default::default() ).ok()?;
    let rgba  = image.pixels.iter().flat_map( |pixel| pixel.to_srgba_unmultiplied() ).collect();

    let [ width, height ] = image.size.map( |side| u32::try_from( side ).unwrap_or( 0 ) );

    let icon = eframe::egui::IconData { rgba, width, height };

    Some( icon )
}
