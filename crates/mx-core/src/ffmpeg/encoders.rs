//! Which video encoders actually work here.
//!
//! `ffmpeg -encoders` lists what was *compiled in*, not what runs: a build
//! can include NVENC on a machine with no NVIDIA GPU. The only reliable test
//! is a tiny real encode, so that's what [`detect`] does.

use std::thread;

use serde::Serialize;

use super::Job;
use crate::cancel::CancelToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Software,
    /// NVIDIA
    Nvenc,
    /// Intel Quick Sync
    Qsv,
    /// AMD
    Amf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Encoder {
    pub name: &'static str,
    pub codec: Codec,
    pub backend: Backend,
}

const fn enc(name: &'static str, codec: Codec, backend: Backend) -> Encoder {
    Encoder { name, codec, backend }
}

/// Preference order within each codec: GPU first (several times faster),
/// software last (always available, best quality per bit).
pub const KNOWN: &[Encoder] = &[
    enc("h264_nvenc", Codec::H264, Backend::Nvenc),
    enc("h264_qsv", Codec::H264, Backend::Qsv),
    enc("h264_amf", Codec::H264, Backend::Amf),
    enc("libx264", Codec::H264, Backend::Software),
    enc("hevc_nvenc", Codec::Hevc, Backend::Nvenc),
    enc("hevc_qsv", Codec::Hevc, Backend::Qsv),
    enc("hevc_amf", Codec::Hevc, Backend::Amf),
    enc("libx265", Codec::Hevc, Backend::Software),
    enc("av1_nvenc", Codec::Av1, Backend::Nvenc),
    enc("av1_qsv", Codec::Av1, Backend::Qsv),
    enc("av1_amf", Codec::Av1, Backend::Amf),
    enc("libsvtav1", Codec::Av1, Backend::Software),
];

impl Encoder {
    pub fn lookup(name: &str) -> Option<Self> {
        KNOWN.iter().copied().find(|e| e.name == name)
    }

    pub fn is_hardware(&self) -> bool {
        self.backend != Backend::Software
    }

    /// A sensible constant-quality starting point, in this encoder's own
    /// scale. Scales are NOT comparable across encoders: CRF 23 on x264 and
    /// global_quality 23 on QSV produce different files.
    pub fn default_quality(&self) -> u32 {
        match (self.backend, self.codec) {
            (Backend::Software, Codec::H264) => 23,
            (Backend::Software, Codec::Hevc) => 28,
            (Backend::Software, Codec::Av1) => 35,
            (Backend::Qsv | Backend::Amf, _) => 23,
            (Backend::Nvenc, _) => 25,
        }
    }

    /// Constant-quality rate control (lower = better, bigger).
    pub fn quality_args(&self, q: u32) -> Vec<String> {
        let q = q.to_string();
        let args: Vec<&str> = match self.backend {
            Backend::Software => vec!["-crf", &q],
            Backend::Nvenc => vec!["-rc", "vbr", "-cq", &q, "-b:v", "0"],
            Backend::Qsv => vec!["-global_quality", &q],
            Backend::Amf => vec!["-rc", "cqp", "-qp_i", &q, "-qp_p", &q],
        };
        args.into_iter().map(String::from).collect()
    }

    /// Force 8-bit 4:2:0 on software encoders so output plays everywhere
    /// (10-bit or 4:4:4 sources would otherwise produce High 10/4:4:4 H.264
    /// that browsers and phones reject). Hardware encoders negotiate their
    /// own surface format, and forcing yuv420p breaks QSV.
    pub fn compat_args(&self) -> &'static [&'static str] {
        if self.is_hardware() { &[] } else { &["-pix_fmt", "yuv420p"] }
    }

    /// Everything needed after the inputs to encode video with this encoder:
    /// codec, constant-quality rate control and the compatibility pixel format.
    pub fn encode_args(&self, quality: Option<u32>) -> Vec<String> {
        let mut args = vec!["-c:v".to_string(), self.name.to_string()];
        args.extend(self.quality_args(quality.unwrap_or_else(|| self.default_quality())));
        args.extend(self.compat_args().iter().map(|s| s.to_string()));
        args
    }

    fn works(&self) -> bool {
        Job::new("-")
            .args(["-f", "lavfi", "-i", "color=black:s=256x256:r=30:d=0.2"])
            .args(["-c:v", self.name])
            .args(self.compat_args())
            .args(["-f", "null"])
            .run(&CancelToken::new(), |_| {})
            .is_ok()
    }
}

