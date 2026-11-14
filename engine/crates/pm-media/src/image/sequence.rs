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

/// A sequence read in place: the folder, the numbering pattern and the
/// first and last number. It is a small file an asset retains as its
/// original; frame `n` of the clip is file number `first + n`, read as a
/// still at its own depth. A number with no file is a missing frame, never
/// another one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Format version (first, so the bytes start with [`MANIFEST_MAGIC`]).
    pub powermove_image_sequence: u32,
    pub dir: PathBuf,
    pub prefix: String,
    pub suffix: String,
    /// Digits numbers are zero-padded to; 0 when they are not padded.
    pub padding: usize,
    pub first: i64,
    pub last: i64,
    /// Frames per second, `[num, den]` (24000/1001 for 23.976): the rate
    /// the sequence was imported at, which a proxy of it is made at.
    pub rate: [u32; 2],
}

/// The frame (slot, from 0) a numbered sequence shows at `seconds` of its
/// source at `fps`: the shipped `sequencePlaybackTime` rule, ⌊(t + 10⁻⁷) ·
/// fps⌋, held at its ends. A sequence's frames lie on an exact grid, with
/// no container timestamps to absorb: one rule for its files, its proxy and
/// an earlier build's transcoded rendition, with or without Interpret
/// Footage's rate.
pub fn sequence_slot(seconds: f64, fps: f64, frames: u64) -> u64 {
    let last = frames.saturating_sub(1) as f64;
    let slot = ((seconds + 1e-7) * fps).floor();
    if slot.is_nan() {
        0
    } else {
        slot.clamp(0.0, last) as u64
    }
}

/// The most numbers a sequence's range may span, gaps included (over eleven
/// hours at 24 fps): every number in it is a frame of the clip, and a
/// numbering far sparser than that is not one sequence.
pub const MAX_FRAMES: u64 = 1_000_000;

/// How a manifest's bytes begin.
pub const MANIFEST_MAGIC: &[u8] = b"{\"powermoveImageSequence\":";

