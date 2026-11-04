//! Tiny built-in word lists. For tests, demos and as a last-resort fallback ONLY.
//! Real dictionaries are compiled with `arf-dictc` into FST files.

use crate::dict::WordSet;
use crate::types::Lang;
use crate::validator::Validator;

pub const SEED_AR: &str = include_str!("../data/seed_ar.txt");
pub const SEED_EN: &str = include_str!("../data/seed_en.txt");

pub fn seed_validator(build_models: bool) -> Validator {
    Validator::new(
        Box::new(WordSet::from_text(Lang::Arabic, SEED_AR)),
        Box::new(WordSet::from_text(Lang::English, SEED_EN)),
        build_models,
    )
}
