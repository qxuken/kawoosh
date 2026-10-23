//! The locale a terminal's process is given where kawoosh was opened
//! without one.
//!
//! An app opened from Finder, the Dock or Spotlight has launchd's
//! environment, which names no locale: no `LANG`, no `LC_*`. A shell
//! started in it is in the C locale, and what reads its locale — `less`
//! paging `git log`, `ls`, readline — takes UTF-8 for bytes it cannot
//! show: `Зобков` comes out `<D0><97><D0><BE>…`. Terminal.app, iTerm2,
//! WezTerm and Alacritty each give their shells `LANG` from the
//! system's locale; so does kawoosh, only where nothing names one
//! already — a locale the user set, in the environment kawoosh was
//! started from or in a terminal's own variables, is theirs.

/// The variables that name the locale of characters, in the order
/// they win: any one set is a locale chosen.
const NAMES: [&str; 3] = ["LC_ALL", "LC_CTYPE", "LANG"];

/// `("LANG", locale)` for a child given `envs` over this process's
/// environment, where neither names a locale; None where one does, or
/// on a system that is not macOS (a Linux session names one).
pub(crate) fn missing(envs: &[(String, String)]) -> Option<(&'static str, String)> {
    let named = |k: &str| {
        envs.iter().any(|(n, v)| n == k && !v.is_empty())
            || std::env::var_os(k).is_some_and(|v| !v.is_empty())
    };
    if NAMES.iter().any(|k| named(k)) {
        return None;
    }
    system().map(|l| ("LANG", l))
}

/// The system's locale as a `LANG`: its identifier with `.UTF-8`
/// (`ru_RU.UTF-8`) where the system has that locale, else
/// `en_US.UTF-8` — a region the language is not had in (`en_RU`) has
/// no locale of its own, and UTF-8 is what matters.
#[cfg(target_os = "macos")]
fn system() -> Option<String> {
    let found = identifier()
        .map(|id| format!("{}.UTF-8", id.split('@').next().unwrap_or(&id)))
        .filter(|l| std::path::Path::new("/usr/share/locale").join(l).is_dir());
    Some(found.unwrap_or_else(|| "en_US.UTF-8".into()))
}

#[cfg(not(target_os = "macos"))]
fn system() -> Option<String> {
    None
}

/// The current locale's identifier, `en_US` or `ru_RU@rg=…`.
#[cfg(target_os = "macos")]
fn identifier() -> Option<String> {
    use core_foundation_sys::base::CFRelease;
    use core_foundation_sys::locale::{CFLocaleCopyCurrent, CFLocaleGetIdentifier};
    use core_foundation_sys::string::{CFStringGetCString, kCFStringEncodingUTF8};
    let mut buf = [0u8; 128];
    // SAFETY: the locale is copied (ours to release) and released once;
    // its identifier is got (not ours), read while the locale lives, into
    // a buffer of the length said.
    let ok = unsafe {
        let locale = CFLocaleCopyCurrent();
        if locale.is_null() {
            return None;
        }
        let id = CFLocaleGetIdentifier(locale);
        let ok = !id.is_null()
            && CFStringGetCString(
                id,
                buf.as_mut_ptr().cast(),
                buf.len() as _,
                kCFStringEncodingUTF8,
            ) != 0;
        CFRelease(locale.cast());
        ok
    };
    if !ok {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0)?;
    String::from_utf8(buf[..end].to_vec())
        .ok()
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A locale a terminal's variables name is left as it is.
    #[test]
    fn a_named_locale_is_kept() {
        let envs = [("LC_ALL".to_string(), "ru_RU.UTF-8".to_string())];
        assert_eq!(missing(&envs), None);
    }

    /// With none named anywhere, macOS gives one in UTF-8 that the
    /// system has.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_locale_is_utf8() {
        let l = system().unwrap();
        assert!(l.ends_with(".UTF-8"), "{l}");
        assert!(
            std::path::Path::new("/usr/share/locale").join(&l).is_dir(),
            "{l}"
        );
    }
}
