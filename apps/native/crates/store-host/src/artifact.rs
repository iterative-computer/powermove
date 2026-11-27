//! Signed, bounded archive admission; bundles stay immutable and unextracted.
use crate::{Error, Result, hex, identity::ExtensionId, permission::Manifest};
use ed25519_dalek::{Signature, VerifyingKey};
use flate2::read::MultiGzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSeal {
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseStatement {
    pub format: u32,
    pub key_id: String,
    pub publisher: String,
    pub extension: ExtensionId,
    pub release_id: String,
    pub archive_sha256: String,
    pub files: BTreeMap<String, FileSeal>,
}
impl ReleaseStatement {
    /// Protocol v1 signing bytes: domain + deterministic struct JSON. Map keys
    /// sort lexicographically; this is not arbitrary JSON/JCS canonicalization.
    pub fn signing_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = b"powermove.store-release.v1\0".to_vec();
        bytes.extend(serde_json::to_vec(self).map_err(|_| Error::Invalid("release statement"))?);
        Ok(bytes)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRelease {
    pub statement: ReleaseStatement,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub archive_bytes: usize,
    pub expanded_bytes: usize,
    pub files: usize,
    pub manifest_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            archive_bytes: 64 << 20,
            expanded_bytes: 128 << 20,
            files: 4096,
            manifest_bytes: 1 << 20,
        }
    }
}
/// Store-controlled publisher display metadata. Signature validity by itself
/// is not a verified-publisher badge. Never populate this from a manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublisherIdentity {
    pub name: String,
    pub verified: bool,
}
#[derive(Debug, Default)]
pub struct TrustStore(BTreeMap<String, (VerifyingKey, PublisherIdentity)>);
impl TrustStore {
    pub fn trust(&mut self, publisher: String, key_id: String, key: VerifyingKey) -> Result<()> {
        self.trust_with_identity(publisher, key_id, key, PublisherIdentity::default())
    }
    /// Host-only provisioning from the authenticated Store publisher registry.
    pub fn trust_with_identity(
        &mut self,
        publisher: String,
        key_id: String,
        key: VerifyingKey,
        identity: PublisherIdentity,
    ) -> Result<()> {
        let id = trust_id(&publisher, &key_id);
        if publisher.is_empty() || key_id.is_empty() || self.0.contains_key(&id) {
            return Err(Error::Conflict);
        }
        self.0.insert(id, (key, identity));
        Ok(())
    }
    pub fn revoke(&mut self, publisher: &str, key_id: &str) {
        self.0.remove(&trust_id(publisher, key_id));
    }
}
fn trust_id(publisher: &str, key_id: &str) -> String {
    format!("{}:{publisher}{key_id}", publisher.len())
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|p| !p.is_empty() && !p.starts_with('.') && !p.chars().any(char::is_control))
}

/// Only `verify` can construct this. Files are exact virtual paths; do not unpack
/// with a different tar reader (case/Unicode aliases on macOS can lose data).
#[derive(Debug)]
pub struct VerifiedRelease {
    pub(crate) statement: ReleaseStatement,
    pub(crate) manifest: Manifest,
    pub(crate) proof: SignedRelease,
    pub(crate) publisher_identity: PublisherIdentity,
    files: BTreeMap<String, Vec<u8>>,
    archive: Vec<u8>,
}
impl VerifiedRelease {
    pub fn statement(&self) -> &ReleaseStatement {
        &self.statement
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }
}

