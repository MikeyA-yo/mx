use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use mx_core::ffmpeg::ops::Aspect;
use mx_core::analysis::Threshold;
use mx_core::units;
use mx_core::ytdlp::Container;

#[derive(Parser)]
#[command(name = "mx", version, about = "Media extractor and processor built on yt-dlp + ffmpeg")]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Args)]
pub struct GlobalArgs {
    /// Netscape-format cookies.txt for sites that need a login
    #[arg(long, global = true, value_name = "FILE")]
    pub cookies: Option<PathBuf>,

    /// Read cookies from a local browser (firefox, chrome, edge, brave, ...)
    #[arg(long, global = true, value_name = "BROWSER")]
    pub cookies_from_browser: Option<String>,

    /// Path to the yt-dlp executable [env: YTDLP_PATH]
    #[arg(long, global = true, value_name = "PATH")]
    pub yt_dlp: Option<PathBuf>,

    /// Replace output files that already exist
    #[arg(short = 'y', long, global = true)]
    pub overwrite: bool,

    /// Show the commands being run and their raw output
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

fn time(s: &str) -> Result<f64, String> {
    units::parse_time(s).map_err(|e| e.to_string())
}

/// "auto", "-30", "-30dB"
fn threshold(s: &str) -> Result<Threshold, String> {
    if s.eq_ignore_ascii_case("auto") {
        return Ok(Threshold::Auto);
    }
    let number = s.trim().trim_end_matches(['B', 'b']).trim_end_matches(['d', 'D']);
    number
        .parse::<f64>()
        .map(Threshold::Db)
        .map_err(|_| format!("invalid threshold {s:?}; use auto or a level like -30dB"))
}

fn size(s: &str) -> Result<u64, String> {
    units::parse_size(s).map_err(|e| e.to_string())
}

#[derive(Subcommand)]
pub enum Command {
    /// Show metadata for a URL without downloading
    Info {
        url: String,
        /// Print yt-dlp's full JSON instead of a summary
        #[arg(long)]
        json: bool,
        /// Treat the URL as a playlist/channel and list its entries
        #[arg(long)]
        playlist: bool,
    },
    /// List every format (resolution/codec/bitrate) the site offers
    Formats { url: String },
    /// Download media
    Download(DownloadArgs),
    /// Inspect a local media file with ffprobe
    Probe { file: PathBuf },
    /// Re-encode a local file, using a GPU encoder when one works
    Convert {
        input: PathBuf,
        /// Output file; the extension picks the container (mp4, mkv, mov, ...)
        output: PathBuf,
        #[command(flatten)]
        encode: EncodeArgs,
    },
    /// Change the container without re-encoding: lossless and near-instant
    Remux {
        input: PathBuf,
        /// Output file [default: input name with the --to extension]
        output: Option<PathBuf>,
        /// Container to use when no output file is given
        #[arg(short, long, value_enum, default_value_t = ContainerArg::Mp4, conflicts_with = "output")]
        to: ContainerArg,
    },
    /// Cut a time range (frame-exact by default, --fast for instant keyframe cuts)
    Trim {
        input: PathBuf,
        output: PathBuf,
        /// Start time: seconds (90) or [h:]m:s (1:30)
        #[arg(long, default_value = "0", value_parser = time)]
        from: f64,
        /// End time [default: end of input]
        #[arg(long, value_parser = time, conflicts_with = "length")]
        to: Option<f64>,
        /// Length instead of an end time
        #[arg(long, value_parser = time)]
        length: Option<f64>,
        /// Copy without re-encoding: instant and lossless, but starts at the
        /// nearest keyframe at or before --from
        #[arg(long)]
        fast: bool,
        #[command(flatten)]
        encode: EncodeArgs,
    },
    /// Change the frame shape, e.g. landscape to 9:16 for Shorts/Reels/TikTok
    Reframe {
        input: PathBuf,
        output: PathBuf,
        /// Target aspect ratio
        #[arg(short, long, default_value = "9:16")]
        aspect: Aspect,
        /// How to fill the new frame
        #[arg(long, value_enum, default_value_t = FitArg::Blur)]
        fit: FitArg,
        /// With --fit crop: which part to keep, 0 (left/top) to 1 (right/bottom)
        #[arg(long, default_value_t = 0.5)]
        position: f64,
        /// Length of the shorter side in pixels (1080 -> 1080x1920 for 9:16)
        #[arg(long, default_value_t = 1080)]
        short_edge: u32,
        #[command(flatten)]
        encode: EncodeArgs,
    },
    /// Find the pauses in a recording (analysis only)
    Silence {
        input: PathBuf,
        #[command(flatten)]
        detect: SilenceArgs,
    },
    /// Remove pauses: tighten talking-head videos, tutorials and podcasts
    Jumpcut {
        input: PathBuf,
        output: PathBuf,
        #[command(flatten)]
        detect: SilenceArgs,
        #[command(flatten)]
        encode: EncodeArgs,
    },
    /// Find the most eventful moments (loud, lively, busy picture, fast cuts)
    /// and optionally export them as clips
    Highlights {
        input: PathBuf,
        /// How many highlights to find
        #[arg(short = 'n', long, default_value_t = 3)]
        count: usize,
        /// Length of each highlight; edges move up to 10% to land on a pause or cut
        #[arg(short, long, default_value = "30", value_parser = time)]
        length: f64,
        /// Only search from this time on
        #[arg(long, value_parser = time)]
        from: Option<f64>,
        /// Only search up to this time (e.g. where the end credits start)
        #[arg(long, value_parser = time)]
        to: Option<f64>,
        /// Render each highlight as a clip into this directory
        #[arg(long, value_name = "DIR")]
        export: Option<PathBuf>,
        /// Export vertical 9:16 clips for Shorts/Reels/TikTok
        #[arg(long, requires = "export", conflicts_with = "aspect")]
        vertical: bool,
        /// Export clips in this aspect ratio instead, e.g. 1:1
        #[arg(short, long, requires = "export")]
        aspect: Option<Aspect>,
        /// How reframed clips fill the new frame
        #[arg(long, value_enum, default_value_t = FitArg::Blur)]
        fit: FitArg,
        /// With --fit crop: which part to keep, 0 (left/top) to 1 (right/bottom)
        #[arg(long, default_value_t = 0.5)]
        position: f64,
        /// Shorter side of reframed clips in pixels [default: the source's, at most 1080]
        #[arg(long)]
        short_edge: Option<u32>,
        /// Print the highlights as JSON
        #[arg(long, conflicts_with = "export")]
        json: bool,
        #[command(flatten)]
        encode: EncodeArgs,
    },
    /// Shrink a file to fit a size limit (two-pass encode)
    Compress {
        input: PathBuf,
        output: PathBuf,
        /// Target size, e.g. 10MB, 700KB (decimal units)
        #[arg(short, long, value_parser = size)]
        size: u64,
        /// HEVC gives better quality per MB but plays in fewer places
        #[arg(short, long, value_enum, default_value_t = CompressCodecArg::H264)]
        codec: CompressCodecArg,
    },
    /// Speed up or slow down (audio keeps its pitch)
    Speed {
        input: PathBuf,
        output: PathBuf,
        /// 2 = twice as fast, 0.5 = half speed
        factor: f64,
        #[command(flatten)]
        encode: EncodeArgs,
    },
    /// Make a GIF with a palette tuned to the clip
    Gif {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value = "0", value_parser = time)]
        from: f64,
        #[arg(long, value_parser = time)]
        to: Option<f64>,
        #[arg(long, default_value_t = 15)]
        fps: u32,
        /// Width in pixels (never upscales)
        #[arg(long, default_value_t = 480)]
        width: u32,
    },
    /// Check that yt-dlp, ffmpeg, ffprobe and a JS runtime are available
    Doctor,
}

