#[path = "../../media.rs"]
pub mod media;

#[cfg(test)]
mod tests {
    use super::media;
    use pm_kernel::{Kernel, KernelConfig, LibraryStore, OpenSource};
    use serde_json::{Value, json};
    use std::fs;

    /// Real kernel journal replay → canonical publication → corpus selection/binding.
    fn journal_asset(asset: Value, reference: Value) -> Value {
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join("store");
        fs::create_dir(&store).unwrap();
        let base = json!({"id":"synthetic","compId":"C1","compName":"Synthetic","w":32,"h":24,"fps":30,"dur":1,"bg":"#000000","revision":0,"layers":[],"assets":{},"comps":{}});
        let journal = json!([{"revision":1,"patches":[
            {"path":["assets","clip"],"exists":true,"value":asset},
            {"path":["layers"],"exists":true,"value":[{"id":"layer","type":"video","d":reference,"from":0,"dur":1}]}]}]);
        fs::write(store.join("project.synthetic.json"), base.to_string()).unwrap();
        fs::write(
            store.join("projectJournal.synthetic.json"),
            journal.to_string(),
        )
        .unwrap();
        let loaded = LibraryStore::open(root.path())
            .load("synthetic")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.journal_applied, 1);
        let mut kernel = Kernel::new(KernelConfig::default());
        kernel
            .open("synthetic", OpenSource::Library(loaded))
            .unwrap();
        let doc = kernel
            .project("synthetic")
            .unwrap()
            .store()
            .snapshot()
            .document
            .to_value()
            .unwrap();
        let published: Value =
            serde_json::from_str(&pm_model::json::to_canonical_string(&doc)).unwrap();
        let selected = media::project_assets(&published);
        assert_eq!(selected.as_object().unwrap().len(), 1);
        selected["clip"].clone()
    }

    #[test]
    fn journal_added_asset_uses_content_key_before_stale_bare_id() {
        let a = journal_asset(
            json!({"storageKey":"media:current","revision":"r2"}),
            json!({"asset":"clip"}),
        );
        let map = json!({"records":[
            {"key":"clip","revision":"r2","path":"stale","sha256":"old"},
            {"key":"media:current","revision":"r2","path":"current","sha256":"new"}]});
        assert_eq!(
            media::binding("clip", &a, &map)
                .unwrap()
                .unwrap()
                .path
                .to_str(),
            Some("current")
        );
    }

    #[test]
    fn journal_texture_map_is_selected_and_bare_id_fallback_binds() {
        let a = journal_asset(
            json!({"fingerprint":"new","revision":"r2"}),
            json!({"data":{"object":{"material":{"maps":{"albedo":"clip"}}}}}),
        );
        let map =
            json!({"records":[{"key":"clip","revision":"r2","path":"fallback","sha256":"same"}]});
        assert_eq!(
            media::binding("clip", &a, &map)
                .unwrap()
                .unwrap()
                .path
                .to_str(),
            Some("fallback")
        );
    }

    #[test]
    fn journal_revision_mismatch_and_unavailable_original_stay_offline() {
        let a = journal_asset(
            json!({"storageKey":"media:x","revision":"new","sourcePath":"/synthetic/unavailable"}),
            json!({"asset":"clip"}),
        );
        let map =
            json!({"records":[{"key":"media:x","revision":"old","path":"wrong"}],"linked":{}});
        assert_eq!(media::binding("clip", &a, &map).unwrap(), None);
    }

    #[test]
    fn journal_proxy_record_is_never_an_export_original() {
        let a = journal_asset(json!({"storageKey":"media:x"}), json!({"asset":"clip"}));
        for flags in [
            json!({"playbackProxy":true}),
            json!({"playbackProxyVersion":1}),
        ] {
            let map = json!({"records":[{"key":"media:x","path":"proxy","value":flags}]});
            assert!(media::binding("clip", &a, &map).unwrap().unwrap().proxy);
        }
    }

    #[test]
    fn journal_link_fallback_requires_capture_and_ambiguous_bytes_are_refused() {
        let a = journal_asset(
            json!({"link":{"path":"/synthetic/linked"}}),
            json!({"asset":"clip"}),
        );
        let map = json!({"records":[],"linked":{"/synthetic/linked":"copied"}});
        assert_eq!(
            media::binding("clip", &a, &map)
                .unwrap()
                .unwrap()
                .path
                .to_str(),
            Some("copied")
        );
        for second in [
            json!({"key":"clip","path":"two"}),
            json!({"key":"clip","path":"two","sha256":"other"}),
        ] {
            let map = json!({"records":[{"key":"clip","path":"one","sha256":"same"},second]});
            assert_eq!(media::binding("clip", &a, &map), Err("ambiguous_media"));
        }
    }

    #[test]
    fn duplicate_origins_require_complete_equal_checksums_and_proxy_provenance() {
        let hash = "a".repeat(64);
        let a = json!({});
        let map = json!({"records":[{"key":"clip","path":"one","sha256":hash},
            {"key":"clip","path":"two","sha256":hash}]});
        assert_eq!(
            media::binding("clip", &a, &map)
                .unwrap()
                .unwrap()
                .path
                .to_str(),
            Some("one")
        );
        let mut proxy = map.clone();
        proxy["records"][1]["value"] = json!({"playbackProxy":true});
        assert_eq!(media::binding("clip", &a, &proxy), Err("ambiguous_media"));
        let mut partial = map;
        partial["records"][1]
            .as_object_mut()
            .unwrap()
            .remove("sha256");
        assert_eq!(media::binding("clip", &a, &partial), Err("ambiguous_media"));
    }

    #[test]
    fn captured_content_hashes_and_snapshot_containment_are_verified() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("original");
        fs::write(&path, b"synthetic original").unwrap();
        let hash = blake3::hash(b"synthetic original").to_hex().to_string();
        let a = journal_asset(json!({"contentHash":hash}), json!({"asset":"clip"}));
        let map = json!({"records":[{"key":"clip","path":path,"blake3":hash}]});
        let binding = media::binding("clip", &a, &map).unwrap().unwrap();
        binding.verify(root.path()).unwrap();
        fs::write(&path, b"same path, wrong bytes").unwrap();
        assert!(
            binding
                .verify(root.path())
                .unwrap_err()
                .to_string()
                .contains("media_content_hash_mismatch")
        );
        let other = tempfile::tempdir().unwrap();
        assert!(
            binding
                .verify(other.path())
                .unwrap_err()
                .to_string()
                .contains("escaped snapshot")
        );
    }

    #[test]
    fn history_and_unused_assets_do_not_expand_current_selection() {
        let doc = json!({"assets":{"used":{},"unused":{}},"layers":[{"d":{"asset":"used"}}],"history":{"asset":"unused"}});
        assert_eq!(media::project_assets(&doc), json!({"used":{}}));
    }
}
pub mod shaders;

#[path = "../../rotation_pixels.rs"]
pub mod rotation_pixels;
