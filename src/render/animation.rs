//! Character animation. A key is not a bone's pose but its offset from the bind pose in model space:
//! the bone's skinning transform is its own offset followed by its parent's skinning transform.

use eframe::egui;
use mormesh::layout::character::{ CharacterMesh, Keyframe };
use rust_i18n::t;

use crate::render::model::{ Vertex, to_usize };
use crate::ui::theme::{ self, Palette };
use crate::ui::widgets::{ self, from_f32, to_f32 };

/// Keys lie on a grid of 1/60 s.
const FPS: f32 = 60.0;

/// Longest step of one frame, so a stall does not skip the animation ahead.
const MAX_STEP: f32 = 0.1;

/// Row-vector transform: `point * rows + translation`.
#[ derive( Clone, Copy ) ]
struct Transform
{
    rows:        [[f32; 3]; 3],
    translation: [f32; 3],
}

impl Transform
{
    const IDENTITY: Self = Self { rows: [ [ 1.0, 0.0, 0.0 ], [ 0.0, 1.0, 0.0 ], [ 0.0, 0.0, 1.0 ] ], translation: [ 0.0; 3 ] };

    /// The quaternion's matrix taken as rows, which turns the other way: the game stores the inverse rotation.
    fn from_key( key: &Keyframe ) -> Self
    {
        let [ x, y, z, w ] = key.rotation;
        let rows =
        [
            [ 1.0 - ( 2.0 * ( ( y * y ) + ( z * z ) ) ), 2.0 * ( ( x * y ) - ( z * w ) ), 2.0 * ( ( x * z ) + ( y * w ) ) ],
            [ 2.0 * ( ( x * y ) + ( z * w ) ), 1.0 - ( 2.0 * ( ( x * x ) + ( z * z ) ) ), 2.0 * ( ( y * z ) - ( x * w ) ) ],
            [ 2.0 * ( ( x * z ) - ( y * w ) ), 2.0 * ( ( y * z ) + ( x * w ) ), 1.0 - ( 2.0 * ( ( x * x ) + ( y * y ) ) ) ],
        ];

        Self { rows, translation: key.position }
    }

    fn rotate( &self, vector: [f32; 3] ) -> [f32; 3]
    {
        let [ x, y, z ] = vector;
        let [ a, b, c ] = self.rows;

        [ 0, 1, 2 ].map( |column| ( x * a[ column ] ) + ( y * b[ column ] ) + ( z * c[ column ] ) )
    }

    fn apply( &self, point: [f32; 3] ) -> [f32; 3]
    {
        let rotated = self.rotate( point );

        [ 0, 1, 2 ].map( |axis| rotated[ axis ] + self.translation[ axis ] )
    }

    /// `self`, then `next`.
    fn then( &self, next: &Self ) -> Self
    {
        Self { rows: self.rows.map( |row| next.rotate( row ) ), translation: next.apply( self.translation ) }
    }
}

#[ derive( Clone ) ]
pub struct Clip
{
    /// The animation's name, after its bone group's unless that is `all`.
    label: String,

    /// Bone of each track.
    bones:  Vec<usize>,
    tracks: Vec<Vec<Keyframe>>,

    /// Time of the last key of the first track, as the engine takes it.
    duration: f32,
}

impl Clip
{
    fn frames( &self ) -> usize
    {
        from_f32( ( self.duration * FPS ).round() )
    }
}

/// Bones, vertex weights and animations of a character.
#[ derive( Clone ) ]
pub struct Skeleton
{
    /// Bones with their parents, each after its parent.
    order: Vec<(usize, Option<usize>)>,

    /// Bone and weight of each vertex's influences.
    weights: Vec<Vec<(usize, f32)>>,

    clips: Vec<Clip>,
}

