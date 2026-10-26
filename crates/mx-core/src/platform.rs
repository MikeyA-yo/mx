//! Site profiles layered on top of yt-dlp's extractors.
//!
//! yt-dlp already ships ~1800 extractors, so we never parse sites ourselves.
//! A `Platform` only carries what *we* know about a site: how to name it, and
//! what usually goes wrong there. Unknown hosts fall through to `Other` and
//! still work through yt-dlp's own extractor matching.

use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    YouTube,
    TikTok,
    Instagram,
    X,
    Facebook,
    Reddit,
    Twitch,
    Vimeo,
    SoundCloud,
    Other,
}

impl Platform {
    pub fn detect(url: &str) -> Self {
        let Ok(parsed) = Url::parse(url) else {
            return Self::Other;
        };
        let Some(host) = parsed.host_str() else {
            return Self::Other;
        };
        let on = |domains: &[&str]| {
            domains
                .iter()
                .any(|d| host == *d || host.ends_with(&format!(".{d}")))
        };

        if on(&["youtube.com", "youtu.be", "youtube-nocookie.com"]) {
            Self::YouTube
        } else if on(&["tiktok.com"]) {
            Self::TikTok
        } else if on(&["instagram.com", "instagr.am"]) {
            Self::Instagram
        } else if on(&["x.com", "twitter.com"]) {
            Self::X
        } else if on(&["facebook.com", "fb.watch", "fb.com"]) {
            Self::Facebook
        } else if on(&["reddit.com", "redd.it"]) {
            Self::Reddit
        } else if on(&["twitch.tv"]) {
            Self::Twitch
        } else if on(&["vimeo.com"]) {
            Self::Vimeo
        } else if on(&["soundcloud.com"]) {
            Self::SoundCloud
        } else {
            Self::Other
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::YouTube => "YouTube",
            Self::TikTok => "TikTok",
            Self::Instagram => "Instagram",
            Self::X => "X",
            Self::Facebook => "Facebook",
            Self::Reddit => "Reddit",
            Self::Twitch => "Twitch",
            Self::Vimeo => "Vimeo",
            Self::SoundCloud => "SoundCloud",
            Self::Other => "this site",
        }
    }

    /// Sites where anonymous extraction fails often enough that it's worth
    /// warning before we even call yt-dlp.
    pub fn usually_needs_login(self) -> bool {
        matches!(self, Self::Instagram | Self::Facebook | Self::Vimeo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_major_sites_and_subdomains() {
        let cases = [
            ("https://www.youtube.com/watch?v=abc", Platform::YouTube),
            ("https://youtu.be/abc", Platform::YouTube),
            ("https://music.youtube.com/watch?v=abc", Platform::YouTube),
            ("https://vm.tiktok.com/ZMabc/", Platform::TikTok),
            ("https://www.instagram.com/reel/abc/", Platform::Instagram),
            ("https://x.com/user/status/1", Platform::X),
            ("https://mobile.twitter.com/user/status/1", Platform::X),
            ("https://fb.watch/abc/", Platform::Facebook),
            ("https://old.reddit.com/r/videos/comments/abc/", Platform::Reddit),
            ("https://www.twitch.tv/videos/1", Platform::Twitch),
            ("https://example.com/video.mp4", Platform::Other),
            ("not a url", Platform::Other),
        ];
        for (url, expected) in cases {
            assert_eq!(Platform::detect(url), expected, "{url}");
        }
    }

    #[test]
    fn lookalike_domains_are_not_matched() {
        assert_eq!(Platform::detect("https://notyoutube.com/x"), Platform::Other);
        assert_eq!(Platform::detect("https://x.com.evil.net/x"), Platform::Other);
    }
}
