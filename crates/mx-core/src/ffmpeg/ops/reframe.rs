//! Changing the frame shape, e.g. landscape 16:9 to vertical 9:16 for
//! Shorts/Reels/TikTok.

use std::path::Path;
use std::str::FromStr;

use anyhow::{Result, anyhow, bail};

use super::{VideoEncode, aac, finish};
use crate::ffmpeg::Job;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aspect {
    pub w: u32,
    pub h: u32,
}

impl FromStr for Aspect {
    type Err = anyhow::Error;

    /// "9:16", "1:1", "4:5", "16:9"
    fn from_str(s: &str) -> Result<Self> {
        let (w, h) = s.split_once(':').ok_or_else(|| anyhow!("aspect must look like 9:16"))?;
        let (w, h): (u32, u32) = (w.trim().parse()?, h.trim().parse()?);
        if w == 0 || h == 0 {
            bail!("aspect sides must be positive");
        }
        Ok(Self { w, h })
    }
}

impl Aspect {
    /// Output size with the shorter side = `short_edge`, both sides even
    /// (4:2:0 video can't have odd dimensions).
    pub fn frame(&self, short_edge: u32) -> (u32, u32) {
        let even = |x: f64| ((x / 2.0).round() as u32).max(1) * 2;
        let short = even(short_edge as f64);
        if self.w <= self.h {
            (short, even(short as f64 * self.h as f64 / self.w as f64))
        } else {
            (even(short as f64 * self.w as f64 / self.h as f64), short)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fit {
    /// Fill the frame and cut off the overflow. `position` 0.0..=1.0 picks
    /// which part survives along the overflowing axis (0.5 = centre).
    Crop { position: f64 },
    /// Whole picture in the middle over a blurred, zoomed copy of itself:
    /// the standard look for landscape clips posted as Shorts.
    Blur,
    /// Whole picture with black bars.
    Pad,
}

/// Graph from `[0:v:0]` to `[v]` that fills a `w`x`h` frame.
pub(super) fn filtergraph(fit: Fit, w: u32, h: u32) -> String {
    // Scale the picture to fit inside / to cover the target frame.
    let contain = format!("scale={w}:{h}:force_original_aspect_ratio=decrease:force_divisible_by=2");
    let cover = |w: u32, h: u32| format!("scale={w}:{h}:force_original_aspect_ratio=increase");
    match fit {
        Fit::Crop { position } => {
            let p = position.clamp(0.0, 1.0);
            // Only one of (iw-ow), (ih-oh) is non-zero after covering, so one
            // position value works for both landscape and portrait targets.
            format!("[0:v:0]{},crop={w}:{h}:(iw-ow)*{p}:(ih-oh)*{p},setsar=1[v]", cover(w, h))
        }
        Fit::Pad => format!("[0:v:0]{contain},pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1[v]"),
        Fit::Blur => {
            // Blurring a full 1080x1920 frame is slow and pointless: shrink the
            // background 8x, blur that, and scale it back up. Same look, a
            // fraction of the work.
            let (bw, bh) = ((w / 16).max(1) * 2, (h / 16).max(1) * 2);
            format!(
                "[0:v:0]split=2[bg][fg];\
                 [bg]{},crop={bw}:{bh},boxblur=4:2,scale={w}:{h},setsar=1[bgb];\
                 [fg]{contain},setsar=1[fgs];\
                 [bgb][fgs]overlay=(W-w)/2:(H-h)/2[v]",
                cover(bw, bh)
            )
        }
    }
}

/// Reframe `input` to `aspect`, with the short side `short_edge` pixels
/// (1080 gives 1080x1920 for 9:16, 1080x1080 for 1:1).
pub fn reframe(
    input: &Path,
    output: &Path,
    aspect: Aspect,
    fit: Fit,
    short_edge: u32,
    video: VideoEncode,
) -> Job {
    let (w, h) = aspect.frame(short_edge);
    let job = Job::new(output)
        .input(input)
        .args(["-filter_complex", &filtergraph(fit, w, h)])
        .args(["-map", "[v]", "-map", "0:a?"])
        .args(video.args())
        .args(aac(192));
    finish(job, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aspects_and_computes_even_frames() {
        let a = |s: &str| s.parse::<Aspect>().unwrap();
        assert_eq!(a("9:16").frame(1080), (1080, 1920));
        assert_eq!(a("1:1").frame(1080), (1080, 1080));
        assert_eq!(a("4:5").frame(1080), (1080, 1350));
        assert_eq!(a("16:9").frame(720), (1280, 720));
        // 21:9 at 1080 is 2520 wide; odd-sized inputs still round to even.
        assert_eq!(a("21:9").frame(1080), (2520, 1080));
        assert_eq!(a("9:16").frame(361), (362, 644));
        assert!("9x16".parse::<Aspect>().is_err());
        assert!("0:16".parse::<Aspect>().is_err());
    }

    #[test]
    fn crop_position_is_applied_on_both_axes() {
        let g = filtergraph(Fit::Crop { position: 0.25 }, 1080, 1920);
        assert!(g.contains("crop=1080:1920:(iw-ow)*0.25:(ih-oh)*0.25"), "{g}");
    }

    #[test]
    fn blur_background_is_downscaled_before_blurring() {
        let g = filtergraph(Fit::Blur, 1080, 1920);
        assert!(g.contains("[bg]scale=134:240:force_original_aspect_ratio=increase,crop=134:240,boxblur"), "{g}");
        assert!(g.contains("scale=1080:1920,setsar=1[bgb]"), "{g}");
    }
}