/// Test every known encoder in parallel (~1 s total) and return the ones
/// that work, in preference order.
pub fn detect() -> Vec<Encoder> {
    thread::scope(|s| {
        let probes: Vec<_> = KNOWN.iter().map(|e| (e, s.spawn(|| e.works()))).collect();
        probes
            .into_iter()
            .filter_map(|(e, h)| h.join().unwrap_or(false).then_some(*e))
            .collect()
    })
}

/// Below this output height, software encoding is preferred.
///
/// GPU encoders pay a fixed cost per frame (copying it to the GPU and
/// back), so they don't speed up as frames get smaller, while software
/// encoders do. Measured on an Intel iGPU (QuickSync) against libx264:
/// QSV ran ~200-240 fps at both 360p and 1080p; x264 ran ~440 fps at 360p
/// but was 2.8x slower than QSV at 1080p60. The crossover sits around 720p.
pub const HARDWARE_MIN_HEIGHT: u32 = 720;

/// Preferred working encoder for `codec`. `output_height`, when known,
/// steers small outputs to software (see [`HARDWARE_MIN_HEIGHT`]); if no
/// software encoder works, hardware is still used rather than nothing.
pub fn pick(
    available: &[Encoder],
    codec: Codec,
    allow_hardware: bool,
    output_height: Option<u32>,
) -> Option<Encoder> {
    let candidates = available.iter().copied().filter(|e| e.codec == codec);
    let small = output_height.is_some_and(|h| h < HARDWARE_MIN_HEIGHT);
    if !allow_hardware || small {
        let software = candidates.clone().find(|e| !e.is_hardware());
        if software.is_some() || !allow_hardware {
            return software;
        }
    }
    candidates.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_prefers_hardware_and_can_be_forced_to_software() {
        let available = [
            Encoder::lookup("h264_qsv").unwrap(),
            Encoder::lookup("libx264").unwrap(),
            Encoder::lookup("libsvtav1").unwrap(),
        ];
        assert_eq!(pick(&available, Codec::H264, true, None).unwrap().name, "h264_qsv");
        assert_eq!(pick(&available, Codec::H264, false, None).unwrap().name, "libx264");
        assert_eq!(pick(&available, Codec::Av1, true, None).unwrap().name, "libsvtav1");
        assert_eq!(pick(&available, Codec::Hevc, true, None), None);
    }

    #[test]
    fn small_outputs_go_to_software_unless_only_hardware_works() {
        let both = [Encoder::lookup("h264_qsv").unwrap(), Encoder::lookup("libx264").unwrap()];
        assert_eq!(pick(&both, Codec::H264, true, Some(360)).unwrap().name, "libx264");
        assert_eq!(pick(&both, Codec::H264, true, Some(720)).unwrap().name, "h264_qsv");
        assert_eq!(pick(&both, Codec::H264, true, Some(1920)).unwrap().name, "h264_qsv");

        let gpu_only = [Encoder::lookup("hevc_qsv").unwrap()];
        assert_eq!(pick(&gpu_only, Codec::Hevc, true, Some(360)).unwrap().name, "hevc_qsv");
        assert_eq!(pick(&gpu_only, Codec::Hevc, false, Some(360)), None);
    }

    #[test]
    fn quality_flags_match_each_backend() {
        let q = |name: &str| Encoder::lookup(name).unwrap().quality_args(23);
        assert_eq!(q("libx264"), ["-crf", "23"]);
        assert_eq!(q("h264_qsv"), ["-global_quality", "23"]);
        assert_eq!(q("hevc_nvenc"), ["-rc", "vbr", "-cq", "23", "-b:v", "0"]);
    }
}
