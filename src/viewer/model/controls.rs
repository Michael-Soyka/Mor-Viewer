use eframe::egui;

use rust_i18n::t;
use crate::render::weather::Weather;
use crate::ui::theme::Palette;
use crate::ui::widgets::{ self, from_f32 };
use crate::viewer::model::frame::{ Fog, TimeOfDay };

/// Fog sliders reach this far, past the city's size; the end keeps this gap after the start.
const FOG_MAX:      f32 = 60_000.0;
const FOG_MIN_SPAN: f32 = 100.0;

/// Fog start, end and color; the end stays past the start.
pub fn fog_settings( ui: &mut egui::Ui, palette: &Palette, fog: &mut Fog ) -> bool
{
    let before = *fog;

    egui::Grid::new( "fog" ).show(
        ui,
        |ui|
        {
            distance( ui, palette, &t!( "model.fog_start" ), &mut fog.start );
            distance( ui, palette, &t!( "model.fog_end" ), &mut fog.end );
            ui.label( t!( "model.fog_color" ) );
            egui::color_picker::color_edit_button_srgba( ui, &mut fog.color, egui::color_picker::Alpha::Opaque );
            ui.end_row();
        }
    );
    fog.end = fog.end.max( fog.start + FOG_MIN_SPAN );

    *fog != before
}

/// Grid row: label, slider, value in world units.
fn distance( ui: &mut egui::Ui, palette: &Palette, label: &str, value: &mut f32 )
{
    ui.label( label );
    if let Some( distance ) = widgets::slider( ui, palette, *value, ( 0.0, FOG_MAX ) )
    {
        *value = ( distance / 100.0 ).round() * 100.0;
    }
    ui.label( format!( "{value:.0}" ) );
    ui.end_row();
}

/// Grid row: label, slider, value in whole degrees.
pub fn angle( ui: &mut egui::Ui, palette: &Palette, label: &str, value: &mut f32, range: (f32, f32) ) -> bool
{
    ui.label( label );
    let result = widgets::slider( ui, palette, *value, range );
    if let Some( angle ) = result
    {
        *value = angle.round();
    }
    ui.label( format!( "{value:.0}°" ) );
    ui.end_row();

    result.is_some()
}

/// Hour slider and weather choice; returns whether either changed.
pub fn time_settings( ui: &mut egui::Ui, palette: &Palette, time: &mut TimeOfDay ) -> bool
{
    let before = *time;

    egui::Grid::new( "time" ).show(
        ui,
        |ui|
        {
            ui.label( t!( "model.hour" ) );
            if let Some( hour ) = widgets::slider( ui, palette, time.hour, ( 0.0, 24.0 ) )
            {
                time.hour = ( hour * 4.0 ).round() / 4.0;
            }
            let minutes = from_f32( ( time.hour * 60.0 ).round() );
            ui.label( format!( "{:02}:{:02}", ( minutes / 60 ) % 24, minutes % 60 ) );
            ui.end_row();
        }
    );
    ui.horizontal(
        |ui|
        {
            for weather in Weather::ALL
            {
                ui.selectable_value( &mut time.weather, weather, t!( weather.key() ) );
            }
        }
    );
    ui.checkbox( &mut time.street_lights, t!( "model.street_lights" ) );

    *time != before
}
