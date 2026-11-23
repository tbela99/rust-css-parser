//! Port of `src/ast/features/type.ts`.

/// feature walk mode (`FeatureWalkMode`)
#[allow(non_snake_case)]
pub mod FeatureWalkMode {
    /// pre process
    pub const Pre: u32 = 0x1;
    /// post process
    pub const Post: u32 = 0x2;
}
