//! Image sequences (section 13 "Formats at cutover": any image format, with
//! gaps reported): numbered files found from one member.

use crate::error::{MediaError, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSequence {
    pub dir: PathBuf,
    pub prefix: String,
    pub suffix: String,
    /// Digits of zero-padded numbers; 0 when numbers are not padded.
    pub padding: usize,
    /// Frame numbers and files, ascending.
    pub frames: Vec<(i64, PathBuf)>,
    /// Missing numbers between the first and last frame, as inclusive ranges.
    pub gaps: Vec<(i64, i64)>,
}

impl ImageSequence {
    /// Explicit files selected by the host, matching today's importer:
    /// padding may vary, gaps are reported, duplicate numbers are errors.
    /// Unlike directory discovery this never adds an unselected file.
    pub fn from_paths(paths: &[PathBuf]) -> Result<Self> {
        if paths.len() < 2 {
            return Err(MediaError::invalid(
                "choose at least 2 numbered image frames",
            ));
        }
        let mut frames = Vec::with_capacity(paths.len());
        let (mut prefix, mut suffix) = (String::new(), String::new());
        for (i, path) in paths.iter().enumerate() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| MediaError::invalid("sequence filename is not UTF-8"))?;
            let (p, digits, s) = split(name)
                .filter(|(_, _, s)| s.starts_with('.') && !s[1..].contains('.'))
                .ok_or_else(|| MediaError::invalid("sequence images must end in frame numbers"))?;
            let number = digits
                .parse::<i64>()
                .ok()
                .filter(|v| *v <= 9_007_199_254_740_991)
                .ok_or_else(|| {
                    MediaError::invalid("sequence frame number is not a safe integer")
                })?;
            let s = s.to_ascii_lowercase();
            if i == 0 {
                prefix = p.into();
                suffix = s;
            } else if p != prefix || s != suffix {
                return Err(MediaError::invalid(
                    "sequence images must share their name and extension",
                ));
            }
            frames.push((number, path.clone()));
        }
        frames.sort_by_key(|f| f.0);
        let mut gaps = Vec::new();
        for pair in frames.windows(2) {
            let (a, b) = (pair[0].0, pair[1].0);
            if a == b {
                return Err(MediaError::invalid(format!("duplicate frame number {a}")));
            }
            if b > a + 1 {
                gaps.push((a + 1, b - 1));
            }
        }
        Ok(Self {
            dir: paths[0].parent().unwrap_or(Path::new(".")).into(),
            prefix,
            suffix,
            padding: 0,
            frames,
            gaps,
        })
    }

    /// Compat level 1 plays the available files consecutively, skipping
    /// missing numbers. This differs from `file_for`'s numbered hold mode.
    pub fn available_file(&self, index: usize) -> Option<&Path> {
        self.frames.get(index).map(|f| f.1.as_path())
    }

    pub fn first(&self) -> i64 {
        self.frames[0].0
    }
    pub fn last(&self) -> i64 {
        self.frames[self.frames.len() - 1].0
    }
    /// The file showing for frame number `n`: the frame itself, or the last
    /// frame before a gap (gaps hold the previous image).
    pub fn file_for(&self, n: i64) -> Option<&Path> {
        let i = self.frames.partition_point(|f| f.0 <= n).checked_sub(1)?;
        Some(&self.frames[i].1)
    }
}

/// Splits a file name into prefix, digits, suffix around the last run of
/// digits before the extension.
fn split(name: &str) -> Option<(&str, &str, &str)> {
    let ext_at = name.rfind('.').unwrap_or(name.len());
    let stem = &name[..ext_at];
    let end = stem
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_ascii_digit())
        .map(|(i, _)| i + 1)?;
    let start = stem[..end]
        .char_indices()
        .rev()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    Some((&name[..start], &name[start..end], &name[end..]))
}

/// The sequence `path` belongs to, or None when it has no frame number or
/// is the only member.
pub fn detect(path: &Path) -> Result<Option<ImageSequence>> {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Ok(None);
    };
    let Some((prefix, digits, suffix)) = split(name) else {
        return Ok(None);
    };
    let padded = digits.len() > 1 && digits.starts_with('0');
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut frames: Vec<(i64, PathBuf)> = Vec::new();
    for e in std::fs::read_dir(&dir)? {
        let e = e?;
        let n = e.file_name();
        let Some(n) = n.to_str() else { continue };
        let Some((p, d, s)) = split(n) else { continue };
        if p != prefix || s != suffix {
            continue;
        }
        let ok = if padded {
            d.len() == digits.len()
        } else {
            d.len() == 1 || !d.starts_with('0') || d.len() == digits.len()
        };
        if !ok {
            continue;
        }
        if let Ok(v) = d.parse::<i64>() {
            frames.push((v, e.path()));
        }
    }
    if frames.len() < 2 {
        return Ok(None);
    }
    frames.sort();
    frames.dedup_by_key(|f| f.0);
    let mut gaps = Vec::new();
    for w in frames.windows(2) {
        if w[1].0 > w[0].0 + 1 {
            gaps.push((w[0].0 + 1, w[1].0 - 1));
        }
    }
    Ok(Some(ImageSequence {
        dir,
        prefix: prefix.to_string(),
        suffix: suffix.to_string(),
        padding: if padded { digits.len() } else { 0 },
        frames,
        gaps,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_the_last_number() {
        assert_eq!(
            split("shot_v2.0001.exr"),
            Some(("shot_v2.", "0001", ".exr"))
        );
        assert_eq!(split("frame12.png"), Some(("frame", "12", ".png")));
        assert_eq!(split("still.png"), None);
    }

    #[test]
    fn finds_frames_and_gaps() {
        let dir = tempfile::Builder::new()
            .prefix("pm-media-seq-")
            .tempdir()
            .unwrap();
        for n in [1, 2, 3, 6, 7, 10] {
            std::fs::write(dir.path().join(format!("shot.{n:04}.png")), b"x").unwrap();
        }
        std::fs::write(dir.path().join("shot.12.png"), b"x").unwrap(); // other padding
        std::fs::write(dir.path().join("other.0004.png"), b"x").unwrap();
        let s = detect(&dir.path().join("shot.0003.png")).unwrap().unwrap();
        assert_eq!(
            s.frames.iter().map(|f| f.0).collect::<Vec<_>>(),
            vec![1, 2, 3, 6, 7, 10]
        );
        assert_eq!(s.gaps, vec![(4, 5), (8, 9)]);
        assert_eq!(s.padding, 4);
        assert_eq!(s.file_for(5).unwrap().file_name().unwrap(), "shot.0003.png");
        assert_eq!(s.file_for(0), None);
    }
}
