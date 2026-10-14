//! Format and usage translation.
//!
//! ## Status
//!
//! Split out of `WgpuDevice` and `WgpuPipeline`. Two translation tables live in
//! two files today, which is how the `count: Some(1)` bind group layout bug
//! survived a test suite: the mapping was correct where it was written and the
//! other site was written separately.
