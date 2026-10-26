use eframe::egui::{ ImageSource, include_image };

pub const APP: ImageSource<'static> = include_image!( "../assets/icons/app.svg" );

pub const CHEVRON: ImageSource<'static> = include_image!( "../assets/icons/tree/chevron.svg" );
pub const ARCHIVE: ImageSource<'static> = include_image!( "../assets/icons/tree/archive.svg" );
pub const FOLDER:  ImageSource<'static> = include_image!( "../assets/icons/tree/folder.svg" );
pub const SEARCH:  ImageSource<'static> = include_image!( "../assets/icons/tree/search.svg" );

pub const TEXTURE:  ImageSource<'static> = include_image!( "../assets/icons/files/texture.svg" );
pub const FONT:     ImageSource<'static> = include_image!( "../assets/icons/files/font.svg" );
pub const MESH:     ImageSource<'static> = include_image!( "../assets/icons/files/mesh.svg" );
pub const BINARY:   ImageSource<'static> = include_image!( "../assets/icons/files/binary.svg" );
pub const TEXT:     ImageSource<'static> = include_image!( "../assets/icons/files/text.svg" );
pub const WORLD:    ImageSource<'static> = include_image!( "../assets/icons/files/world.svg" );
pub const INTERIOR: ImageSource<'static> = include_image!( "../assets/icons/files/interior.svg" );
pub const PATH:     ImageSource<'static> = include_image!( "../assets/icons/files/path.svg" );
pub const AUDIO:    ImageSource<'static> = include_image!( "../assets/icons/files/audio.svg" );

pub const WIREFRAME: ImageSource<'static> = include_image!( "../assets/icons/render/wireframe.svg" );
pub const SOLID:     ImageSource<'static> = include_image!( "../assets/icons/render/solid.svg" );
pub const MATERIAL:  ImageSource<'static> = include_image!( "../assets/icons/render/material.svg" );
pub const SHADED:    ImageSource<'static> = include_image!( "../assets/icons/render/shaded.svg" );
pub const LIGHT:     ImageSource<'static> = include_image!( "../assets/icons/render/light.svg" );
pub const RENDER:    ImageSource<'static> = include_image!( "../assets/icons/render/render.svg" );

pub const PLAY:  ImageSource<'static> = include_image!( "../assets/icons/actions/play.svg" );
pub const PAUSE: ImageSource<'static> = include_image!( "../assets/icons/actions/pause.svg" );
