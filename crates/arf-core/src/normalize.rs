use crate::types::Lang;

/// Strips diacritics/tatweel and unifies alef forms.
pub fn normalize_arabic(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(*c as u32, 0x064B..=0x065F | 0x0670 | 0x0640))
        .map(|c| match c {
            'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
            c => c,
        })
        .collect()
}

pub fn normalize_english(s: &str) -> String {
    s.to_lowercase()
}

pub fn normalize(lang: Lang, s: &str) -> String {
    match lang {
        Lang::Arabic => normalize_arabic(s),
        Lang::English => normalize_english(s),
    }
}

/// The token without leading/trailing non-letters (symbols and digits are layout-neutral
/// or ambiguous, so validation runs on the letter core).
pub fn core(s: &str) -> &str {
    s.trim_matches(|c: char| !c.is_alphabetic())
}

pub fn letter_count(s: &str) -> usize {
    s.chars().filter(|c| c.is_alphabetic()).count()
}
