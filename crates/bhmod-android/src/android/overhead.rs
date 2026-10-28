//! The game's world-to-screen transform for overhead icons ([`crate::overhead`]): `World.tapProjectionMatrix` and
//! `World.tapModelviewMatrix` (`GLKMatrix4`, 16 floats, column-major) and `World.tapViewport` (`GLint[4]`), the values
//! the game unprojects taps with, so they match what was drawn in the last frame.

use super::objc::{self, Id};
use crate::overhead::Camera;

fn floats(world: Id, ivar: &str) -> Option<[f32; 16]> {
    let p = objc::ivar_ptr(world, "World", ivar) as *const [f32; 16];
    // SAFETY: a GLKMatrix4 ivar of the live World.
    (!p.is_null()).then(|| unsafe { *p })
}

/// The camera of `world` (null: none).
pub fn camera(world: Id) -> Option<Camera> {
    if world.is_null() {
        return None;
    }
    let projection = floats(world, "tapProjectionMatrix")?;
    let modelview = floats(world, "tapModelviewMatrix")?;
    let v = objc::ivar_ptr(world, "World", "tapViewport") as *const [i32; 4];
    if v.is_null() {
        return None;
    }
    // SAFETY: the GLint[4] viewport ivar of the live World.
    let viewport = unsafe { *v };
    Some(Camera { projection, modelview, viewport })
}
