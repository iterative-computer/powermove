//! A display-only update check using the same policy and enclosure as Sparkle.
use crate::update_distribution::{
    self, Architecture, Channel, Check, SignatureVerifier, Transport, UpdateConfig,
};
use anyhow::{Context, Result, ensure};
use std::io::Read;
use url::Url;

const SPARKLE: &str = "http://www.andymatuschak.org/xml-namespaces/sparkle";
const MAX_APPCAST: u64 = 128 * 1024;
const DOWNLOAD_ORIGIN: &str = "https://cloud.trypowermove.com";

#[derive(Debug, PartialEq, Eq)]
pub struct Notice {
    pub version: String,
    pub download_url: Url,
}

pub fn check(
    config: &UpdateConfig,
    appcast: &Url,
    macos_major: u32,
    current: &str,
    architecture: Architecture,
    now: u64,
    transport: &mut impl Transport,
    verifier: &impl SignatureVerifier,
) -> Result<Option<Notice>> {
    if !config.enabled || config.channel != Channel::Latest || macos_major < 14 {
        return Ok(None);
    }
    update_distribution::validate_https(appcast)?;
    let Check::Available(offer) = update_distribution::check_for_download(
        config,
        macos_major,
        current,
        architecture,
        now,
        transport,
        verifier,
    )?
    else {
        return Ok(None);
    };
    let release = offer.release();
    let version = semver::Version::parse(&release.version)?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty(),
        "stable numeric version required"
    );

    let mut xml = String::new();
    transport
        .open(appcast)?
        .take(MAX_APPCAST + 1)
        .read_to_string(&mut xml)?;
    ensure!(
        xml.len() as u64 <= MAX_APPCAST,
        "appcast exceeds size limit"
    );
    let doc = roxmltree::Document::parse(&xml)?;
    let rss = doc.root_element();
    ensure!(rss.has_tag_name("rss"), "expected RSS appcast");
    let channel = rss
        .children()
        .find(|node| node.has_tag_name("channel"))
        .context("missing RSS channel")?;
    let matching = channel
        .children()
        .filter(|item| item.has_tag_name("item"))
        .filter(|item| {
            let field = |name| {
                item.children()
                    .find(|node| node.has_tag_name((SPARKLE, name)))
                    .and_then(|node| node.text())
            };
            if field("version") != Some(release.sparkle_version())
                || field("shortVersionString") != Some(release.version.as_str())
                || field("channel").is_some()
            {
                return false;
            }
            let mut enclosures = item
                .children()
                .filter(|node| node.has_tag_name("enclosure"));
            enclosures.next().is_some_and(|asset| {
                asset.attribute("url") == Some(release.archive_url.as_str())
                    && asset.attribute("length").and_then(|len| len.parse().ok())
                        == Some(release.size)
                    && asset.attribute((SPARKLE, "edSignature"))
                        == Some(release.sparkle_signature.as_str())
                    && asset.attribute((SPARKLE, "deltaFrom")).is_none()
            }) && enclosures.next().is_none()
        })
        .count();
    ensure!(
        matching == 1,
        "appcast does not bind to the signed update policy"
    );

    // The feed cannot choose an external destination for the browser.
    let architecture = match architecture {
        Architecture::Arm64 => "arm64",
        Architecture::X86_64 => "x64",
    };
    let download_url = Url::parse(&format!(
        "{DOWNLOAD_ORIGIN}/native/{version}/Powermove-Native-{version}-mac-{architecture}.dmg"
    ))?;
    Ok(Some(Notice {
        version: release.version.clone(),
        download_url,
    }))
}
