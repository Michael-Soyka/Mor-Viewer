use rust_i18n::t;
use serde::{ Deserialize, Serialize };

#[ derive( Clone, Copy, PartialEq, Eq, Serialize, Deserialize ) ]
pub enum Language
{
    English,
    Russian,
}

impl Language
{
    pub const ALL: [Self; 2] = [ Self::English, Self::Russian ];

    /// The system language if there is a translation for it, English otherwise.
    pub fn system() -> Self
    {
        let locale = sys_locale::get_locale().unwrap_or_default();

        if locale.starts_with( "ru" ) { Self::Russian } else { Self::English }
    }

    /// Own name, so it reads right whichever language is active.
    pub fn label( self ) -> &'static str
    {
        match self
        {
            Self::English => "English",
            Self::Russian => "Русский",
        }
    }

    fn code( self ) -> &'static str
    {
        match self
        {
            Self::English => "en",
            Self::Russian => "ru",
        }
    }
}

pub fn apply( language: Language )
{
    rust_i18n::set_locale( language.code() );
}

/// `key.one`, `key.few` or `key.many` by the active language's plural rules.
pub fn count( key: &str, value: usize ) -> String
{
    let russian = rust_i18n::locale().starts_with( "ru" );
    let form    = if russian { russian_form( value ) } else if value == 1 { "one" } else { "many" };

    t!( format!( "{key}.{form}" ), count = value ).into_owned()
}

fn russian_form( value: usize ) -> &'static str
{
    let ( ones, tens ) = ( value % 10, value % 100 );

    if ones == 1 && tens != 11
    {
        "one"
    }
    else if ( 2..=4 ).contains( &ones ) && !( 12..=14 ).contains( &tens )
    {
        "few"
    }
    else
    {
        "many"
    }
}
