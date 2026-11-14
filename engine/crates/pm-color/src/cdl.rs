//! ASC CDL files (`.cc`, `.cdl`, `.ccc`): one colour correction's slope,
//! offset and power per channel and its saturation (ASC CDL v1.2). Read from
//! the XML any on-set or grading tool writes, written as v1.2 `.cc` or `.cdl`.
//!
//! The correction is `out = (in × slope + offset) ^ power`, then saturation
//! with the Rec. 709 luma weights. Values are kept as written; the reader
//! refuses what the specification does not allow (a negative slope or
//! saturation, a power that is not positive) with a sentence.

/// One colour correction.
#[derive(Clone, Debug, PartialEq)]
pub struct Cdl {
    /// `id` of the correction, when the file names one.
    pub id: Option<String>,
    pub slope: [f64; 3],
    pub offset: [f64; 3],
    pub power: [f64; 3],
    pub saturation: f64,
}

impl Default for Cdl {
    fn default() -> Self {
        Self {
            id: None,
            slope: [1.; 3],
            offset: [0.; 3],
            power: [1.; 3],
            saturation: 1.,
        }
    }
}

/// The file layouts the writer knows, by extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// `.cc`: a lone `ColorCorrection`.
    Correction,
    /// `.cdl`: a `ColorDecisionList` with one `ColorDecision`.
    DecisionList,
}

impl Layout {
    /// From a file name's extension: `.cdl` is a decision list, anything
    /// else a lone correction.
    pub fn of(name: &str) -> Layout {
        if name.to_ascii_lowercase().ends_with(".cdl") {
            Layout::DecisionList
        } else {
            Layout::Correction
        }
    }
}

/// The text between the first `<name …>` and its `</name>` at or after
/// `from`, the element's name with or without a namespace prefix, and where
/// the element ends. `None` when there is no such element.
fn element<'a>(text: &'a str, name: &str, from: usize) -> Option<(&'a str, &'a str, usize)> {
    let mut at = from;
    while let Some(open) = text[at..].find('<').map(|i| at + i) {
        let rest = &text[open + 1..];
        let end = rest.find(|c: char| c == '>' || c.is_whitespace() || c == '/')?;
        let tag = &rest[..end];
        let local = tag.rsplit(':').next().unwrap_or(tag);
        if local == name && !tag.starts_with('/') {
            let head_end = open + 1 + rest.find('>')?;
            let head = &text[open + 1..head_end];
            if head.ends_with('/') {
                return Some((head, "", head_end + 1));
            }
            let close = format!("</{tag}>");
            let body_end = head_end + 1 + text[head_end + 1..].find(&close)?;
            return Some((head, &text[head_end + 1..body_end], body_end + close.len()));
        }
        at = open + 1;
    }
    None
}

