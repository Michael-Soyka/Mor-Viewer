use std::path::Path;

use mormesh::gltf::Gltf;

/// `.gltf` gets its buffer beside it as `.bin`; any other name is written as `.glb`.
pub fn save( gltf: &Gltf, target: &Path ) -> Result<(), String>
{
    let text = target.extension().is_some_and( |ext| ext.eq_ignore_ascii_case( "gltf" ) );
    if !text
    {
        let glb = gltf.to_glb().map_err( |err| err.to_string() )?;

        return write( target, &glb );
    }

    let bin_path = target.with_extension( "bin" );
    let bin_uri  = bin_path.file_name().map( |name| name.to_string_lossy().into_owned() ).unwrap_or_default();
    write( target, gltf.to_gltf( &bin_uri ).as_bytes() )?;

    write( &bin_path, gltf.bin() )
}

fn write( path: &Path, bytes: &[u8] ) -> Result<(), String>
{
    std::fs::write( path, bytes ).map_err( |err| format!( "{}: {err}", path.display() ) )
}