pub fn verify(
    release: &SignedRelease,
    archive: &[u8],
    trust: &TrustStore,
    limits: Limits,
) -> Result<VerifiedRelease> {
    let s = &release.statement;
    if s.format != 1
        || s.files.is_empty()
        || s.files.len() > limits.files
        || s.publisher.len() > 256
        || s.key_id.len() > 128
        || s.release_id.is_empty()
        || s.release_id.len() > 256
        || !valid_hash(&s.archive_sha256)
        || archive.len() > limits.archive_bytes
    {
        return Err(Error::Invalid("release bounds"));
    }
    let mut total = 0u64;
    for (path, seal) in &s.files {
        if !valid_path(path) || !valid_hash(&seal.sha256) {
            return Err(Error::Invalid("file seal"));
        }
        total = total
            .checked_add(seal.bytes)
            .ok_or(Error::Invalid("expanded size"))?;
        if total > limits.expanded_bytes as u64 {
            return Err(Error::Invalid("expanded size"));
        }
    }
    let (key, publisher_identity) = trust
        .0
        .get(&trust_id(&s.publisher, &s.key_id))
        .ok_or(Error::Integrity("untrusted signing key"))?;
    let sig = Signature::from_slice(&release.signature)
        .map_err(|_| Error::Integrity("signature length"))?;
    key.verify_strict(&s.signing_bytes()?, &sig)
        .map_err(|_| Error::Integrity("signature"))?;
    if hex(&Sha256::digest(archive)) != s.archive_sha256 {
        return Err(Error::Integrity("archive hash"));
    }
    let tar_limit = limits
        .expanded_bytes
        .checked_add(
            limits
                .files
                .checked_mul(2048)
                .ok_or(Error::Invalid("limits"))?,
        )
        .and_then(|n| n.checked_add(10240))
        .ok_or(Error::Invalid("limits"))?;
    let mut raw = Vec::new();
    MultiGzDecoder::new(archive)
        .take(tar_limit as u64 + 1)
        .read_to_end(&mut raw)?;
    if raw.len() > tar_limit {
        return Err(Error::Invalid("decompression budget"));
    }
    let mut tar = tar::Archive::new(raw.as_slice());
    let mut files = BTreeMap::new();
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            return Err(Error::Integrity("non-regular archive entry"));
        }
        let path = std::str::from_utf8(&entry.path_bytes())
            .map_err(|_| Error::Invalid("UTF-8 path"))?
            .to_string();
        if !valid_path(&path) || files.contains_key(&path) {
            return Err(Error::Integrity("duplicate or unsafe path"));
        }
        let expected = s
            .files
            .get(&path)
            .ok_or(Error::Integrity("unlisted file"))?;
        if entry.size() != expected.bytes {
            return Err(Error::Integrity("file length"));
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        if hex(&Sha256::digest(&data)) != expected.sha256 {
            return Err(Error::Integrity("file hash"));
        }
        files.insert(path, data);
    }
    if files.len() != s.files.len() {
        return Err(Error::Integrity("missing file"));
    }
    let manifest_bytes = files
        .get("manifest.json")
        .ok_or(Error::Integrity("missing manifest"))?;
    if manifest_bytes.len() > limits.manifest_bytes {
        return Err(Error::Invalid("manifest size"));
    }
    let manifest: Manifest =
        serde_json::from_slice(manifest_bytes).map_err(|_| Error::Invalid("manifest"))?;
    manifest.validate()?;
    if let Some(icon) = &manifest.icon
        && (!valid_path(icon) || !files.contains_key(icon))
    {
        return Err(Error::Integrity("manifest icon"));
    }
    if manifest.id != s.extension {
        return Err(Error::Integrity("manifest id"));
    }
    Ok(VerifiedRelease {
        statement: s.clone(),
        manifest,
        proof: release.clone(),
        publisher_identity: publisher_identity.clone(),
        files,
        archive: archive.to_vec(),
    })
}

/// What the Store's registry records of a release, read over HTTPS from the
/// Store origin: the facts [`admit_registry`] holds the archive to.
#[derive(Debug, Clone)]
pub struct RegistryRelease {
    pub release_id: String,
    /// The id the registry recorded from the manifest at publish.
    pub extension: ExtensionId,
    /// The repository owner's publisher id, under which permissions are
    /// decided.
    pub publisher: String,
    /// SHA-256 of the gzipped tar the Store serves.
    pub tar_sha256: String,
    /// The git tree hash (SHA-1) of the release's files.
    pub tree_sha: String,
}

