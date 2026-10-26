/// Reversed depth: 1 at `near`, falling to 0 at `far`, or far away without one, so precision holds across a whole city.
pub fn perspective( fov_y: f32, aspect: f32, near: f32, far: Option<f32> ) -> [[f32; 4]; 4]
{
    let focal = 1.0 / ( fov_y / 2.0 ).tan();

    // Depth near * ( far - d ) / ( d * ( far - near ) ) for the distance d along the view.
    let ( scale, offset ) = match far
    {
        Some( far ) =>
        {
            let far = far.max( near * 2.0 );
            ( near / ( far - near ), ( near * far ) / ( far - near ) )
        }
        None => ( 0.0, near ),
    };

    [ [ focal / aspect, 0.0, 0.0, 0.0 ], [ 0.0, focal, 0.0, 0.0 ], [ 0.0, 0.0, scale, -1.0 ], [ 0.0, 0.0, offset, 0.0 ] ]
}

pub fn look_at( eye: [f32; 3], target: [f32; 3], up: [f32; 3] ) -> [[f32; 4]; 4]
{
    let forward = normalize( sub( target, eye ) );
    let side    = normalize( cross( forward, up ) );
    let up      = cross( side, forward );

    let matrix =
    [
        [ side[ 0 ], up[ 0 ], -forward[ 0 ], 0.0 ],
        [ side[ 1 ], up[ 1 ], -forward[ 1 ], 0.0 ],
        [ side[ 2 ], up[ 2 ], -forward[ 2 ], 0.0 ],
        [ -dot( side, eye ), -dot( up, eye ), dot( forward, eye ), 1.0 ],
    ];

    matrix
}

/// Column-major product `a * b`.
pub fn mul( a: [[f32; 4]; 4], b: [[f32; 4]; 4] ) -> [[f32; 4]; 4]
{
    b.map( |column| [ 0, 1, 2, 3 ].map( |row| ( 0..4 ).map( |k| a[ k ][ row ] * column[ k ] ).sum() ) )
}

pub fn add( a: [f32; 3], b: [f32; 3] ) -> [f32; 3]
{
    [ a[ 0 ] + b[ 0 ], a[ 1 ] + b[ 1 ], a[ 2 ] + b[ 2 ] ]
}

pub fn sub( a: [f32; 3], b: [f32; 3] ) -> [f32; 3]
{
    [ a[ 0 ] - b[ 0 ], a[ 1 ] - b[ 1 ], a[ 2 ] - b[ 2 ] ]
}

pub fn dot( a: [f32; 3], b: [f32; 3] ) -> f32
{
    ( a[ 0 ] * b[ 0 ] ) + ( a[ 1 ] * b[ 1 ] ) + ( a[ 2 ] * b[ 2 ] )
}

pub fn cross( a: [f32; 3], b: [f32; 3] ) -> [f32; 3]
{
    [ ( a[ 1 ] * b[ 2 ] ) - ( a[ 2 ] * b[ 1 ] ), ( a[ 2 ] * b[ 0 ] ) - ( a[ 0 ] * b[ 2 ] ), ( a[ 0 ] * b[ 1 ] ) - ( a[ 1 ] * b[ 0 ] ) ]
}

pub fn normalize( v: [f32; 3] ) -> [f32; 3]
{
    let length = dot( v, v ).sqrt();

    if length > 0.0 { v.map( |value| value / length ) } else { v }
}
