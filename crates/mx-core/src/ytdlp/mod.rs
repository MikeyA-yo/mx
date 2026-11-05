//! Thin, typed wrapper around the yt-dlp CLI.
//!
//! We drive yt-dlp as a subprocess rather than embedding Python: extractors
//! break weekly and `yt-dlp -U` / `pip install -U yt-dlp` fixes them without
//! rebuilding this binary.

pub mod model;
pub mod progress;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::cancel::CancelToken;
use crate::error::ExtractError;
use crate::platform::Platform;
use crate::process;
use model::MediaInfo;
use progress::Event;

#[derive(Debug, Default, Clone)]
pub struct Auth {
    pub cookies_file: Option<PathBuf>,
    /// e.g. "firefox", "chrome", "edge:Profile 1"
    pub cookies_from_browser: Option<String>,
}

impl Auth {
    pub fn is_set(&self) -> bool {
        self.cookies_file.is_some() || self.cookies_from_browser.is_some()
    }
}

/// What to fetch and how to package it.
#[derive(Debug, Clone)]
pub enum Selection {
    /// Best video+audio, optionally capped at a height (1080, 720, ...).
    Best { max_height: Option<u32> },
    /// Audio only; `codec` re-encodes via ffmpeg (mp3, m4a, opus, ...),
    /// `None` keeps the original stream untouched.
    Audio { codec: Option<String> },
    /// Raw yt-dlp format expression, e.g. "137+140" from `mx formats`.
    Raw(String),
}

/// Container to save video in. Changing it is a remux: the same streams
/// copied into a different file format, lossless and near-instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    Mkv,
    /// Only holds VP8/VP9/AV1 video with Opus/Vorbis audio.
    Webm,
}

impl Container {
    pub fn ext(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub output_dir: PathBuf,
    pub selection: Selection,
    pub playlist: bool,
    /// Prefer H.264/AAC over VP9/AV1/Opus: easier to edit, plays everywhere.
    pub compat: bool,
    /// Force this container; `None` = mp4 when the streams allow, else mkv.
    pub container: Option<Container>,
    /// Write `<name>.info.json` next to the media, for later pipeline stages.
    pub info_json: bool,
}

#[derive(Debug)]
pub struct DownloadOutcome {
    pub files: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

pub struct YtDlp {
    program: OsString,
    /// Extra leading args, e.g. `-m yt_dlp` when running through Python.
    prefix: Vec<OsString>,
    auth: Auth,
    verbose: bool,
}

impl YtDlp {
    /// Find a working yt-dlp: explicit path, then `YTDLP_PATH`, then `yt-dlp`
    /// on PATH, then the Python module (pip `--user` installs often leave the
    /// script directory off PATH, especially on Windows).
    pub fn locate(explicit: Option<PathBuf>, auth: Auth, verbose: bool) -> Result<Self> {
        let mut candidates: Vec<(OsString, Vec<OsString>)> = Vec::new();
        if let Some(path) = explicit.or_else(|| std::env::var_os("YTDLP_PATH").map(PathBuf::from)) {
            candidates.push((path.into(), vec![]));
        }
        candidates.push(("yt-dlp".into(), vec![]));
        for python in ["python", "python3", "py"] {
            candidates.push((python.into(), vec!["-m".into(), "yt_dlp".into()]));
        }

        for (program, prefix) in candidates {
            let ok = crate::process::command(&program)
                .args(&prefix)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                return Ok(Self { program, prefix, auth, verbose });
            }
        }
        bail!("yt-dlp not found. Install it with `pip install -U \"yt-dlp[default]\"` or set YTDLP_PATH")
    }

