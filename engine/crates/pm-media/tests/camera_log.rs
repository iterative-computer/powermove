mod common;

use pm_media::interpret::{Interpretation, Origin, resolve_video};
use pm_media::probe::VideoProbe;
use pm_media::probe::color::electron_frame_reading;
use pm_protocol::color::{Primaries, Transfer};
use std::{fs, path::Path, process::Command};

fn video(path: &Path) -> VideoProbe {
    pm_media::probe::probe(path).unwrap().probe.video.unwrap()
}

fn movie(path: &Path, extra: &[&str]) {
    let result = Command::new(common::ffmpeg_bin())
        .args(["-v", "error", "-i"])
        .arg(common::fixtures_dir().join("prores422.mov"))
        .args(["-map", "0:v:0", "-c", "copy"])
        .args(extra)
        .arg(path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn atom(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut result = ((8 + data.len()) as u32).to_be_bytes().to_vec();
    result.extend_from_slice(kind);
    result.extend_from_slice(data);
    result
}

fn append_atom(data: &mut Vec<u8>, path: &[[u8; 4]], child: &[u8]) {
    let mut start = 0;
    let mut end = data.len();
    for kind in path {
        loop {
            assert!(start + 8 <= end, "missing {kind:?}");
            let len = u32::from_be_bytes(data[start..start + 4].try_into().unwrap()) as usize;
            assert!(len >= 8 && start + len <= end);
            if &data[start + 4..start + 8] == kind {
                end = start + len;
                data[start..start + 4].copy_from_slice(&((len + child.len()) as u32).to_be_bytes());
                start += match kind {
                    b"stsd" => 16,
                    b"apch" | b"apcn" => 86,
                    _ => 8,
                };
                break;
            }
            start += len;
        }
    }
    data.splice(end..end, child.iter().copied());
}

fn sony_metadata(gamma: &str, gamut: Option<&str>) -> Vec<u8> {
    let gamut = gamut.map_or_else(String::new, |gamut| {
        format!(r#"<Item name="CaptureColorPrimaries" value="{gamut}"/>"#)
    });
    let xml = format!(
        r#"<NonRealTimeMeta xmlns="urn:schemas-professionalDisc:nonRealTimeMeta:ver.2.20">
          <AcquisitionRecord><Group name="CameraUnitMetadataSet">
            <Item name="CaptureGammaEquation" value="{gamma}"/>
            {gamut}
          </Group></AcquisitionRecord>
        </NonRealTimeMeta>"#
    );
    let mut meta = vec![0; 4];
    let mut handler = vec![0; 8];
    handler.extend(b"nrtm");
    handler.extend([0; 13]);
    meta.extend(atom(b"hdlr", &handler));
    let mut xml_box = vec![0; 4];
    xml_box.extend(xml.bytes());
    meta.extend(atom(b"xml ", &xml_box));
    atom(b"meta", &meta)
}

#[test]
fn camera_log_import_changes_only_native_transfer_and_primaries() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mov");
    movie(&source, &[]);
    let baseline = video(&source);
    let before = baseline.color.interpreted;
    assert_eq!(before.transfer, Transfer::DisplayE);
    let bytes = fs::read(&source).unwrap();
    let first_sample = [
        *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"stsd", *b"apch",
    ];
    let apple = atom(b"logs", b"com.apple.rec2020.apple-log");
    for (name, path, tag, primaries, transfer, origin) in [
        (
            "apple",
            first_sample.as_slice(),
            apple.clone(),
            Primaries::Bt2020,
            Transfer::AppleLog,
            Some("QuickTime logs"),
        ),
        (
            "apple-log2",
            first_sample.as_slice(),
            atom(b"logs", b"com.apple.rec2020.apple-log2"),
            before.primaries,
            before.transfer,
            None,
        ),
        (
            "duplicate",
            first_sample.as_slice(),
            [apple.clone(), apple].concat(),
            before.primaries,
            before.transfer,
            None,
        ),
        (
            "sony-cine",
            &[*b"moov"],
            sony_metadata("s-log3-cine", None),
            Primaries::SGamut3Cine,
            Transfer::SLog3,
            Some("Sony XAVC NonRealTimeMeta"),
        ),
        (
            "sony-wide",
            &[],
            sony_metadata("s-log3", Some("s-gamut3")),
            Primaries::SGamut3,
            Transfer::SLog3,
            Some("Sony XAVC NonRealTimeMeta"),
        ),
        (
            "sony-sdr",
            &[*b"moov"],
            sony_metadata("rec709", Some("rec709")),
            before.primaries,
            before.transfer,
            None,
        ),
        (
            "sony-unknown-gamut",
            &[*b"moov"],
            sony_metadata("s-log3", Some("rec709")),
            before.primaries,
            before.transfer,
            None,
        ),
    ] {
        let mut modified = bytes.clone();
        append_atom(&mut modified, path, &tag);
        let fixture = dir.path().join(format!("{name}.mov"));
        fs::write(&fixture, modified).unwrap();
        let after = video(&fixture);
        assert_eq!(after.color.tagged, baseline.color.tagged, "{name}");
        let mut expected = before;
        (expected.primaries, expected.transfer) = (primaries, transfer);
        assert_eq!(after.color.interpreted, expected, "{name}");
        let automatic = resolve_video(&after, &Interpretation::default());
        assert_eq!(automatic.color, expected, "{name}");
        if let Some(source) = origin {
            assert_eq!(
                automatic.transfer.origin,
                Origin::File {
                    source: source.into()
                }
            );
        }
        let manual = resolve_video(
            &after,
            &Interpretation {
                transfer: Some(Transfer::Gamma22),
                ..Default::default()
            },
        );
        assert_eq!(manual.color.transfer, Transfer::Gamma22);
        assert_eq!(manual.color.primaries, primaries);
        assert_eq!(
            electron_frame_reading(&after.color, &after.codec.codec, false),
            electron_frame_reading(&baseline.color, &baseline.codec.codec, false),
            "{name}"
        );
    }
}

#[test]
fn panasonic_quicktime_distinguishes_v_log_from_other_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.mov");
    movie(&path, &[]);
    let before = video(&path).color.interpreted;
    for (gamma, gamut, is_log) in [
        ("V-Log", "V-Gamut", true),
        ("V-LogL", "V-Gamut", true),
        ("V-Log", "BT.709", false),
        ("Like709", "BT.709", false),
    ] {
        let meta = format!(
            r#"com.panasonic.Semi-Pro.metadata.xml=<ClipMain xmlns="urn:schemas-Professional-Plug-in:Semi-Pro:ClipMetadata:v1.0">
              <UserArea><AcquisitionMetadata xmlns="urn:schemas-Professional-Plug-in:P2:CameraMetadata:v1.2">
                <CameraUnitMetadata>
                  <Gamma><CaptureGamma>{gamma}</CaptureGamma></Gamma>
                  <Gamut><CaptureGamut>{gamut}</CaptureGamut></Gamut>
                </CameraUnitMetadata>
              </AcquisitionMetadata></UserArea>
            </ClipMain>"#
        );
        let path = dir.path().join(format!("{gamma}-{gamut}.mov"));
        movie(
            &path,
            &["-movflags", "+use_metadata_tags", "-metadata", &meta],
        );
        let mut expected = before;
        if is_log {
            (expected.primaries, expected.transfer) = (Primaries::VGamut, Transfer::VLog);
        }
        assert_eq!(video(&path).color.interpreted, expected, "{gamma}/{gamut}");
    }
}

#[test]
fn arri_quicktime_distinguishes_log_c_from_a_baked_rec709_look() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mov");
    movie(&source, &[]);
    let before = video(&source).color.interpreted;
    for (gamma, primaries, transfer) in [
        ("LOG-C", Primaries::AwgGen3, Transfer::LogC3),
        ("REC-709", before.primaries, before.transfer),
    ] {
        let path = dir.path().join(format!("{gamma}.mov"));
        movie(
            &path,
            &[
                "-movflags",
                "+use_metadata_tags",
                "-metadata",
                &format!("com.arri.camera.ColorGammaSxS={gamma}"),
            ],
        );
        let color = video(&path).color.interpreted;
        assert_eq!((color.primaries, color.transfer), (primaries, transfer));
    }
}

