//! Opt-in end-to-end verification for the published signed Overture pack.
//!
//! Run with `RUN_RELEASE_PACK_TEST=1`.  The normal test suite never downloads
//! a release asset or writes outside its temporary directory.

use places_core::{DataPackManager, LocalDataset, LocalPoiStore};
use std::path::PathBuf;

const MANIFEST_URL: &str = "https://github.com/Tommyweige/GPX-Animator-GPU/releases/download/poi-2026.07.14/poi-manifest.json";
const PUBLIC_KEY_HEX: &str = "6c39c86798e836d9f312c5737ed916bfd5ed4b964dee43dd51eaf9d0b01bd207";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("RUN_RELEASE_PACK_TEST").ok().as_deref() != Some("1") {
        println!("set RUN_RELEASE_PACK_TEST=1 to verify the signed release pack");
        return Ok(());
    }
    let root =
        std::env::temp_dir().join(format!("gpx-animator-release-pack-{}", std::process::id()));
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    let manager =
        DataPackManager::new(&root)?.with_signature_policy(Some(PUBLIC_KEY_HEX.to_owned()), true);
    let paths = manager.download_manifest_and_install(MANIFEST_URL)?;
    let path = paths
        .iter()
        .find(|path| path.ends_with(LocalDataset::Overture.file_name()))
        .ok_or("manifest did not install Overture")?;
    let store = LocalPoiStore::open(PathBuf::from(path), LocalDataset::Overture)?;
    let stats = store.stats()?;
    assert!(
        stats.place_count > 100_000,
        "unexpected pack size: {stats:?}"
    );
    println!(
        "verified {} places at {}",
        stats.place_count,
        path.display()
    );
    drop(store);
    std::fs::remove_dir_all(root)?;
    Ok(())
}
