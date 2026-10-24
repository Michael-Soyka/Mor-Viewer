use std::num::NonZero;
use std::time::Duration;

use eframe::egui::{ self, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, Vec2 };
use rodio::buffer::SamplesBuffer;
use rodio::{ DeviceSinkBuilder, MixerDeviceSink, Player };

use rust_i18n::t;
use crate::i18n;
use crate::preview::Audio;
use crate::theme::{ self, Palette };
use crate::viewer::Sections;
use crate::widgets::{ from_f32, to_f32 };

const PEAK_BUCKETS: usize = 2048;
const WAVE_PADDING: f32   = 16.0;
const BUTTON_SIZE:  f32   = 22.0;

pub struct AudioViewer
{
    audio:    Audio,
    duration: Duration,

    /// Min and max of the mono mix per bucket.
    peaks: Vec<(f32, f32)>,

    sink:   Option<MixerDeviceSink>,
    player: Option<Player>,

    /// Position to start from while nothing is playing.
    position: Duration,
}

impl AudioViewer
{
    pub fn new( audio: Audio ) -> Self
    {
        let frames   = audio.samples.len() / usize::from( audio.channels.max( 1 ) );
        let duration = Duration::from_secs_f64( to_f64( frames ) / f64::from( audio.sample_rate.max( 1 ) ) );
        let peaks    = peaks( &audio );

        Self { audio, duration, peaks, sink: None, player: None, position: Duration::ZERO }
    }

    pub fn show( &mut self, sections: &mut Sections, palette: &Palette ) -> Option<String>
    {
        let mut error = None;
        let playing   = self.player.as_ref().is_some_and( |player| !player.is_paused() && !player.empty() );
        let position  = self.current_position();

        if play_button( &mut sections.action_left, palette, playing ).clicked()
        {
            let toggled = if playing { Ok( self.on_pause() ) } else { self.on_play() };
            error = toggled.err();
        }
        let time = format!( "{} / {}", format_time( position ), format_time( self.duration ) );
        sections.action_left.label( egui::RichText::new( time ).monospace().color( palette.text ) );

        if let Some( target ) = self.waveform( &mut sections.view, palette, position )
        {
            self.on_seek( target );
        }
        if playing
        {
            sections.view.ctx().request_repaint();
        }

        let channels = match self.audio.channels
        {
            1 => t!( "audio.mono" ).into_owned(),
            2 => t!( "audio.stereo" ).into_owned(),
            count => i18n::count( "audio.channels", usize::from( count ) ),
        };
        sections.footer_text( palette, &t!( "audio.format", rate = self.audio.sample_rate, channels = channels ) );
        if let Some( ( _, title ) ) = self.audio.comments.iter().find( |( key, _ )| key.eq_ignore_ascii_case( "title" ) )
        {
            sections.footer_right.label( egui::RichText::new( title ).size( theme::STATUS_TEXT_SIZE ).color( palette.menu_text ) );
        }

        error
    }

    fn current_position( &self ) -> Duration
    {
        match &self.player
        {
            Some( player ) if !player.empty() => player.get_pos(),
            _ => self.position,
        }
    }

    fn on_play( &mut self ) -> Result<(), String>
    {
        if let Some( player ) = &self.player
            && !player.empty()
        {
            player.play();
            return Ok( () );
        }
        if self.position >= self.duration
        {
            self.position = Duration::ZERO;
        }

        let sink = match &mut self.sink
        {
            Some( sink ) => sink,
            None =>
            {
                let mut sink = DeviceSinkBuilder::open_default_sink().map_err( |err| err.to_string() )?;
                sink.log_on_drop( false );
                self.sink.insert( sink )
            }
        };
        let channels    = NonZero::new( self.audio.channels ).ok_or( t!( "audio.no_channels" ) )?;
        let sample_rate = NonZero::new( self.audio.sample_rate ).ok_or( t!( "audio.no_rate" ) )?;
        let player      = Player::connect_new( sink.mixer() );
        player.append( SamplesBuffer::new( channels, sample_rate, self.audio.samples.clone() ) );
        if !self.position.is_zero()
        {
            player.try_seek( self.position ).map_err( |err| err.to_string() )?;
        }
        self.player = Some( player );

        Ok( () )
    }

    fn on_pause( &mut self )
    {
        if let Some( player ) = &self.player
        {
            player.pause();
            self.position = player.get_pos();
        }
    }

