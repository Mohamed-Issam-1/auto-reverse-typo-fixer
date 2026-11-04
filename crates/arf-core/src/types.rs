use zeroize::Zeroize;

/// A physical keystroke. The engine stores keys, never the characters they produced,
/// because different keys can yield the same character in one layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Key {
    /// Hardware scan code (set 1) of a main-block typing key.
    pub scan: u16,
    pub shift: bool,
    pub caps: bool,
}

impl Zeroize for Key {
    fn zeroize(&mut self) {
        self.scan.zeroize();
        self.shift.zeroize();
        self.caps.zeroize();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Arabic,
    English,
}

impl Lang {
    pub fn other(self) -> Lang {
        match self {
            Lang::Arabic => Lang::English,
            Lang::English => Lang::Arabic,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Lang::Arabic => "ar",
            Lang::English => "en",
        }
    }
    pub fn from_code(s: &str) -> Option<Lang> {
        match s {
            "ar" => Some(Lang::Arabic),
            "en" => Some(Lang::English),
            _ => None,
        }
    }
}

/// Opaque identifier of an installed layout (the HKL value on Windows).
pub type LayoutId = u64;

/// Translates a physical key into the text it produces under a given layout.
/// On Windows this is implemented with `ToUnicodeEx`; in tests with static tables.
pub trait KeyboardLayout: Send + Sync {
    fn lang(&self) -> Lang;
    /// `None` for keys that produce nothing (or a dead key) under this layout.
    fn translate(&self, key: Key) -> Option<String>;
}

/// Set-1 scan codes used by the engine.
pub mod scan {
    pub const SPACE: u16 = 0x39;
    pub const BACKSPACE: u16 = 0x0E;

    /// Main-block typing keys, in the order: `` ` `` 1-0 - = q-p [ ] \ a-l ; ' z-m , . /
    pub const TYPING_KEYS: [u16; 47] = [
        0x29, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x10, 0x11,
        0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x2B, 0x1E, 0x1F, 0x20, 0x21,
        0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33,
        0x34, 0x35,
    ];

    pub fn is_typing_key(scan: u16) -> bool {
        TYPING_KEYS.contains(&scan)
    }
}
