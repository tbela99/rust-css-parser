//! Port of `src/ast/transform/*`: the matrices of the transform functions. A matrix is `[f64; 16]` in
//! column-major order (`type.d.ts`); the operations write into matrices provided by the caller.

pub mod compute;
pub mod matrix;
pub mod minify;
pub mod perspective;
pub mod rotate;
pub mod scale;
pub mod skew;
pub mod translate;
pub mod utils;

/// 4x4 matrix, column-major order
pub type Matrix = [f64; 16];

/// decomposition of a 3d matrix: translate → rotate → skew → scale (`DecomposedMatrix3D`)
#[derive(Clone, Copy, Debug, Default)]
pub struct DecomposedMatrix3D {
    pub translate_x: f64,
    pub translate_y: f64,
    pub translate_z: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub scale_z: f64,
    pub skew_xy: f64,
    pub skew_xz: f64,
    pub skew_yz: f64,
    /// rotation axis
    pub rotate_x: f64,
    pub rotate_y: f64,
    pub rotate_z: f64,
    /// rotation angle in degrees
    pub rotate_angle: f64,
    pub perspective_x: f64,
    pub perspective_y: f64,
    pub perspective_z: f64,
    pub perspective_w: f64,
}