impl Skeleton
{
    /// `None` without animations.
    pub fn new( mesh: &CharacterMesh ) -> Option<Self>
    {
        let mut clips = Vec::new();

        for group in &mesh.groups
        {
            let bones: Vec<usize> = group.bones.iter().map( |&bone| to_usize( bone ) ).collect();
            for animation in &group.animations
            {
                let label = if group.name == "all" { animation.name.clone() } else { format!( "{} / {}", group.name, animation.name ) };
                let clip  = Clip { label, bones: bones.clone(), tracks: animation.tracks.clone(), duration: animation.duration() };
                clips.push( clip );
            }
        }
        if clips.is_empty()
        {
            return None;
        }

        let parents: Vec<Option<usize>> = mesh.bones.iter().map( |bone| bone.parent.map( to_usize ) ).collect();
        let mut order = Vec::with_capacity( parents.len() );
        let mut stack: Vec<usize> = ( 0..parents.len() ).filter( |&bone| parents.get( bone ) == Some( &None ) ).collect();

        while let Some( bone ) = stack.pop()
        {
            let parent = parents.get( bone ).copied().flatten();
            order.push( ( bone, parent ) );
            stack.extend( ( 0..parents.len() ).filter( |&child| parents.get( child ) == Some( &Some( bone ) ) ) );
        }

        let weights = mesh.weights.iter().map( |influences| influences.iter().map( |influence| ( to_usize( influence.bone ), influence.weight ) ).collect() ).collect();
        let skeleton = Self { order, weights, clips };

        Some( skeleton )
    }

    /// `bind` posed by `clip` at `time` into `posed`: positions and normals.
    fn pose( &self, clip: &Clip, time: f32, bind: &[Vertex], posed: &mut [Vertex] )
    {
        let bone_count = self.order.len();
        let mut offsets = vec![ Transform::IDENTITY; bone_count ];

        for ( &bone, track ) in clip.bones.iter().zip( &clip.tracks )
        {
            if let Some( key ) = sample( track, time )
                && let Some( offset ) = offsets.get_mut( bone )
            {
                *offset = Transform::from_key( &key );
            }
        }

        let mut skins = vec![ Transform::IDENTITY; bone_count ];

        for &( bone, parent ) in &self.order
        {
            let offset = offsets.get( bone ).copied().unwrap_or( Transform::IDENTITY );
            let parent = parent.and_then( |parent| skins.get( parent ) ).copied().unwrap_or( Transform::IDENTITY );
            if let Some( skin ) = skins.get_mut( bone )
            {
                *skin = offset.then( &parent );
            }
        }

        for ( ( vertex, source ), influences ) in posed.iter_mut().zip( bind ).zip( &self.weights )
        {
            let position = [ source[ 0 ], source[ 1 ], source[ 2 ] ];
            let normal   = [ source[ 3 ], source[ 4 ], source[ 5 ] ];
            let mut moved  = [ 0.0; 3 ];
            let mut turned = [ 0.0; 3 ];

            for &( bone, weight ) in influences
            {
                let Some( skin ) = skins.get( bone ) else { continue };
                let ( point, direction ) = ( skin.apply( position ), skin.rotate( normal ) );
                for axis in 0..3
                {
                    moved[ axis ] += weight * point[ axis ];
                    turned[ axis ] += weight * direction[ axis ];
                }
            }

            let length = ( ( turned[ 0 ] * turned[ 0 ] ) + ( turned[ 1 ] * turned[ 1 ] ) + ( turned[ 2 ] * turned[ 2 ] ) ).sqrt().max( f32::EPSILON );
            vertex[ ..3 ].copy_from_slice( &moved );
            vertex[ 3..6 ].copy_from_slice( &turned.map( |value| value / length ) );
        }
    }
}