    /// How yt-dlp is being invoked, for `mx doctor`.
    pub fn invocation(&self) -> String {
        std::iter::once(&self.program)
            .chain(&self.prefix)
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn version(&self) -> Result<String> {
        let out = self.base_command().arg("--version").output()?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Whether yt-dlp can impersonate a real browser's network fingerprint
    /// (needs the optional `curl_cffi` package). TikTok answers requests
    /// that can't with a bot challenge instead of the video page.
    pub fn can_impersonate(&self) -> Result<bool> {
        let out = self.base_command().arg("--list-impersonate-targets").output()?;
        Ok(String::from_utf8_lossy(&out.stdout).contains("curl_cffi"))
    }

    fn base_command(&self) -> Command {
        let mut cmd = crate::process::command(&self.program);
        cmd.args(&self.prefix)
            // Child Python inherits the Windows ANSI code page otherwise, and
            // titles with emoji or CJK come back mangled.
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            // Ignore the user's yt-dlp config so runs are reproducible.
            .arg("--ignore-config");
        if let Some(file) = &self.auth.cookies_file {
            cmd.arg("--cookies").arg(file);
        }
        if let Some(browser) = &self.auth.cookies_from_browser {
            cmd.args(["--cookies-from-browser", browser]);
        }
        cmd
    }

    fn log(&self, cmd: &Command) {
        if self.verbose {
            let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy()).collect();
            eprintln!("$ {} {}", cmd.get_program().to_string_lossy(), args.join(" "));
        }
    }

    /// Raw metadata JSON. Playlists are listed flat (ids/titles only) so a
    /// 500-video channel doesn't trigger 500 page fetches.
    pub fn dump_json(&self, url: &str, playlist: bool) -> Result<serde_json::Value> {
        let mut cmd = self.base_command();
        cmd.arg("--dump-single-json")
            .arg(if playlist { "--flat-playlist" } else { "--no-playlist" })
            .arg("--")
            .arg(url);
        self.log(&cmd);

        let out = cmd.output().context("failed to run yt-dlp")?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(ExtractError::classify(&stderr, Platform::detect(url)).into());
        }
        if self.verbose {
            eprint!("{}", String::from_utf8_lossy(&out.stderr));
        }
        serde_json::from_slice(&out.stdout).context("yt-dlp returned invalid JSON")
    }

    pub fn info(&self, url: &str, playlist: bool) -> Result<MediaInfo> {
        let json = self.dump_json(url, playlist)?;
        serde_json::from_value(json).context("unexpected yt-dlp JSON shape")
    }

    pub fn download(
        &self,
        url: &str,
        req: &DownloadRequest,
        cancel: &CancelToken,
        mut on_event: impl FnMut(Event),
    ) -> Result<DownloadOutcome> {
        if req.compat && req.container == Some(Container::Webm) {
            bail!("--compat picks H.264/AAC, which a webm file can't hold; use mp4 or mkv");
        }
        let mut cmd = self.base_command();
        cmd.args(progress::template_args())
            .args(download_args(req))
            .arg("--")
            .arg(url);
        self.log(&cmd);

        let mut files = Vec::new();
        let finished = process::run_streaming(cmd, cancel, |line| {
            match progress::parse_line(line) {
                Some(Event::Done(path)) => {
                    files.push(path.clone());
                    on_event(Event::Done(path));
                }
                Some(event) => on_event(event),
                None if self.verbose && !line.trim().is_empty() => eprintln!("{line}"),
                None => {}
            }
        })?;

        let stderr = finished.stderr;
        if !finished.status.success() {
            let err = anyhow::Error::from(ExtractError::classify(&stderr, Platform::detect(url)));
            return Err(match req.container {
                Some(Container::Webm) => err.context(
                    "this video isn't offered in webm-compatible codecs (VP9/AV1 + Opus); use mkv or mp4",
                ),
                Some(c) if stderr.to_lowercase().contains("postprocessing") => err.context(format!(
                    "ffmpeg couldn't put these codecs in a .{} file; mkv holds any codec",
                    c.ext()
                )),
                _ => err,
            });
        }
        let warnings = stderr
            .lines()
            .filter_map(|l| l.strip_prefix("WARNING:"))
            .map(|l| l.trim().to_string())
            .collect();
        Ok(DownloadOutcome { files, warnings })
    }
}

/// yt-dlp format filters for codecs a webm file can hold. Codec strings
/// look like "vp9", "vp09.00.10.08", "av01.0.05M.08", "opus".
const WEBM_VIDEO: &str = "[vcodec~='^(vp0?[89]|av0?1)']";
const WEBM_AUDIO: &str = "[acodec~='^(opus|vorbis)']";

/// Translate a request into yt-dlp flags. Kept separate from process
/// handling so the mapping is unit-testable.
fn download_args(req: &DownloadRequest) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |a: &str| args.push(a.into());

    match &req.selection {
        Selection::Best { max_height } => {
            let cap = max_height.map(|h| format!("[height<={h}]")).unwrap_or_default();
            // webm is the one container with codec limits, so pick only
            // streams it can hold rather than download and fail the remux.
            let (v, a) = if req.container == Some(Container::Webm) {
                (WEBM_VIDEO, WEBM_AUDIO)
            } else {
                ("", "")
            };
            push("-f");
            push(&format!("bv*{v}{cap}+ba{a}/b{v}{a}{cap}"));
            if req.compat {
                // Rank H.264 above resolution: a 1080p H.264 beats 4K AV1
                // when the goal is "opens in any editor".
                push("-S");
                push("vcodec:h264,res,acodec:aac");
            }
            // yt-dlp only merges into mp4 when it's sure both codecs fit and
            // is conservative about VP9, so the default falls back to mkv.
            let merge = match (req.container, req.compat) {
                (Some(c), _) => c.ext(),
                (None, true) => "mp4",
                (None, false) => "mp4/mkv",
            };
            push("--merge-output-format");
            push(merge);
        }
        Selection::Audio { codec } => {
            push("-f");
            push("ba/b");
            push("--extract-audio");
            if let Some(codec) = codec {
                push("--audio-format");
                push(codec);
            }
        }
        Selection::Raw(expr) => {
            push("-f");
            push(expr);
        }
    }