    fn on_seek( &mut self, target: Duration )
    {
        self.position = target.min( self.duration );
        if let Some( player ) = &self.player
            && !player.empty()
        {
            // Seeking within an in-memory buffer cannot fail.
            player.try_seek( self.position ).ok();
        }
    }

    /// Paints the waveform with a playhead; returns the position clicked or dragged to.
    fn waveform( &self, ui: &mut egui::Ui, palette: &Palette, position: Duration ) -> Option<Duration>
    {
        let ( area, response ) = ui.allocate_exact_size( ui.available_size(), Sense::click_and_drag() );
        let rect = area.shrink( WAVE_PADDING );
        if rect.width() <= 0.0 || rect.height() <= 0.0
        {
            return None;
        }

        let painter = ui.painter();
        let center  = rect.center().y;
        let half    = rect.height() / 2.0;

        let progress = if self.duration.is_zero() { 0.0 } else { ( position.as_secs_f32() / self.duration.as_secs_f32() ).clamp( 0.0, 1.0 ) };
        let head = rect.min.x + ( progress * rect.width() );

        let columns = from_f32( rect.width().floor() ).max( 1 );
        for column in 0..columns
        {
            let x      = rect.min.x + to_f32( column ) + 0.5;
            let bucket = ( column * self.peaks.len() ) / columns;
            let ( low, high ) = self.peaks.get( bucket ).copied().unwrap_or_default();
            let color = if x <= head { palette.text } else { palette.menu_text };
            let top    = center - ( high * half );
            let bottom = ( center - ( low * half ) ).max( top + 1.0 );
            painter.line_segment( [ Pos2::new( x, top ), Pos2::new( x, bottom ) ], Stroke::new( 1.0, color ) );
        }
        painter.line_segment( [ Pos2::new( head, rect.min.y ), Pos2::new( head, rect.max.y ) ], Stroke::new( 1.0, palette.channels[ 0 ] ) );

        let pointer  = response.interact_pointer_pos()?;
        let fraction = ( ( pointer.x - rect.min.x ) / rect.width() ).clamp( 0.0, 1.0 );
        let target   = self.duration.mul_f32( fraction );

        Some( target )
    }
}

impl Drop for AudioViewer
{
    fn drop( &mut self )
    {
        if let Some( player ) = &self.player
        {
            player.stop();
        }
    }
}

fn play_button( ui: &mut egui::Ui, palette: &Palette, playing: bool ) -> egui::Response
{
    let ( rect, response ) = ui.allocate_exact_size( Vec2::splat( BUTTON_SIZE ), Sense::click() );
    let painter = ui.painter();
    if response.hovered()
    {
        painter.rect_filled( rect, CornerRadius::same( theme::CONTROL_RADIUS ), palette.hovered );
    }

    let icon = Rect::from_center_size( rect.center(), Vec2::splat( 10.0 ) );

    if playing
    {
        for left in [ icon.min.x + 1.0, icon.max.x - 4.0 ]
        {
            let bar = Rect::from_min_size( Pos2::new( left, icon.min.y ), Vec2::new( 3.0, icon.height() ) );
            painter.rect_filled( bar, CornerRadius::ZERO, palette.text );
        }
    }
    else
    {
        let points = vec![ icon.left_top() + Vec2::new( 1.0, 0.0 ), Pos2::new( icon.max.x, icon.center().y ), icon.left_bottom() + Vec2::new( 1.0, 0.0 ) ];
        painter.add( Shape::convex_polygon( points, palette.text, Stroke::NONE ) );
    }

    response
}

fn peaks( audio: &Audio ) -> Vec<(f32, f32)>
{
    let channels = usize::from( audio.channels.max( 1 ) );
    let frames   = audio.samples.len() / channels;
    if frames == 0
    {
        return Vec::new();
    }

    let buckets   = PEAK_BUCKETS.min( frames );
    let mut peaks = vec![ ( 0.0f32, 0.0f32 ); buckets ];

    for ( frame, samples ) in audio.samples.chunks_exact( channels ).enumerate()
    {
        let mono = samples.iter().sum::<f32>() / to_f32( channels );
        let peak = &mut peaks[ ( frame * buckets ) / frames ];
        peak.0 = peak.0.min( mono );
        peak.1 = peak.1.max( mono );
    }

    peaks
}

fn format_time( time: Duration ) -> String
{
    let seconds = time.as_secs();
    let tenths  = time.subsec_millis() / 100;

    format!( "{}:{:02}.{tenths}", seconds / 60, seconds % 60 )
}

/// Frame counts fit an f64 mantissa for any realistic clip.
fn to_f64( value: usize ) -> f64
{
    value as f64
}
