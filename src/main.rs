//! Usage: mor-viewer [data_dir | archive.vfs]
//! Without an argument, falls back to the `PATHOLOGIC_DATA` environment variable.

#![ cfg_attr( not( debug_assertions ), windows_subsystem = "windows" ) ]

mod app;
mod preview;

use std::path::PathBuf;

fn main() -> eframe::Result
{
    let start = std::env::args_os().nth( 1 ).or_else( || std::env::var_os( "PATHOLOGIC_DATA" ) ).map( PathBuf::from );

    let options = eframe::NativeOptions
    {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size( [ 1280.0, 800.0 ] ),
        ..Default::default()
    };

    eframe::run_native( "Mor Viewer", options, Box::new( |cc| Ok( Box::new( app::ViewerApp::new( cc, start ) ) ) ) )
}
