use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use eframe::egui_wgpu::RenderState;

use rust_i18n::t;
use crate::model_viewer::{ Camera, ModelViewer };
use crate::scene::{ self, LoadedScene, Progress };
use crate::settings::ViewerSettings;
use crate::theme::Palette;
use crate::viewer::Sections;

const PROGRESS_REPAINT: Duration = Duration::from_millis( 100 );

/// A whole level, read on a thread of its own and shown in the model viewer once ready.
pub struct SceneViewer
{
    state:    RenderState,
    progress: Arc<Progress>,
    task:     Option<JoinHandle<Result<LoadedScene, String>>>,
    viewer:   Option<ModelViewer>,
}

impl SceneViewer
{
    pub fn new( state: &RenderState, archives: Vec<PathBuf>, file: String ) -> Self
    {
        let progress = Arc::new( Progress::default() );
        let shared   = Arc::clone( &progress );
        let task     = std::thread::spawn( move || scene::load( &archives, &file, &shared ) );

        Self { state: state.clone(), progress, task: Some( task ), viewer: None }
    }

    /// Returns the loading error, once.
    pub fn show( &mut self, sections: &mut Sections, palette: &Palette, view: &ViewerSettings ) -> Option<String>
    {
        let mut error = None;

        if let Some( task ) = self.task.take_if( |task| task.is_finished() )
        {
            match task.join()
            {
                Ok( Ok( loaded ) ) => self.viewer = Some( viewer( &self.state, loaded ) ),
                Ok( Err( err ) ) => error = Some( err ),
                Err( _ ) => error = Some( t!( "scene.failed" ).into_owned() ),
            }
        }

        if let Some( viewer ) = &mut self.viewer
        {
            viewer.show( sections, palette, view );
        }
        else if self.task.is_some()
        {
            let done    = self.progress.done.load( Ordering::Relaxed );
            let total   = self.progress.total.load( Ordering::Relaxed );
            let message = t!( "scene.loading", done = done, total = total );
            let view    = &mut sections.view;
            view.centered_and_justified( |ui| ui.label( egui::RichText::new( message ).color( palette.menu_text ) ) );
            view.ctx().request_repaint_after( PROGRESS_REPAINT );
        }

        error
    }
}

fn viewer( state: &RenderState, loaded: LoadedScene ) -> ModelViewer
{
    let camera = match loaded.start
    {
        Some( ( position, direction ) ) => Camera::standing( position, direction ),
        None => Camera::framing( &loaded.model ),
    };

    ModelViewer::from_decoded( state, &loaded.model, &loaded.textures, loaded.missing, camera ).with_lightmaps().with_ambient( loaded.ambient ).with_lights( loaded.lights ).with_paths( loaded.paths.as_ref() )
}
