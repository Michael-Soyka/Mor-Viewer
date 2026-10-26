use std::collections::HashMap;

use eframe::egui_wgpu::{ RenderState, wgpu };

use crate::render::model::Model;
use crate::render::target::COLOR_FORMAT;

/// Path and data of a texture file by the name a material refers to.
pub type FindTexture<'a> = &'a dyn Fn( &str ) -> Option<(String, Vec<u8>)>;

/// Decoded textures by lowercase name.
pub type Textures = HashMap<String, DecodedTexture>;

pub struct DecodedTexture
{
    pub width:  u32,
    pub height: u32,
    pub mips:   Vec<Vec<u8>>,
}

/// Every texture the model's parts name, and the names not found; `limit` skips mips wider than it.
pub fn decode_textures( model: &Model, find_texture: FindTexture, limit: Option<u32> ) -> (Textures, Vec<String>)
{
    let mut textures = Textures::new();
    let mut missing  = Vec::new();

    for name in model.parts.iter().filter_map( |part| part.texture.as_deref() )
    {
        let key = name.to_lowercase();
        if textures.contains_key( &key ) || missing.contains( &key )
        {
            continue;
        }
        match decode_texture( name, find_texture, limit )
        {
            Some( texture ) =>
            {
                textures.insert( key, texture );
            }
            None => missing.push( key ),
        }
    }

    ( textures, missing )
}

fn decode_texture( name: &str, find_texture: FindTexture, limit: Option<u32> ) -> Option<DecodedTexture>
{
    let ( path, data ) = find_texture( name )?;
    if crate::data::preview::image_format( &path ).is_some()
    {
        let image   = crate::data::preview::decode_image( &path, &data ).ok()?;
        let decoded = DecodedTexture { width: image.width, height: image.height, mips: vec![ image.rgba ] };
        return Some( decoded );
    }

    let texture = mortex::Texture::parse( &data ).ok()?;
    let last    = texture.mip_count().saturating_sub( 1 );
    let first   = limit.map_or( 0, |limit| ( 0..last ).find( |&mip| ( texture.width() >> mip ) <= limit ).unwrap_or( last ) );
    let mips: Vec<Vec<u8>> = ( first..texture.mip_count() ).map_while( |mip| texture.decode( 0, mip ).ok().map( |image| image.rgba ) ).collect();
    if mips.is_empty()
    {
        return None;
    }
    let ( width, height ) = ( ( texture.width() >> first ).max( 1 ), ( texture.height() >> first ).max( 1 ) );

    Some( DecodedTexture { width, height, mips } )
}

pub fn upload_texture( state: &RenderState, decoded: &DecodedTexture ) -> wgpu::TextureView
{
    upload( state, decoded.width, decoded.height, &decoded.mips, COLOR_FORMAT )
}

/// `mips` from the full size down, tightly packed rows of `format` texels.
pub fn upload( state: &RenderState, width: u32, height: u32, mips: &[Vec<u8>], format: wgpu::TextureFormat ) -> wgpu::TextureView
{
    let texel = format.block_copy_size( None ).unwrap_or( 4 );

    let mip_count = u32::try_from( mips.len() ).unwrap_or( 1 );
    let descriptor = wgpu::TextureDescriptor
    {
        label:           Some( "model albedo" ),
        size:            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: mip_count,
        sample_count:    1,
        dimension:       wgpu::TextureDimension::D2,
        format,
        usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats:    &[],
    };
    let texture = state.device.create_texture( &descriptor );

    for ( mip, rgba ) in ( 0..mip_count ).zip( mips )
    {
        let ( mip_width, mip_height ) = ( ( width >> mip ).max( 1 ), ( height >> mip ).max( 1 ) );
        let destination = wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: mip, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All };
        let layout      = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some( mip_width * texel ), rows_per_image: Some( mip_height ) };
        let extent      = wgpu::Extent3d { width: mip_width, height: mip_height, depth_or_array_layers: 1 };
        state.queue.write_texture( destination, rgba, layout, extent );
    }

    texture.create_view( &wgpu::TextureViewDescriptor::default() )
}