impl Manifest {
    /// The pattern of explicitly chosen frames. Every chosen file must be
    /// the one its number names under one padding, so that the pattern
    /// finds exactly them (and frames rendered into a gap later).
    pub fn of(sequence: &ImageSequence, rate: pm_time::Rate) -> Result<Self> {
        let rate = [rate.num(), rate.den()].map(|v| u32::try_from(v).unwrap_or(0));
        if rate.contains(&0) {
            return Err(MediaError::invalid(
                "image sequence: frame rate out of range",
            ));
        }
        let names: Vec<(&str, &str, &str)> = sequence
            .frames
            .iter()
            .map(|(_, path)| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(split)
                    .ok_or_else(|| MediaError::invalid("sequence images must end in frame numbers"))
            })
            .collect::<Result<_>>()?;
        let (prefix, _, suffix) = names[0];
        let padded = names
            .iter()
            .any(|(_, d, _)| d.len() > 1 && d.starts_with('0'));
        let padding = if padded {
            names.iter().map(|(_, d, _)| d.len()).min().unwrap_or(0)
        } else {
            0
        };
        if sequence.last().saturating_sub(sequence.first()) >= MAX_FRAMES as i64 {
            return Err(MediaError::invalid(format!(
                "image sequence: frames numbered {} to {} span more than {MAX_FRAMES} numbers",
                sequence.first(),
                sequence.last()
            )));
        }
        let manifest = Manifest {
            powermove_image_sequence: 1,
            dir: sequence.dir.clone(),
            prefix: prefix.into(),
            suffix: suffix.into(),
            padding,
            first: sequence.first(),
            last: sequence.last(),
            rate,
        };
        for ((number, path), (p, _, s)) in sequence.frames.iter().zip(&names) {
            if *p != prefix || *s != suffix || manifest.file(*number) != *path {
                return Err(MediaError::invalid(
                    "sequence images must be numbered the same way (for example frame_0001.png)",
                ));
            }
        }
        Ok(manifest)
    }

    /// The file of frame number `number`.
    pub fn file(&self, number: i64) -> PathBuf {
        let digits = format!("{:0width$}", number, width = self.padding);
        self.dir
            .join(format!("{}{}{}", self.prefix, digits, self.suffix))
    }

    /// Frames from the first number to the last, gaps included.
    pub fn frames(&self) -> u64 {
        self.last
            .saturating_sub(self.first)
            .saturating_add(1)
            .max(0) as u64
    }

    /// The numbers of the range that have a file now, ascending: one
    /// listing of the folder, never one look per number, so a sparse
    /// numbering costs what its files cost. `None` when `cancel` is set
    /// while it lists.
    pub fn present_numbers(
        &self,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> std::io::Result<Option<Vec<i64>>> {
        let mut numbers = Vec::new();
        for (i, entry) in std::fs::read_dir(&self.dir)?.enumerate() {
            if i % 1024 == 0 && cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(None);
            }
            let entry = entry?;
            let name = entry.file_name();
            let Some((prefix, digits, suffix)) = name.to_str().and_then(split) else {
                continue;
            };
            if prefix != self.prefix || suffix != self.suffix {
                continue;
            }
            let Ok(number) = digits.parse::<i64>() else {
                continue;
            };
            // The name its number has under this numbering, and a file.
            if (self.first..=self.last).contains(&number)
                && self.file(number).file_name() == Some(name.as_os_str())
                && entry.file_type().is_ok_and(|t| !t.is_dir())
            {
                numbers.push(number);
            }
        }
        numbers.sort_unstable();
        Ok(Some(numbers))
    }

    /// Whether any file of the range is where the manifest says: a folder
    /// moved or renamed leaves none, and the sequence is offline until it is
    /// relinked (a single missing number is that frame's Media Offline).
    pub fn present(&self) -> bool {
        let never = std::sync::atomic::AtomicBool::new(false);
        self.present_numbers(&never)
            .ok()
            .flatten()
            .is_some_and(|numbers| !numbers.is_empty())
    }

    /// Numbers in the range with no file now, as inclusive ranges (from
    /// [`Manifest::present_numbers`]). `None` when cancelled.
    pub fn missing_until(&self, cancel: &std::sync::atomic::AtomicBool) -> Option<Vec<(i64, i64)>> {
        let present = self.present_numbers(cancel).ok()?;
        let present = present?;
        let mut out = Vec::new();
        let mut next = self.first;
        for n in present {
            if n > next {
                out.push((next, n - 1));
            }
            next = n + 1;
        }
        if next <= self.last {
            out.push((next, self.last));
        }
        Some(out)
    }

    /// Numbers in the range with no file now, as inclusive ranges.
    pub fn missing(&self) -> Vec<(i64, i64)> {
        self.missing_until(&std::sync::atomic::AtomicBool::new(false))
            .unwrap_or_else(|| vec![(self.first, self.last)])
    }

    /// How the sequence is named in the Project panel:
    /// `shot.[0001-0048].png`.
    pub fn name(&self) -> String {
        let n = |v: i64| format!("{:0width$}", v, width = self.padding);
        format!(
            "{}[{}-{}]{}",
            self.prefix,
            n(self.first),
            n(self.last),
            self.suffix
        )
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a manifest serializes")
    }

    /// The number showing at `seconds` of its source at `fps` (its own rate,
    /// or Interpret Footage's): number `first` + [`sequence_slot`].
    pub fn number_at(&self, seconds: f64, fps: f64) -> i64 {
        self.first + sequence_slot(seconds, fps, self.frames()) as i64
    }

    /// The rate it was imported at.
    pub fn rate(&self) -> Option<pm_time::Rate> {
        pm_time::Rate::new(i64::from(self.rate[0]), i64::from(self.rate[1])).ok()
    }

    /// The manifest `path` holds, or `None` when the file is not one.
    pub fn read(path: &Path) -> Result<Option<Self>> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let mut head = [0u8; MANIFEST_MAGIC.len()];
        if file.read_exact(&mut head).is_err() || head != MANIFEST_MAGIC {
            return Ok(None);
        }
        let mut bytes = head.to_vec();
        file.take(64 << 10).read_to_end(&mut bytes)?;
        let manifest: Manifest = serde_json::from_slice(&bytes)
            .map_err(|e| MediaError::invalid(format!("image sequence: {e}")))?;
        if manifest.powermove_image_sequence != 1
            || manifest.last < manifest.first
            || manifest.frames() > MAX_FRAMES
            || manifest.rate.contains(&0)
        {
            return Err(MediaError::invalid("image sequence: unknown format"));
        }
        Ok(Some(manifest))
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

    #[test]
    fn a_manifest_names_every_number_by_one_pattern() {
        let dir = tempfile::Builder::new()
            .prefix("pm-media-seq-")
            .tempdir()
            .unwrap();
        let paths: Vec<_> = [9, 10, 12]
            .iter()
            .map(|n| {
                let path = dir.path().join(format!("Shot_v2.{n:03}.EXR"));
                std::fs::write(&path, b"x").unwrap();
                path
            })
            .collect();
        let rate = pm_time::Rate::FPS_23_976;
        let m = Manifest::of(&ImageSequence::from_paths(&paths).unwrap(), rate).unwrap();
        assert_eq!(m.rate, [24000, 1001]);
        assert_eq!((m.padding, m.first, m.last, m.frames()), (3, 9, 12, 4));
        assert_eq!(m.file(11), dir.path().join("Shot_v2.011.EXR"));
        assert_eq!(m.name(), "Shot_v2.[009-012].EXR");
        assert_eq!(m.missing(), vec![(11, 11)]);
        let file = dir.path().join("sequence.json");
        std::fs::write(&file, m.to_bytes()).unwrap();
        assert_eq!(Manifest::read(&file).unwrap(), Some(m));
        assert_eq!(Manifest::read(&paths[0]).unwrap(), None);
        // Mixed padding has no one pattern: never guess a file's name.
        let mixed: Vec<_> = ["f_1.png", "f_02.png"]
            .iter()
            .map(|n| {
                let path = dir.path().join(n);
                std::fs::write(&path, b"x").unwrap();
                path
            })
            .collect();
        assert!(Manifest::of(&ImageSequence::from_paths(&mixed).unwrap(), rate).is_err());
    }

    /// A sparse numbering (two files a billion apart) is not one sequence:
    /// refused at once, with no look at the numbers between; within the
    /// bound, what is missing comes from one listing of the folder.
    #[test]
    fn a_sparse_numbering_is_refused_without_walking_its_range() {
        let dir = tempfile::tempdir().unwrap();
        let far = [1_i64, 1_000_000_001].map(|n| dir.path().join(format!("shot.{n:010}.png")));
        for path in &far {
            std::fs::write(path, b"x").unwrap();
        }
        let started = std::time::Instant::now();
        let sequence = ImageSequence::from_paths(&far).unwrap();
        let refused = Manifest::of(&sequence, pm_time::Rate::FPS_24).unwrap_err();
        assert!(
            refused.to_string().contains("more than 1000000 numbers"),
            "{refused}"
        );
        // Just inside the bound: two files, a million numbers.
        let near = [1_i64, 1_000_000].map(|n| dir.path().join(format!("shot.{n:010}.png")));
        std::fs::write(&near[1], b"x").unwrap();
        let m = Manifest::of(
            &ImageSequence::from_paths(&near).unwrap(),
            pm_time::Rate::FPS_24,
        )
        .unwrap();
        assert_eq!(m.missing(), vec![(2, 999_999)]);
        assert!(m.present());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        assert_eq!(m.missing_until(&cancelled), None);
        // An untrusted manifest spanning more is not read.
        let wide = Manifest {
            last: 5_000_000,
            ..m
        };
        let path = dir.path().join("manifest");
        std::fs::write(&path, wide.to_bytes()).unwrap();
        assert!(Manifest::read(&path).is_err());
    }
}