#[derive(Args)]
pub struct DownloadArgs {
    pub url: String,

    /// Output directory (files go into <dir>/<Site>/...)
    #[arg(short, long, default_value = "downloads")]
    pub output: PathBuf,

    /// Cap video height, e.g. 1080, 720, 480
    #[arg(short = 'q', long, value_name = "HEIGHT", conflicts_with_all = ["audio_only", "format"])]
    pub max_height: Option<u32>,

    /// Audio only
    #[arg(short = 'x', long, conflicts_with = "format")]
    pub audio_only: bool,

    /// Re-encode extracted audio (mp3, m4a, opus, flac, wav)
    #[arg(long, requires = "audio_only", value_name = "CODEC")]
    pub audio_format: Option<String>,

    /// Raw yt-dlp format expression, e.g. "137+140" (see `mx formats`)
    #[arg(short, long)]
    pub format: Option<String>,

    /// Prefer H.264/AAC in an mp4 (best for editors and old players)
    #[arg(long, conflicts_with_all = ["audio_only", "format"])]
    pub compat: bool,

    /// Save in this container (lossless remux, no re-encoding)
    #[arg(long, value_enum, conflicts_with = "audio_only")]
    pub container: Option<ContainerArg>,

    /// Download the whole playlist when the URL points into one
    #[arg(long)]
    pub playlist: bool,

    /// Save yt-dlp's metadata as <name>.info.json beside the media
    #[arg(long)]
    pub info_json: bool,
}

#[derive(Args)]
pub struct SilenceArgs {
    /// Level below which audio counts as silence: auto adapts to the
    /// recording's noise floor, or give a level like -35dB
    #[arg(short, long, default_value = "auto", value_parser = threshold, allow_hyphen_values = true)]
    pub threshold: Threshold,

    /// Shortest pause to remove, in seconds (shorter gaps are between words)
    #[arg(long, default_value_t = 0.4)]
    pub min_silence: f64,

    /// Silence left at each side of a cut, in seconds, so speech isn't clipped
    #[arg(long, default_value_t = 0.1)]
    pub padding: f64,
}

/// Encoder choice for every command that re-encodes video.
#[derive(Args)]
pub struct EncodeArgs {
    /// Target video codec
    #[arg(short, long, value_enum, default_value_t = CodecArg::H264)]
    pub codec: CodecArg,

    /// Exact encoder instead of auto-picking (libx264, h264_qsv, hevc_nvenc, ...)
    #[arg(short, long, conflicts_with_all = ["codec", "cpu"])]
    pub encoder: Option<String>,

    /// Never use a hardware encoder
    #[arg(long)]
    pub cpu: bool,

    /// Constant quality in the encoder's own scale, lower = better [default: per encoder]
    #[arg(long)]
    pub quality: Option<u32>,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum CodecArg {
    H264,
    Hevc,
    Av1,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ContainerArg {
    Mp4,
    Mkv,
    /// VP9/AV1 + Opus only
    Webm,
}

impl ContainerArg {
    pub fn core(self) -> Container {
        match self {
            Self::Mp4 => Container::Mp4,
            Self::Mkv => Container::Mkv,
            Self::Webm => Container::Webm,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum CompressCodecArg {
    H264,
    Hevc,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum FitArg {
    /// Whole picture over a blurred copy of itself
    Blur,
    /// Fill the frame, cutting off the edges
    Crop,
    /// Whole picture with black bars
    Pad,
}