#[test]
fn mxf_camera_log_uses_the_picture_descriptor() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mxf");
    let result = Command::new(common::ffmpeg_bin())
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=720x576:r=25",
        ])
        .args([
            "-frames:v",
            "2",
            "-c:v",
            "mpeg2video",
            "-pix_fmt",
            "yuv422p",
        ])
        .args([
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let before = video(&source).color.interpreted;
    assert_eq!(before.transfer, Transfer::DisplayE);
    let mxf = fs::read(source).unwrap();
    let tag = b"\x32\x10\x00\x10\x06\x0e\x2b\x34\x04\x01\x01\x01\x04\x01\x01\x01\x01\x02\x00\x00";
    let fields = mxf
        .windows(tag.len())
        .enumerate()
        .filter_map(|(i, bytes)| (bytes == tag).then_some(i))
        .collect::<Vec<_>>();
    assert!(
        !fields.is_empty(),
        "fixture is missing its BT.709 transfer label"
    );
    for (name, ul, primaries, transfer) in [
        (
            "log-c3",
            [0x0e, 0x17, 0x04, 0x01, 0x03, 0x01, 0x02, 0x01],
            Primaries::AwgGen3,
            Transfer::LogC3,
        ),
        (
            "log-c4",
            [0x0e, 0x17, 0x01, 0x02, 0x04, 0x02, 0, 0],
            Primaries::AwgGen4,
            Transfer::LogC4,
        ),
        (
            "v-log",
            [0x04, 0x01, 0x01, 0x01, 0x01, 0x0f, 0, 0],
            Primaries::VGamut,
            Transfer::VLog,
        ),
    ] {
        let mut modified = mxf.clone();
        for &i in &fields {
            modified[i + 11] = 0x0d;
            modified[i + 12..i + 20].copy_from_slice(&ul);
        }
        let path = dir.path().join(format!("{name}.mxf"));
        fs::write(&path, modified).unwrap();
        let after = video(&path).color.interpreted;
        let mut expected = before;
        (expected.primaries, expected.transfer) = (primaries, transfer);
        assert_eq!(after, expected, "{name}");
    }
}
