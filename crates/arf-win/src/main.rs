#[cfg(windows)]
fn main() {
    std::process::exit(arf_win::cli::run());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("arf-win runs on Windows only (this build is only type-checked on this platform).");
    std::process::exit(1);
}