/// An attribute's value in an element's head (`id="…"`).
fn attribute(head: &str, name: &str) -> Option<String> {
    let at = head.find(&format!("{name}="))? + name.len() + 1;
    let quote = head[at..].chars().next()?;
    let rest = &head[at + 1..];
    let value = &rest[..rest.find(quote)?];
    Some(
        value
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

/// Three numbers of `node` inside `body`, or `default` when it is absent.
fn triple(body: &str, node: &str, default: [f64; 3]) -> Result<[f64; 3], String> {
    let Some((_, text, _)) = element(body, node, 0) else {
        return Ok(default);
    };
    let values: Vec<f64> = text
        .split_whitespace()
        .map(|v| v.parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect::<Option<_>>()
        .ok_or_else(|| format!("The CDL’s {node} isn’t three numbers: “{}”.", text.trim()))?;
    <[f64; 3]>::try_from(values)
        .map_err(|_| format!("The CDL’s {node} isn’t three numbers: “{}”.", text.trim()))
}

/// Reads the one colour correction of a `.cc`, `.cdl` or `.ccc` file. A file
/// holding several is refused: which one applies is the person's choice.
pub fn parse(text: &str) -> Result<Cdl, String> {
    let mut corrections = Vec::new();
    let mut at = 0;
    while let Some((head, body, end)) = element(text, "ColorCorrection", at) {
        corrections.push((head, body));
        at = end;
    }
    let (head, body) = match corrections.as_slice() {
        [] => return Err("This file has no ASC CDL color correction.".into()),
        [one] => *one,
        many => {
            return Err(format!(
                "This file holds {} color corrections; choose a file with one.",
                many.len()
            ));
        }
    };
    let sop = element(body, "SOPNode", 0).map_or("", |(_, b, _)| b);
    // v1.2 writes SatNode, v1.01 SATNode.
    let saturation = match element(body, "SatNode", 0)
        .or_else(|| element(body, "SATNode", 0))
        .and_then(|(_, b, _)| element(b, "Saturation", 0))
    {
        Some((_, text, _)) => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("The CDL’s Saturation isn’t a number: “{}”.", text.trim()))?,
        None => 1.,
    };
    let cdl = Cdl {
        id: attribute(head, "id").filter(|id| !id.is_empty()),
        slope: triple(sop, "Slope", [1.; 3])?,
        offset: triple(sop, "Offset", [0.; 3])?,
        power: triple(sop, "Power", [1.; 3])?,
        saturation,
    };
    if cdl.slope.iter().any(|s| *s < 0.) {
        return Err("The CDL’s Slope is negative; a slope is 0 or more.".into());
    }
    if cdl.power.iter().any(|p| *p <= 0.) {
        return Err("The CDL’s Power isn’t positive; a power is more than 0.".into());
    }
    if cdl.saturation < 0. {
        return Err("The CDL’s Saturation is negative; it is 0 or more.".into());
    }
    Ok(cdl)
}

fn numbers(v: [f64; 3]) -> String {
    format!("{} {} {}", number(v[0]), number(v[1]), number(v[2]))
}

/// Six decimals, as grading tools write a CDL (a negative zero as zero).
fn number(v: f64) -> String {
    let text = format!("{v:.6}");
    if text == "-0.000000" {
        "0.000000".into()
    } else {
        text
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The ASC CDL v1.2 file of `cdl`, in `layout`.
pub fn write(cdl: &Cdl, layout: Layout) -> String {
    let id = cdl
        .id
        .as_deref()
        .map(|id| format!(" id=\"{}\"", escape(id)))
        .unwrap_or_default();
    let correction = |indent: &str| {
        format!(
            "{indent}<ColorCorrection{id}>\n\
             {indent}  <SOPNode>\n\
             {indent}    <Slope>{}</Slope>\n\
             {indent}    <Offset>{}</Offset>\n\
             {indent}    <Power>{}</Power>\n\
             {indent}  </SOPNode>\n\
             {indent}  <SatNode>\n\
             {indent}    <Saturation>{}</Saturation>\n\
             {indent}  </SatNode>\n\
             {indent}</ColorCorrection>\n",
            numbers(cdl.slope),
            numbers(cdl.offset),
            numbers(cdl.power),
            number(cdl.saturation),
        )
    };
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
    match layout {
        Layout::Correction => {
            let body = correction("");
            format!(
                "{xml}{}",
                body.replacen(
                    "<ColorCorrection",
                    "<ColorCorrection xmlns=\"urn:ASC:CDL:v1.2\"",
                    1
                )
            )
        }
        Layout::DecisionList => format!(
            "{xml}<ColorDecisionList xmlns=\"urn:ASC:CDL:v1.2\">\n  <ColorDecision>\n{}  </ColorDecision>\n</ColorDecisionList>\n",
            correction("    ")
        ),
    }
}

/// `cdl` applied to an R′G′B′ value, no clamp (OpenColorIO's no-clamp
/// style: a negative value passes the power unchanged).
pub fn apply(cdl: &Cdl, rgb: [f64; 3]) -> [f64; 3] {
    let sop: [f64; 3] = std::array::from_fn(|c| {
        let v = rgb[c] * cdl.slope[c] + cdl.offset[c];
        if v > 0. { v.powf(cdl.power[c]) } else { v }
    });
    let luma = 0.2126 * sop[0] + 0.7152 * sop[1] + 0.0722 * sop[2];
    sop.map(|v| luma + (v - luma) * cdl.saturation)
}

/// The Color Wheels parameters (effect `colorWheels`) that grade exactly as
/// `cdl`: gain = slope, offset = offset, gamma = −log₂ power, lift 0 and the
/// master controls neutral, saturation in percent. Refused, in a sentence,
/// when a value is outside what the wheels hold.
pub fn to_wheels(cdl: &Cdl) -> Result<Vec<(String, f64)>, String> {
    let held = |name: &str, v: f64, lo: f64, hi: f64| {
        if (lo..=hi).contains(&v) {
            Ok(v)
        } else {
            Err(format!(
                "The CDL’s {name} {v} is outside what Color Wheels hold ({lo} to {hi})."
            ))
        }
    };
    let mut out = vec![
        ("liftMaster".to_owned(), 0.),
        ("gammaMaster".to_owned(), 0.),
        ("gainMaster".to_owned(), 1.),
        ("offsetMaster".to_owned(), 0.),
    ];
    for (c, channel) in ["Red", "Green", "Blue"].into_iter().enumerate() {
        out.push((format!("lift{channel}"), 0.));
        out.push((
            format!("gain{channel}"),
            held("Slope", cdl.slope[c], 0., 8.)?,
        ));
        out.push((
            format!("offset{channel}"),
            held("Offset", cdl.offset[c], -1., 1.)?,
        ));
        let power = held("Power", cdl.power[c], 0.25, 4.)?;
        out.push((format!("gamma{channel}"), -power.log2()));
    }
    out.push((
        "saturation".to_owned(),
        held("Saturation", cdl.saturation, 0., 2.)? * 100.,
    ));
    Ok(out)
}

/// The CDL Color Wheels grade as, from their values (`value(key)`): slope
/// = gain × (1 − lift), offset = gain × lift + offset, power = 2^−gamma per
/// channel, saturation / 100. Refused when Temperature, Tint, Exposure or
/// Contrast are in use, which a CDL can't hold.
pub fn from_wheels(value: impl Fn(&str) -> f64) -> Result<Cdl, String> {
    let unheld: Vec<&str> = [
        ("temperature", "Temperature"),
        ("tint", "Tint"),
        ("exposure", "Exposure"),
        ("contrast", "Contrast"),
    ]
    .into_iter()
    .filter(|(key, _)| value(key) != 0.)
    .map(|(_, label)| label)
    .collect();
    if !unheld.is_empty() {
        return Err(format!(
            "A CDL holds slope, offset, power and saturation only: set {} to 0 to export one.",
            unheld.join(", ")
        ));
    }
    let mut cdl = Cdl {
        saturation: value("saturation") / 100.,
        ..Default::default()
    };
    for (c, channel) in ["Red", "Green", "Blue"].into_iter().enumerate() {
        let lift = value("liftMaster") + value(&format!("lift{channel}"));
        let gain = value("gainMaster") * value(&format!("gain{channel}"));
        let offset = value("offsetMaster") + value(&format!("offset{channel}"));
        let gamma = value("gammaMaster") + value(&format!("gamma{channel}"));
        cdl.slope[c] = gain * (1. - lift);
        cdl.offset[c] = gain * lift + offset;
        cdl.power[c] = (-gamma).exp2();
    }
    if cdl.slope.iter().any(|s| *s < 0.) {
        return Err("Lift above 1 makes a negative slope, which a CDL can’t hold.".into());
    }
    Ok(cdl)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.cdl` as on-set tools write it: a namespace, a decision list,
    /// descriptions and a viewing note.
    const ON_SET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ColorDecisionList xmlns="urn:ASC:CDL:v1.01">
  <Description>Day 3</Description>
  <ColorDecision>
    <ColorCorrection id="A003C012">
      <Description>warm</Description>
      <SOPNode>
        <Description>primary</Description>
        <Slope>1.1 0.98 0.9</Slope>
        <Offset>-0.02 0 0.015</Offset>
        <Power>0.95 1.0 1.05</Power>
      </SOPNode>
      <SATNode>
        <Saturation>1.2</Saturation>
      </SATNode>
    </ColorCorrection>
  </ColorDecision>
</ColorDecisionList>
"#;

    #[test]
    fn a_decision_list_from_set_is_read() {
        let cdl = parse(ON_SET).unwrap();
        assert_eq!(cdl.id.as_deref(), Some("A003C012"));
        assert_eq!(cdl.slope, [1.1, 0.98, 0.9]);
        assert_eq!(cdl.offset, [-0.02, 0., 0.015]);
        assert_eq!(cdl.power, [0.95, 1., 1.05]);
        // v1.01 spells it SATNode.
        assert_eq!(cdl.saturation, 1.2);
    }

    #[test]
    fn a_correction_with_a_prefix_and_saturation_is_read() {
        let text = r#"<cdl:ColorCorrection xmlns:cdl="urn:ASC:CDL:v1.2" id="look">
 <cdl:SOPNode><cdl:Slope>2 2 2</cdl:Slope><cdl:Offset>0.1 0.1 0.1</cdl:Offset><cdl:Power>1 1 1</cdl:Power></cdl:SOPNode>
 <cdl:SatNode><cdl:Saturation>0.5</cdl:Saturation></cdl:SatNode>
</cdl:ColorCorrection>"#;
        let cdl = parse(text).unwrap();
        assert_eq!(cdl.slope, [2.; 3]);
        assert_eq!(cdl.offset, [0.1; 3]);
        assert_eq!(cdl.saturation, 0.5);
    }

    #[test]
    fn written_files_read_back_exactly() {
        let cdl = Cdl {
            id: Some("shot <1> & 2".into()),
            slope: [1.25, 0.875, 1.],
            offset: [-0.0625, 0., 0.03125],
            power: [0.8, 1.25, 1.],
            saturation: 1.5,
        };
        for layout in [Layout::Correction, Layout::DecisionList] {
            let text = write(&cdl, layout);
            assert!(text.contains("urn:ASC:CDL:v1.2"), "{text}");
            assert_eq!(parse(&text).unwrap(), cdl, "{text}");
        }
        assert_eq!(Layout::of("Shot.CDL"), Layout::DecisionList);
        assert_eq!(Layout::of("shot.cc"), Layout::Correction);
    }

    #[test]
    fn what_the_specification_does_not_allow_is_refused_in_a_sentence() {
        let correction = |sop: &str, sat: &str| {
            format!(
                "<ColorCorrection><SOPNode>{sop}</SOPNode><SatNode><Saturation>{sat}</Saturation></SatNode></ColorCorrection>"
            )
        };
        for (text, says) in [
            (
                correction("<Slope>-1 1 1</Slope>", "1"),
                "Slope is negative",
            ),
            (
                correction("<Power>1 0 1</Power>", "1"),
                "Power isn’t positive",
            ),
            (correction("", "-0.5"), "Saturation is negative"),
            (
                correction("<Slope>1 1</Slope>", "1"),
                "Slope isn’t three numbers",
            ),
            (
                correction("<Offset>a b c</Offset>", "1"),
                "Offset isn’t three numbers",
            ),
            ("<ColorCorrectionCollection/>".to_owned(), "no ASC CDL"),
            (
                format!("{}{}", correction("", "1"), correction("", "1")),
                "holds 2 color corrections",
            ),
        ] {
            let error = parse(&text).unwrap_err();
            assert!(error.contains(says), "{error}");
            assert!(error.ends_with('.'), "{error}");
        }
    }

    #[test]
    fn color_wheels_hold_a_cdl_and_give_it_back() {
        let cdl = parse(ON_SET).unwrap();
        let wheels: std::collections::HashMap<String, f64> =
            to_wheels(&cdl).unwrap().into_iter().collect();
        let back = from_wheels(|k| wheels.get(k).copied().unwrap_or(0.)).unwrap();
        for c in 0..3 {
            assert!((back.slope[c] - cdl.slope[c]).abs() < 1e-12);
            assert!((back.offset[c] - cdl.offset[c]).abs() < 1e-12);
            assert!((back.power[c] - cdl.power[c]).abs() < 1e-12);
        }
        assert!((back.saturation - cdl.saturation).abs() < 1e-12);
        // Lift folds into slope and offset.
        let lifted = from_wheels(|k| match k {
            "liftMaster" => 0.1,
            "gainMaster" | "gainRed" | "gainGreen" | "gainBlue" => 1.,
            "saturation" => 100.,
            _ => 0.,
        })
        .unwrap();
        assert!((lifted.slope[0] - 0.9).abs() < 1e-12 && (lifted.offset[0] - 0.1).abs() < 1e-12);
        let error = from_wheels(|k| if k == "exposure" { 1. } else { 0. }).unwrap_err();
        assert!(error.contains("set Exposure to 0"), "{error}");
        let error = to_wheels(&Cdl {
            slope: [9., 1., 1.],
            ..Default::default()
        })
        .unwrap_err();
        assert!(error.contains("Slope 9 is outside"), "{error}");
    }

    /// The published definition, on numbers worked by hand.
    #[test]
    fn a_correction_is_applied_as_the_specification_says() {
        let cdl = Cdl {
            slope: [2., 1., 0.5],
            offset: [0.1, 0., -0.1],
            power: [1., 2., 1.],
            saturation: 1.,
            ..Default::default()
        };
        let out = apply(&cdl, [0.25, 0.5, 0.1]);
        for (got, want) in out.iter().zip([0.6, 0.25, -0.05]) {
            assert!((got - want).abs() < 1e-12, "{out:?}");
        }
        let grey = apply(
            &Cdl {
                saturation: 0.,
                ..Default::default()
            },
            [1., 0., 0.],
        );
        assert!(grey.iter().all(|v| (v - 0.2126).abs() < 1e-12), "{grey:?}");
    }
}