/// The track at `time`: the keys around it blended, the ends held.
fn sample( track: &[Keyframe], time: f32 ) -> Option<Keyframe>
{
    let next = track.partition_point( |key| key.time <= time );
    let Some( from ) = next.checked_sub( 1 ).and_then( |index| track.get( index ) ) else { return track.first().copied() };
    let Some( to ) = track.get( next ) else { return Some( *from ) };

    let span   = to.time - from.time;
    let amount = if span > 0.0 { ( time - from.time ) / span } else { 0.0 };

    // The shorter way round; keys are close enough for a normalized blend.
    let dot  = ( 0..4 ).map( |index| from.rotation[ index ] * to.rotation[ index ] ).sum::<f32>();
    let sign = if dot < 0.0 { -1.0 } else { 1.0 };
    let mixed: [f32; 4] = std::array::from_fn( |index| ( from.rotation[ index ] * ( 1.0 - amount ) ) + ( to.rotation[ index ] * sign * amount ) );
    let length   = mixed.iter().map( |value| value * value ).sum::<f32>().sqrt().max( f32::EPSILON );
    let rotation = mixed.map( |value| value / length );
    let position = std::array::from_fn( |index| ( from.position[ index ] * ( 1.0 - amount ) ) + ( to.position[ index ] * amount ) );

    Some( Keyframe { time, rotation, position } )
}

/// Playback of a character's animations over its bind pose.
pub struct Player
{
    skeleton: Skeleton,
    bind:     Vec<Vertex>,
    posed:    Vec<Vertex>,
    clip:     usize,
    time:     f32,
    playing:  bool,

    /// The pose lags behind the clip or the time.
    stale: bool,
}

impl Player
{
    pub fn new( skeleton: Skeleton, bind: Vec<Vertex> ) -> Self
    {
        Self { skeleton, posed: bind.clone(), bind, clip: 0, time: 0.0, playing: false, stale: true }
    }

    pub fn playing( &self ) -> bool
    {
        self.playing
    }

    /// Moves the time on by `seconds`, round the clip.
    pub fn advance( &mut self, seconds: f32 )
    {
        let Some( clip ) = self.skeleton.clips.get( self.clip ) else { return };
        if clip.duration <= 0.0
        {
            return;
        }
        self.time  = ( self.time + seconds.min( MAX_STEP ) ) % clip.duration;
        self.stale = true;
    }

    /// The new pose, if the clip or the time moved since the last one.
    pub fn pose( &mut self ) -> Option<&[Vertex]>
    {
        if !std::mem::take( &mut self.stale )
        {
            return None;
        }
        let clip = self.skeleton.clips.get( self.clip )?;
        self.skeleton.pose( clip, self.time, &self.bind, &mut self.posed );

        Some( &self.posed )
    }

    /// Clip picker, play button, time slider and frame counter.
    pub fn show( &mut self, ui: &mut egui::Ui, palette: &Palette )
    {
        let Some( clip ) = self.skeleton.clips.get( self.clip ) else { return };
        let frames = clip.frames();
        let frame  = from_f32( ( self.time * FPS ).round() ).min( frames );

        let response = widgets::dropdown_button( ui, palette, &clip.label );
        egui::Popup::menu( &response ).show(
            |ui|
            {
                for ( index, clip ) in self.skeleton.clips.iter().enumerate()
                {
                    if ui.selectable_value( &mut self.clip, index, &clip.label ).changed()
                    {
                        self.time  = 0.0;
                        self.stale = true;
                    }
                }
            }
        );
        widgets::status_separator( ui, palette );

        if widgets::play_button( ui, palette, self.playing ).clicked()
        {
            self.playing = !self.playing;
        }
        if let Some( position ) = widgets::slider( ui, palette, to_f32( frame ), ( 0.0, to_f32( frames ) ) )
        {
            self.time    = position.round() / FPS;
            self.playing = false;
            self.stale   = true;
        }
        widgets::status_separator( ui, palette );

        let counter = t!( "model.frame", current = frame, total = frames );
        ui.label( egui::RichText::new( counter ).size( theme::STATUS_TEXT_SIZE ).color( palette.text ) );
    }
}
