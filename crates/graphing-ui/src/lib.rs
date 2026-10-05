//! graphing's design system: tokens, theme and component kit.
//!
//! Every surface in the app is built from these. A new look means editing
//! this crate, not hunting through views.

pub mod dock;
pub mod kit;
pub mod menu;
pub mod theme;
pub mod tokens;

pub use theme::{UiExt, install};
pub use tokens::Colors;
