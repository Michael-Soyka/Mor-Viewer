use std::fs;
use std::process::Command;

/// Build counter, kept in the repository so it survives `cargo clean`.
const COUNTER: &str = "build-number";

/// Inputs that make a new build; `.git` ones pick up a new commit hash.
const WATCHED: [&str; 6] = [ "src", "assets", "locales", "Cargo.toml", ".git/HEAD", ".git/refs" ];

fn main()
{
    for path in WATCHED
    {
        println!( "cargo::rerun-if-changed={path}" );
    }

    let previous = fs::read_to_string( COUNTER ).ok().and_then( |text| text.trim().parse::<u64>().ok() ).unwrap_or( 0 );
    let build    = previous + 1;
    fs::write( COUNTER, format!( "{build}\n" ) ).expect( "build counter is writable" );

    let commit = Command::new( "git" )
        .args( [ "rev-parse", "--short", "HEAD" ] )
        .output()
        .ok()
        .filter( |output| output.status.success() )
        .map( |output| String::from_utf8_lossy( &output.stdout ).trim().to_owned() )
        .unwrap_or_else( || "unknown".to_owned() );

    println!( "cargo::rustc-env=MOR_BUILD={build}" );
    println!( "cargo::rustc-env=MOR_COMMIT={commit}" );

    #[ cfg( windows ) ]
    embed_icon();
}

/// The executable's icon for Explorer and shortcuts; the window sets its own at runtime.
#[ cfg( windows ) ]
fn embed_icon()
{
    if std::env::var( "CARGO_CFG_TARGET_OS" ).is_ok_and( |os| os != "windows" )
    {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon( "assets/icons/app.ico" );
    resource.compile().expect( "icon resource compiles" );
}
