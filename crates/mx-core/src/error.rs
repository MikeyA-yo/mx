use crate::platform::Platform;

/// yt-dlp failures, sorted into the handful of causes you can actually act on.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error(
        "{site} is asking this IP to prove it isn't a bot.\n  \
         This is about the network, not you: shared IPs (office Wi-Fi, VPNs, cloud servers) get flagged.\n  \
         Fix: switch networks (e.g. mobile data), or pass cookies from a logged-in browser (--cookies-from-browser firefox)"
    )]
    BotCheck { site: &'static str },

    #[error(
        "couldn't read the browser's cookie database: it's locked while the browser is running.\n  \
         Fix: fully quit the browser (including background/tray processes) and retry,\n  \
         or export cookies.txt and pass --cookies <file>"
    )]
    CookiesLocked,

    #[error(
        "couldn't decrypt the browser's cookies. Chrome-based browsers on Windows use App-Bound\n  \
         Encryption, which yt-dlp can't read even with the browser closed.\n  \
         Fix: use --cookies-from-browser firefox, or export cookies.txt with a browser extension\n  \
         (e.g. \"Get cookies.txt LOCALLY\") and pass --cookies <file>"
    )]
    CookiesEncrypted,

    #[error(
        "{site} requires a logged-in session for this content.\n  \
         Fix: --cookies-from-browser <browser> or --cookies <cookies.txt>"
    )]
    LoginRequired { site: &'static str },

    #[error("content is private, removed, or otherwise unavailable: {0}")]
    Unavailable(String),

    #[error("content is geo-restricted from this location: {0}")]
    GeoBlocked(String),

    #[error("no yt-dlp extractor recognises this URL")]
    Unsupported,

    #[error("none of the site's formats match what was asked for (see `mx formats <url>`)")]
    NoMatchingFormat,

    #[error("yt-dlp failed: {0}")]
    Other(String),
}

impl ExtractError {
    pub fn classify(stderr: &str, platform: Platform) -> Self {
        let message = stderr
            .lines()
            .rev()
            .find_map(|l| l.trim().strip_prefix("ERROR:"))
            .map(str::trim)
            .unwrap_or_else(|| stderr.trim())
            .to_string();
        let lower = message.to_lowercase();
        let site = platform.name();
        let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));

        // Cookie failures first: they happen before any site request, and the
        // generic login advice ("use --cookies-from-browser") is what caused them.
        if has(&["could not copy chrome cookie database"]) {
            Self::CookiesLocked
        } else if has(&["failed to decrypt with dpapi"]) {
            Self::CookiesEncrypted
        } else if has(&["not a bot", "confirm you're not", "confirm you\u{2019}re not"]) {
            Self::BotCheck { site }
        } else if has(&[
            "logged-in",
            "login required",
            "log in",
            "sign in",
            "requires authentication",
            "use --cookies",
        ]) {
            Self::LoginRequired { site }
        } else if has(&["unsupported url"]) {
            Self::Unsupported
        } else if has(&["requested format is not available"]) {
            // Checked before `Unavailable`, which also matches "not available".
            Self::NoMatchingFormat
        } else if has(&["not available in your country", "geo restrict", "geo-restrict"]) {
            Self::GeoBlocked(message)
        } else if has(&["private video", "video unavailable", "has been removed", "not available"]) {
            Self::Unavailable(message)
        } else {
            Self::Other(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(line: &str) -> ExtractError {
        ExtractError::classify(line, Platform::YouTube)
    }

    #[test]
    fn classifies_real_ytdlp_messages() {
        // Verbatim from yt-dlp 2026.10.21.
        assert!(matches!(
            classify("ERROR: [youtube] jNQXAC9IVRw: Sign in to confirm you\u{2019}re not a bot. Use --cookies-from-browser"),
            ExtractError::BotCheck { .. }
        ));
        assert!(matches!(
            classify("ERROR: [vimeo] 76979871: The web client only works when logged-in. Use --cookies"),
            ExtractError::LoginRequired { .. }
        ));
        assert!(matches!(
            classify("ERROR: Unsupported URL: https://example.com/"),
            ExtractError::Unsupported
        ));
        assert!(matches!(
            classify("ERROR: [Instagram] De63Np3RuWo: Requested format is not available. Use --list-formats"),
            ExtractError::NoMatchingFormat
        ));
        assert!(matches!(
            classify("ERROR: [youtube] abc: Private video. Sign in if you've been granted access"),
            // "Sign in" wins: cookies are the actionable fix for a private video.
            ExtractError::LoginRequired { .. }
        ));
    }

    #[test]
    fn classifies_windows_browser_cookie_failures() {
        // Verbatim from yt_dlp/cookies.py: Brave/Chrome while running, and
        // Chrome 127+ App-Bound Encryption.
        assert!(matches!(
            classify("ERROR: Could not copy Chrome cookie database. See  https://github.com/yt-dlp/yt-dlp/issues/7271  for more info"),
            ExtractError::CookiesLocked
        ));
        assert!(matches!(
            classify("ERROR: Failed to decrypt with DPAPI. See  https://github.com/yt-dlp/yt-dlp/issues/10927  for more info"),
            ExtractError::CookiesEncrypted
        ));
    }

    #[test]
    fn uses_last_error_line_and_ignores_warnings() {
        let stderr = "WARNING: something minor\nERROR: [x] 1: weird failure\n";
        match classify(stderr) {
            ExtractError::Other(msg) => assert_eq!(msg, "[x] 1: weird failure"),
            other => panic!("unexpected {other:?}"),
        }
    }
}