/// Admits a release the Store does not sign, as the shipped app installs one:
/// the archive must hash to the registry's `tarSha256`, its files must hash
/// to its git `treeSha` (dot-entries and `node_modules` are outside every
/// tree, so an archive carrying one is refused), and its manifest must be
/// valid and name the extension the registry recorded. The proof kept is
/// unsigned (format 0), as a migrated install's is.
pub fn admit_registry(
    record: &RegistryRelease,
    archive: &[u8],
    limits: Limits,
) -> Result<VerifiedRelease> {
    if record.release_id.is_empty()
        || record.release_id.len() > 256
        || record.publisher.is_empty()
        || record.publisher.len() > 256
        || !valid_hash(&record.tar_sha256)
        || record.tree_sha.len() != 40
        || !record
            .tree_sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || archive.len() > limits.archive_bytes
    {
        return Err(Error::Invalid("release bounds"));
    }
    if hex(&Sha256::digest(archive)) != record.tar_sha256 {
        return Err(Error::Integrity("archive hash"));
    }
    let tar_limit = limits
        .expanded_bytes
        .checked_add(
            limits
                .files
                .checked_mul(2048)
                .ok_or(Error::Invalid("limits"))?,
        )
        .and_then(|n| n.checked_add(10240))
        .ok_or(Error::Invalid("limits"))?;
    let mut raw = Vec::new();
    MultiGzDecoder::new(archive)
        .take(tar_limit as u64 + 1)
        .read_to_end(&mut raw)?;
    if raw.len() > tar_limit {
        return Err(Error::Invalid("decompression budget"));
    }
    let mut tar = tar::Archive::new(raw.as_slice());
    let mut files = BTreeMap::new();
    let mut folded = std::collections::BTreeSet::new();
    let mut total = 0u64;
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            return Err(Error::Integrity("non-regular archive entry"));
        }
        let path = std::str::from_utf8(&entry.path_bytes())
            .map_err(|_| Error::Invalid("UTF-8 path"))?
            .to_string();
        if !valid_path(&path)
            || path.split('/').any(|p| p == "node_modules")
            || files.contains_key(&path)
            || !folded.insert(path.to_lowercase())
        {
            return Err(Error::Integrity("duplicate or unsafe path"));
        }
        total = total
            .checked_add(entry.size())
            .ok_or(Error::Invalid("expanded size"))?;
        if files.len() >= limits.files || total > limits.expanded_bytes as u64 {
            return Err(Error::Invalid("expanded size"));
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        files.insert(path, data);
    }
    if git_tree(&files)? != record.tree_sha {
        return Err(Error::Integrity("tree hash"));
    }
    let manifest_bytes = files
        .get("manifest.json")
        .ok_or(Error::Integrity("missing manifest"))?;
    if manifest_bytes.len() > limits.manifest_bytes {
        return Err(Error::Invalid("manifest size"));
    }
    let manifest: Manifest =
        serde_json::from_slice(manifest_bytes).map_err(|_| Error::Invalid("manifest"))?;
    manifest.validate()?;
    if let Some(icon) = &manifest.icon
        && (!valid_path(icon) || !files.contains_key(icon))
    {
        return Err(Error::Integrity("manifest icon"));
    }
    if manifest.id != record.extension {
        return Err(Error::Integrity("manifest id"));
    }
    let statement = ReleaseStatement {
        format: 0,
        key_id: String::new(),
        publisher: record.publisher.clone(),
        extension: record.extension.clone(),
        release_id: record.release_id.clone(),
        archive_sha256: record.tar_sha256.clone(),
        files: files
            .iter()
            .map(|(path, data)| {
                (
                    path.clone(),
                    FileSeal {
                        sha256: hex(&Sha256::digest(data)),
                        bytes: data.len() as u64,
                    },
                )
            })
            .collect(),
    };
    Ok(VerifiedRelease {
        proof: SignedRelease {
            statement: statement.clone(),
            signature: vec![],
        },
        statement,
        manifest,
        publisher_identity: PublisherIdentity::default(),
        files,
        archive: archive.to_vec(),
    })
}

