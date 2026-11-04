//! Platform-independent engine of Auto-Reverse Typo Fixer.
//!
//! Pipeline: keystrokes -> [`keybuffer`] -> [`converter`] -> [`validator`] -> [`decision`].
//! Nothing in this crate touches the OS, the network, or the disk.

pub mod config;
pub mod converter;
pub mod decision;
pub mod dict;
pub mod keybuffer;
pub mod layouts;
pub mod ngram;
pub mod normalize;
pub mod prefilter;
pub mod seed;
pub mod types;
pub mod validator;

pub use decision::{Config, Correction, Engine, Sensitivity, UndoPause, UndoPlan, WordFix};
pub use types::{Key, KeyboardLayout, Lang, LayoutId};

#[cfg(test)]
mod tests;
