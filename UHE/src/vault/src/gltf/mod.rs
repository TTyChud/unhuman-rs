//! glTF loading.
//!
//! Split by concern so each piece is testable without the others: the vertex
//! format is a layout, the optimiser is pure arithmetic over arrays, and the
//! loader is the part that touches the file.

pub mod optimizer;
pub mod vertex;
