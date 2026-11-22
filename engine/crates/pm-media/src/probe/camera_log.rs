//! Explicit camera log metadata; camera make/model and generic CICP tags are
//! not enough to tell a log recording from SDR made with the same camera.

use crate::index::CodecConfig;
use pm_protocol::color::{Primaries, Transfer};

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub(crate) struct CameraLog {
    pub primaries: Primaries,
    pub transfer: Transfer,
    pub source: &'static str,
}

pub(crate) fn quicktime(config: &CodecConfig) -> Option<CameraLog> {
    let mut logs = config.atoms.iter().filter(|(kind, _)| kind == b"logs");
    let (_, value) = logs.next()?;
    // CoreMedia's LogTransferFunction extension, stored as a UTF-8 identifier
    // in the video sample description. Apple Log 2 is a different transform.
    (value == b"com.apple.rec2020.apple-log" && logs.next().is_none()).then_some(CameraLog {
        primaries: Primaries::Bt2020,
        transfer: Transfer::AppleLog,
        source: "QuickTime logs",
    })
}

pub(crate) fn arri_quicktime(metadata: &[(String, String)]) -> Option<CameraLog> {
    // ARRI Metadata White Paper, ICI16 (Target Color Space).
    let gamma = metadata_value(metadata, "com.arri.camera.ColorGammaSxS")?;
    gamma
        .trim()
        .eq_ignore_ascii_case("LOG-C")
        .then_some(log_c3("ARRI QuickTime ColorGammaSxS"))
}

fn metadata_value<'a>(metadata: &'a [(String, String)], key: &str) -> Option<&'a str> {
    only(
        metadata
            .iter()
            .filter(|(tag, _)| tag.eq_ignore_ascii_case(key)),
    )
    .map(|(_, value)| value.as_str())
}

pub(crate) const MAX_XML_BYTES: usize = 1024 * 1024;

fn parse_xml(xml: &str) -> Option<roxmltree::Document<'_>> {
    if xml.len() > MAX_XML_BYTES {
        return None;
    }
    roxmltree::Document::parse_with_options(
        xml.trim_end_matches('\0'),
        roxmltree::ParsingOptions {
            nodes_limit: 10_000,
            ..Default::default()
        },
    )
    .ok()
}

fn named_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    ns: &'a str,
    name: &'static str,
) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(move |child| child.has_tag_name((ns, name)))
}

pub(crate) fn sony_nrt_xml(xml: &[u8]) -> Option<Primaries> {
    let doc = parse_xml(std::str::from_utf8(xml).ok()?)?;
    let root = doc.root_element();
    let ns = root.tag_name().namespace()?;
    if root.tag_name().name() != "NonRealTimeMeta"
        || !ns.starts_with("urn:schemas-professionalDisc:nonRealTimeMeta:ver.")
    {
        return None;
    }
    let mut records = named_children(root, ns, "AcquisitionRecord");
    let mut groups = named_children(records.next()?, ns, "Group")
        .filter(|node| node.attribute("name") == Some("CameraUnitMetadataSet"));
    let group = groups.next()?;
    if records.next().is_some() || groups.next().is_some() {
        return None;
    }
    let items = |name| {
        named_children(group, ns, "Item").filter(move |node| node.attribute("name") == Some(name))
    };
    let gamma = only(items("CaptureGammaEquation"))?.attribute("value")?;
    let mut gamuts = items("CaptureColorPrimaries");
    let gamut = match gamuts.next() {
        Some(item) => item.attribute("value")?,
        // Sony also records this pair as one CaptureGammaEquation value.
        None if gamma == "s-log3-cine" => "s-gamut3-cine",
        None => return None,
    };
    if gamuts.next().is_some() {
        return None;
    }
    match (gamma, gamut) {
        ("s-log3" | "s-log3-cine", "s-gamut3-cine") => Some(Primaries::SGamut3Cine),
        ("s-log3", "s-gamut3") => Some(Primaries::SGamut3),
        _ => None,
    }
}

pub(crate) fn panasonic_quicktime(metadata: &[(String, String)]) -> Option<CameraLog> {
    let xml = metadata_value(metadata, "com.panasonic.Semi-Pro.metadata.xml")?;
    let doc = parse_xml(xml)?;
    let root = doc.root_element();
    let clip_ns = root.tag_name().namespace()?;
    if root.tag_name().name() != "ClipMain"
        || !clip_ns.starts_with("urn:schemas-Professional-Plug-in:Semi-Pro:ClipMetadata:v")
    {
        return None;
    }
    let area = only(named_children(root, clip_ns, "UserArea"))?;
    let acquisition = only(area.children().filter(|node| {
        node.tag_name().name() == "AcquisitionMetadata"
            && node.tag_name().namespace().is_some_and(|ns| {
                ns.starts_with("urn:schemas-Professional-Plug-in:P2:CameraMetadata:v")
            })
    }))?;
    let ns = acquisition.tag_name().namespace()?;
    let camera = only(named_children(acquisition, ns, "CameraUnitMetadata"))?;
    let field = |group, name| {
        let node = only(named_children(camera, ns, group))?;
        only(named_children(node, ns, name))?.text().map(str::trim)
    };
    (matches!(field("Gamma", "CaptureGamma")?, "V-Log" | "V-LogL")
        && field("Gamut", "CaptureGamut")? == "V-Gamut")
        .then_some(CameraLog {
            source: "Panasonic Semi-Pro metadata",
            ..v_log()
        })
}

fn only<T>(mut items: impl Iterator<Item = T>) -> Option<T> {
    let item = items.next()?;
    items.next().is_none().then_some(item)
}

pub(crate) fn sony(primaries: Primaries) -> CameraLog {
    CameraLog {
        primaries,
        transfer: Transfer::SLog3,
        source: "Sony XAVC NonRealTimeMeta",
    }
}

fn log_c3(source: &'static str) -> CameraLog {
    CameraLog {
        primaries: Primaries::AwgGen3,
        transfer: Transfer::LogC3,
        source,
    }
}

pub(crate) fn v_log() -> CameraLog {
    CameraLog {
        primaries: Primaries::VGamut,
        transfer: Transfer::VLog,
        source: "SMPTE ST 2115 Camera Log V",
    }
}

pub(crate) fn mxf(transfer_ul: [u8; 16]) -> Option<CameraLog> {
    // The registry version byte does not identify the label.
    if transfer_ul[..7] != [0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01] {
        return None;
    }
    match transfer_ul[8..] {
        // ARRI MXF/ProRes Metadata v3, table 2; SMPTE RDD 55, table 18.
        [0x0e, 0x17, 0x04, 0x01, 0x03, 0x01, 0x02, 0x01]
        | [0x0e, 0x17, 0x01, 0x02, 0x04, 0x01, 0, 0] => Some(log_c3("ARRI MXF picture descriptor")),
        [0x04, 0x01, 0x01, 0x01, 0x01, 0x11, 0, 0] => Some(log_c3("SMPTE ST 2115 Camera Log C3")),
        [0x0e, 0x17, 0x01, 0x02, 0x04, 0x02, 0, 0] => Some(CameraLog {
            primaries: Primaries::AwgGen4,
            transfer: Transfer::LogC4,
            source: "ARRI MXF picture descriptor",
        }),
        [0x04, 0x01, 0x01, 0x01, 0x01, 0x0f, 0, 0] => Some(v_log()),
        _ => None,
    }
}