    // Merging covers separate video+audio downloads; remuxing also covers
    // sites that serve one combined file. Fails (rather than re-encoding)
    // if the codecs don't fit, e.g. H.264 into webm.
    if let Some(container) = req.container
        && !matches!(req.selection, Selection::Audio { .. })
    {
        push("--remux-video");
        push(container.ext());
    }

    push(if req.playlist { "--yes-playlist" } else { "--no-playlist" });
    if req.info_json {
        push("--write-info-json");
    }

    // <out>/<Extractor>/[<playlist>/]<title> [<id>].<ext>
    push("-o");
    push("%(extractor_key)s/%(playlist_title&{}/|)s%(title).150B [%(id)s].%(ext)s");
    args.push("-P".into());
    args.push(req.output_dir.clone().into_os_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(selection: Selection) -> DownloadRequest {
        DownloadRequest {
            output_dir: "out".into(),
            selection,
            playlist: false,
            compat: false,
            container: None,
            info_json: false,
        }
    }

    fn args_of(r: &DownloadRequest) -> Vec<String> {
        download_args(r).into_iter().map(|a| a.into_string().unwrap()).collect()
    }

    fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let i = args.iter().position(|a| a == flag)?;
        args.get(i + 1).map(String::as_str)
    }

    #[test]
    fn height_cap_applies_to_both_merged_and_fallback_formats() {
        let args = args_of(&req(Selection::Best { max_height: Some(720) }));
        assert_eq!(value_after(&args, "-f"), Some("bv*[height<=720]+ba/b[height<=720]"));
        assert_eq!(value_after(&args, "--merge-output-format"), Some("mp4/mkv"));
        assert!(args.contains(&"--no-playlist".to_string()));
    }

    #[test]
    fn compat_mode_prefers_h264_and_forces_mp4() {
        let mut r = req(Selection::Best { max_height: None });
        r.compat = true;
        let args = args_of(&r);
        assert_eq!(value_after(&args, "-f"), Some("bv*+ba/b"));
        assert_eq!(value_after(&args, "-S"), Some("vcodec:h264,res,acodec:aac"));
        assert_eq!(value_after(&args, "--merge-output-format"), Some("mp4"));
    }

    #[test]
    fn container_overrides_merge_target_and_remuxes() {
        let mut r = req(Selection::Best { max_height: None });
        r.container = Some(Container::Mp4);
        let args = args_of(&r);
        assert_eq!(value_after(&args, "--merge-output-format"), Some("mp4"));
        assert_eq!(value_after(&args, "--remux-video"), Some("mp4"));

        // Also applies to explicit format picks, which may be one combined file.
        let mut r = req(Selection::Raw("18".into()));
        r.container = Some(Container::Mkv);
        assert_eq!(value_after(&args_of(&r), "--remux-video"), Some("mkv"));

        // Wins over --compat's mp4, which then only steers codec choice.
        let mut r = req(Selection::Best { max_height: None });
        r.compat = true;
        r.container = Some(Container::Mkv);
        assert_eq!(value_after(&args_of(&r), "--merge-output-format"), Some("mkv"));
    }

    #[test]
    fn webm_selects_only_streams_it_can_hold() {
        let mut r = req(Selection::Best { max_height: Some(720) });
        r.container = Some(Container::Webm);
        let args = args_of(&r);
        assert_eq!(
            value_after(&args, "-f"),
            Some(concat!(
                "bv*[vcodec~='^(vp0?[89]|av0?1)'][height<=720]+ba[acodec~='^(opus|vorbis)']",
                "/b[vcodec~='^(vp0?[89]|av0?1)'][acodec~='^(opus|vorbis)'][height<=720]"
            ))
        );
    }

    #[test]
    fn audio_only_extracts_and_optionally_transcodes() {
        let args = args_of(&req(Selection::Audio { codec: Some("mp3".into()) }));
        assert!(args.contains(&"--extract-audio".to_string()));
        assert_eq!(value_after(&args, "--audio-format"), Some("mp3"));

        let args = args_of(&req(Selection::Audio { codec: None }));
        assert!(!args.contains(&"--audio-format".to_string()));
    }
}
