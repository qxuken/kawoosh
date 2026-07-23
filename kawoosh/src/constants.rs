#[cfg(not(debug_assertions))]
pub const DEFAULT_FONT: &str = ".IosevkaNavcon";

#[cfg(debug_assertions)]
pub const DEFAULT_FONT: &str = "Iosevka Term";
