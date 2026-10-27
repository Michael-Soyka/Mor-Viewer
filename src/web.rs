//! The browser's ways in and out: files are picked in its dialog or dropped on the page, and saved to its downloads.

use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc::{ self, Receiver, Sender };

use eframe::egui;
use wasm_bindgen::{ JsCast, JsValue };

/// A file given by the browser: its name and contents.
pub struct Picked
{
    pub name: String,
    pub data: Arc<[u8]>,
}

/// Files the browser reads frames after they were picked or dropped.
pub struct Picker
{
    sender:   Sender<Vec<Result<Picked, String>>>,
    receiver: Receiver<Vec<Result<Picked, String>>>,
}

impl Default for Picker
{
    fn default() -> Self
    {
        let ( sender, receiver ) = mpsc::channel();

        Self { sender, receiver }
    }
}

impl Picker
{
    /// Shows the dialog; the files come out of [`Picker::take`] once read.
    pub fn pick( &self, ctx: &egui::Context, dialog: rfd::AsyncFileDialog )
    {
        let read = async move
        {
            let handles    = dialog.pick_files().await.unwrap_or_default();
            let mut picked = Vec::new();

            for handle in handles
            {
                let data = handle.read().await;
                picked.push( Ok( Picked { name: handle.file_name(), data: data.into() } ) );
            }

            picked
        };

        self.spawn( ctx, read );
    }

    /// Reads the files dropped on the page; they come out of [`Picker::take`] too.
    pub fn read_dropped( &self, ctx: &egui::Context, dropped: Vec<egui::DroppedFileHandle> )
    {
        let read = async move
        {
            let mut picked = Vec::new();

            for file in dropped
            {
                let name = file.path().to_string_lossy().into_owned();
                let data = file.bytes_async().await.map_err( |err| format!( "{name}: {err}" ) );
                picked.push( data.map( |data| Picked { name, data: data.into() } ) );
            }

            picked
        };

        self.spawn( ctx, read );
    }

    pub fn take( &self ) -> Vec<Result<Picked, String>>
    {
        self.receiver.try_iter().flatten().collect()
    }

    fn spawn( &self, ctx: &egui::Context, read: impl Future<Output = Vec<Result<Picked, String>>> + 'static )
    {
        let sender = self.sender.clone();
        let ctx    = ctx.clone();

        let task = async move
        {
            let picked = read.await;
            if !picked.is_empty() && sender.send( picked ).is_ok()
            {
                ctx.request_repaint();
            }
        };

        wasm_bindgen_futures::spawn_local( task );
    }
}

/// Hands `data` to the browser's downloads as `name`.
pub fn download( name: &str, data: &[u8] ) -> Result<(), String>
{
    let parts    = js_sys::Array::of1( &js_sys::Uint8Array::from( data ) );
    let blob     = web_sys::Blob::new_with_u8_array_sequence( &parts ).map_err( |err| error_text( &err ) )?;
    let url      = web_sys::Url::create_object_url_with_blob( &blob ).map_err( |err| error_text( &err ) )?;
    let document = web_sys::window().and_then( |window| window.document() ).ok_or( "no document" )?;
    let element  = document.create_element( "a" ).map_err( |err| error_text( &err ) )?;
    let anchor   = element.dyn_into::<web_sys::HtmlAnchorElement>().map_err( |_| "no link element" )?;

    anchor.set_href( &url );
    anchor.set_download( name );
    anchor.click();

    web_sys::Url::revoke_object_url( &url ).map_err( |err| error_text( &err ) )
}

/// A JavaScript error's message, or its debug form when it has none.
pub fn error_text( err: &JsValue ) -> String
{
    err.as_string().unwrap_or_else( || format!( "{err:?}" ) )
}
