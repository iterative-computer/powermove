//! Frame ordering and format rules shared with `shared/image-sequence.ts`,
//! `shared/animated-image.ts`, and `shared/media-formats.ts`.

use anyhow::{Result, bail};

/// Frames the proxy encoder will hold still for (`MAX_SEQUENCE_FRAMES`).
pub const MAX_SEQUENCE_FRAMES: u64 = 2400;
/// `Number.MAX_SAFE_INTEGER`
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

const SEQUENCE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "bmp"];
const NATIVE_VIDEO_EXTENSIONS: [&str; 4] = ["mp4", "mov", "m4v", "webm"];
const CONVERTED_VIDEO_EXTENSIONS: [&str; 17] =
    ["mkv", "avi", "wmv", "asf", "flv", "mpg", "mpeg", "m2v", "mts", "m2ts", "ts", "3gp", "3g2", "mxf", "ogv", "dv", "vob"];
const CONVERTED_IMAGE_EXTENSIONS: [&str; 4] = ["tif", "tiff", "heic", "heif"];

#[derive(Debug, PartialEq, Eq)]
pub struct SequenceFrame {
    pub prefix: String,
    pub index: u64,
    pub extension: String,
}

/// `/^(.*?)(\d+)\.(png|jpe?g|webp|bmp)$/i`: the last number before the
/// extension is the frame index. Padding may vary.
pub fn sequence_frame(name: &str) -> Option<SequenceFrame> {
    let (stem, extension) = name.rsplit_once('.')?;
    let extension = extension.to_ascii_lowercase();
    if !SEQUENCE_EXTENSIONS.contains(&extension.as_str()) {
        return None;
    }
    let prefix = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = &stem[prefix.len()..];
    let index: u64 = digits.parse().ok()?;
    (index <= MAX_SAFE_INTEGER).then(|| SequenceFrame { prefix: prefix.to_owned(), index, extension })
}

/// Sort frames by index, rejecting mixed names and duplicate numbers.
pub fn ordered_sequence<T>(files: Vec<T>, name: impl Fn(&T) -> &str) -> Result<Vec<T>> {
    if files.len() < 2 {
        bail!("Choose at least 2 numbered image frames");
    }
    let mut frames = Vec::with_capacity(files.len());
    let mut first: Option<SequenceFrame> = None;
    for file in files {
        let frame = sequence_frame(name(&file)).filter(|frame| {
            first.as_ref().is_none_or(|first| frame.prefix == first.prefix && frame.extension == first.extension)
        });
        let Some(frame) = frame else {
            bail!("Choose images with the same name and extension, ending in frame numbers (for example frame_0001.png)");
        };
        if first.is_none() {
            first = Some(SequenceFrame { prefix: frame.prefix.clone(), index: frame.index, extension: frame.extension.clone() });
        }
        frames.push((frame.index, file));
    }
    frames.sort_by_key(|(index, _)| *index);
    if let Some(pair) = frames.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        bail!("Duplicate frame number {}", pair[1].0);
    }
    Ok(frames.into_iter().map(|(_, file)| file).collect())
}

pub fn valid_sequence_fps(fps: f64) -> bool {
    fps.is_finite() && (1.0..=240.0).contains(&fps)
}

/// `mediaExtension`: the lowercase extension of a name or path, or "".
pub fn media_extension(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((_, extension)) if !extension.is_empty() && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) => {
            extension.to_ascii_lowercase()
        }
        _ => String::new(),
    }
}

/// Every video container Powermove imports, native or converted.
pub fn is_video_extension(extension: &str) -> bool {
    NATIVE_VIDEO_EXTENSIONS.contains(&extension) || CONVERTED_VIDEO_EXTENSIONS.contains(&extension)
}

/// Stills Chromium cannot decode, converted to PNG on import.
pub fn needs_image_conversion(extension: &str) -> bool {
    CONVERTED_IMAGE_EXTENSIONS.contains(&extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(files: &[&'static str]) -> Result<Vec<&'static str>> {
        ordered_sequence(files.to_vec(), |name| name)
    }

    #[test]
    fn parses_the_last_number_before_the_extension() {
        let frame = sequence_frame("shot2_take0012.PNG").unwrap();
        assert_eq!(frame, SequenceFrame { prefix: "shot2_take".into(), index: 12, extension: "png".into() });
        assert!(sequence_frame("frame.png").is_none());
        assert!(sequence_frame("frame1.tif").is_none());
        assert!(sequence_frame("frame99999999999999999999.png").is_none());
    }

    #[test]
    fn orders_frames_numerically_with_mixed_padding() {
        assert_eq!(names(&["f10.png", "f9.png", "f0011.png", "f001.png"]).unwrap(), ["f001.png", "f9.png", "f10.png", "f0011.png"]);
    }

    #[test]
    fn rejects_mixed_names_duplicates_and_single_frames() {
        assert!(names(&["a1.png", "b2.png"]).unwrap_err().to_string().starts_with("Choose images with the same name"));
        assert!(names(&["a1.png", "a2.jpg"]).is_err());
        assert_eq!(names(&["a1.png", "a01.png"]).unwrap_err().to_string(), "Duplicate frame number 1");
        assert_eq!(names(&["a1.png"]).unwrap_err().to_string(), "Choose at least 2 numbered image frames");
    }

    #[test]
    fn classifies_extensions() {
        assert_eq!(media_extension(r"C:\clips\Take.MKV"), "mkv");
        assert_eq!(media_extension("noext"), "");
        assert!(is_video_extension("m2ts") && !is_video_extension("png"));
        assert!(needs_image_conversion("heic") && !needs_image_conversion("png"));
        assert!(valid_sequence_fps(1.0) && valid_sequence_fps(240.0) && !valid_sequence_fps(0.5) && !valid_sequence_fps(f64::NAN));
    }
}
