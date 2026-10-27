use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use eframe::egui;
use eframe::egui_wgpu::RenderState;

use rust_i18n::t;
use crate::data::library::Library;
use crate::render::scene::{ self, LoadedScene, Progress };
use crate::settings::ViewerSettings;
use crate::ui::theme::Palette;
use crate::viewer::model::ModelViewer;
use crate::viewer::model::camera::Camera;
use crate::viewer::Sections;

const PROGRESS_REPAINT: Duration = Duration::from_millis( 100 );

/// A whole level, read on a thread of its own and shown in the model viewer once ready.
pub struct SceneViewer
{
    state:    RenderState,
    progress: Arc<Progress>,
    task:     Option<Task>,
    viewer:   Option<ModelViewer>,
}

/// The level loader: a thread of its own.
#[ cfg( not( target_arch = "wasm32" ) ) ]
struct Task( std::thread::JoinHandle<Result<LoadedScene, String>> );

/// The level loader: the web has no threads, so the level is loaded at once.
#[ cfg( target_arch = "wasm32" ) ]
struct Task( Result<LoadedScene, String> );

impl SceneViewer
{
    pub fn new( state: &RenderState, library: Library, file: String ) -> Self
    {
        let progress = Arc::new( Progress::default() );
        let task     = Task::spawn( library, file, Arc::clone( &progress ) );

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
                Ok( loaded ) => self.viewer = Some( viewer( &self.state, loaded ) ),
                Err( err ) => error = Some( err ),
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

#[ cfg( not( target_arch = "wasm32" ) ) ]
impl Task
{
    fn spawn( library: Library, file: String, progress: Arc<Progress> ) -> Self
    {
        Self( std::thread::spawn( move || scene::load( &library, &file, &progress ) ) )
    }

    fn is_finished( &self ) -> bool
    {
        self.0.is_finished()
    }

    fn join( self ) -> Result<LoadedScene, String>
    {
        self.0.join().unwrap_or_else( |_| Err( t!( "scene.failed" ).into_owned() ) )
    }
}

#[ cfg( target_arch = "wasm32" ) ]
impl Task
{
    fn spawn( library: Library, file: String, progress: Arc<Progress> ) -> Self
    {
        Self( scene::load( &library, &file, &progress ) )
    }

    fn is_finished( &self ) -> bool
    {
        true
    }

    fn join( self ) -> Result<LoadedScene, String>
    {
        self.0
    }
}

fn viewer( state: &RenderState, loaded: LoadedScene ) -> ModelViewer
{
    let camera = match loaded.start
    {
        Some( ( position, direction ) ) => Camera::standing( position, direction ),
        None => Camera::framing( &loaded.model ),
    };

    ModelViewer::from_decoded( state, &loaded.model, &loaded.textures, loaded.missing, camera ).with_lightmaps().with_occlusion().with_ambient( loaded.ambient ).with_lights( loaded.lights ).with_paths( loaded.paths.as_ref() ).with_sky( loaded.sky.as_ref() ).with_street_lights( loaded.street_lights, &loaded.lights_off )
}