/// The git tree hash of `files` (paths to bytes), as the Store's registry
/// computes it: blobs mode 100644, folders 40000, entries in byte order with
/// a folder's name read with a trailing `/`.
fn git_tree(files: &BTreeMap<String, Vec<u8>>) -> Result<String> {
    use sha1::Sha1;
    fn object(kind: &str, body: &[u8]) -> [u8; 20] {
        let mut hash = Sha1::new();
        hash.update(format!("{kind} {}\0", body.len()).as_bytes());
        hash.update(body);
        hash.finalize().into()
    }
    #[derive(Default)]
    struct Node<'a> {
        files: BTreeMap<&'a str, &'a [u8]>,
        dirs: BTreeMap<&'a str, Node<'a>>,
    }
    fn tree(node: &Node) -> [u8; 20] {
        let mut entries: Vec<(Vec<u8>, String, [u8; 20])> = Vec::new();
        for (name, data) in &node.files {
            entries.push((
                name.as_bytes().to_vec(),
                format!("100644 {name}"),
                object("blob", data),
            ));
        }
        for (name, child) in &node.dirs {
            entries.push((
                format!("{name}/").into_bytes(),
                format!("40000 {name}"),
                tree(child),
            ));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut body = Vec::new();
        for (_, head, sha) in entries {
            body.extend(head.as_bytes());
            body.push(0);
            body.extend(sha);
        }
        object("tree", &body)
    }
    let mut root = Node::default();
    for (path, data) in files {
        let mut parts: Vec<&str> = path.split('/').collect();
        let name = parts.pop().ok_or(Error::Invalid("path"))?;
        let mut cursor = &mut root;
        for part in parts {
            if cursor.files.contains_key(part) {
                return Err(Error::Integrity("file and folder share a name"));
            }
            cursor = cursor.dirs.entry(part).or_default();
        }
        if cursor.dirs.contains_key(name) {
            return Err(Error::Integrity("file and folder share a name"));
        }
        cursor.files.insert(name, data);
    }
    if files.is_empty() {
        return Err(Error::Integrity("empty release"));
    }
    Ok(hex(&tree(&root)))
}

/// Worker-only; completion means the complete object is durable. Never removes
/// bundles or user data, including failed candidates. Kernel publishes references.
pub trait ArtifactStore {
    fn preserve(&mut self, release: &VerifiedRelease) -> Result<()>;
}
#[derive(Debug)]
pub struct FileArtifacts {
    root: PathBuf,
}
impl FileArtifacts {
    /// `root` must be an app-owned private directory, not extension-controlled.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("objects"))?;
        fs::create_dir_all(root.join(".staging"))?;
        Ok(Self { root })
    }
    pub fn object_path(&self, digest: &str) -> Result<PathBuf> {
        if !valid_hash(digest) {
            return Err(Error::Invalid("bundle hash"));
        }
        Ok(self.root.join("objects").join(format!("{digest}.tar.gz")))
    }
    /// Recheck bytes before returning an object to compiler/reader adapters.
    pub fn read(&self, digest: &str, limit: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        File::open(self.object_path(digest)?)?
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit || hex(&Sha256::digest(&bytes)) != digest {
            return Err(Error::Integrity("stored bundle"));
        }
        Ok(bytes)
    }
}
static STAGING: AtomicU64 = AtomicU64::new(1);
fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}
impl ArtifactStore for FileArtifacts {
    fn preserve(&mut self, release: &VerifiedRelease) -> Result<()> {
        let digest = &release.statement.archive_sha256;
        let dest = self.object_path(digest)?;
        let staged = self.root.join(".staging").join(format!(
            "{}-{}-{}",
            digest,
            std::process::id(),
            STAGING.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        file.write_all(&release.archive)?;
        file.sync_all()?;
        sync_dir(&self.root.join(".staging"))?;
        match fs::hard_link(&staged, &dest) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                self.read(digest, release.archive.len())?;
            }
            Err(e) => return Err(e.into()),
        }
        // Keep the staged hard link as a recovery archive, including on failure.
        sync_dir(&self.root.join("objects"))?;
        sync_dir(&self.root)?;
        Ok(())
    }
}
