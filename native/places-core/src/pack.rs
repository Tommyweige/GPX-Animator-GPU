//! Signed, versioned local POI data-pack delivery.
//!
//! The desktop application downloads packs on first use and then opens them
//! from `%LOCALAPPDATA%`.  The manifest hash covers the downloaded archive;
//! the optional uncompressed hash covers the SQLite payload.  Production
//! builds set `require_signature=true` and ship the release public key via a
//! non-secret build configuration, while tests can exercise hash-only mode.

use crate::{LocalDataset, PlacesError};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataPackManifest {
    pub schema_version: u32,
    pub pack_id: String,
    pub version: String,
    pub dataset: LocalDatasetWire,
    pub url: String,
    /// SHA-256 of the downloaded bytes (normally a `.sqlite3.zst` archive).
    pub sha256: String,
    #[serde(default)]
    pub uncompressed_sha256: Option<String>,
    #[serde(default)]
    pub bytes: Option<u64>,
    /// Hex encoded Ed25519 signature over `canonical_payload()`.
    #[serde(default)]
    pub signature_hex: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalDatasetWire {
    Overture,
    OpenStreetMap,
}

impl From<LocalDatasetWire> for LocalDataset {
    fn from(value: LocalDatasetWire) -> Self {
        match value {
            LocalDatasetWire::Overture => LocalDataset::Overture,
            LocalDatasetWire::OpenStreetMap => LocalDataset::OpenStreetMap,
        }
    }
}

impl From<LocalDataset> for LocalDatasetWire {
    fn from(value: LocalDataset) -> Self {
        match value {
            LocalDataset::Overture => Self::Overture,
            LocalDataset::OpenStreetMap => Self::OpenStreetMap,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataPackVerification {
    Verified {
        compressed_sha256: String,
        uncompressed_sha256: Option<String>,
        signature_verified: bool,
    },
    HashOnly {
        compressed_sha256: String,
        uncompressed_sha256: Option<String>,
    },
}

impl DataPackManifest {
    pub fn canonical_payload(&self) -> String {
        format!(
            "{}\n{}\n{}\n{:?}\n{}\n{}\n{}",
            self.schema_version,
            self.pack_id,
            self.version,
            self.dataset,
            self.url,
            self.sha256.to_ascii_lowercase(),
            self.uncompressed_sha256.as_deref().unwrap_or_default(),
        )
    }

    pub fn verify(
        &self,
        compressed: &[u8],
        uncompressed: Option<&[u8]>,
        public_key_hex: Option<&str>,
        require_signature: bool,
    ) -> Result<DataPackVerification, PlacesError> {
        let compressed_sha256 = sha256_hex(compressed);
        if !constant_time_eq(&compressed_sha256, &self.sha256.to_ascii_lowercase()) {
            return Err(PlacesError::Storage(format!(
                "data pack hash mismatch: expected {}, got {}",
                self.sha256, compressed_sha256
            )));
        }
        let uncompressed_sha256 = uncompressed.map(sha256_hex);
        if let Some(expected) = &self.uncompressed_sha256 {
            let Some(actual) = &uncompressed_sha256 else {
                return Err(PlacesError::Storage(
                    "data pack requires an uncompressed hash but no payload was supplied".into(),
                ));
            };
            if !constant_time_eq(actual, &expected.to_ascii_lowercase()) {
                return Err(PlacesError::Storage(format!(
                    "uncompressed data pack hash mismatch: expected {}, got {}",
                    expected, actual
                )));
            }
        }
        let Some(signature_hex) = self.signature_hex.as_deref() else {
            if require_signature {
                return Err(PlacesError::Storage(
                    "signed data pack manifest is required but signature_hex is missing".into(),
                ));
            }
            return Ok(DataPackVerification::HashOnly {
                compressed_sha256,
                uncompressed_sha256,
            });
        };
        let Some(public_key_hex) = public_key_hex else {
            return Err(PlacesError::Storage(
                "data pack signature is present but the release public key is unavailable".into(),
            ));
        };
        let public_key = decode_fixed_hex::<32>(public_key_hex)
            .map_err(|error| PlacesError::Storage(format!("invalid pack public key: {error}")))?;
        let signature = decode_fixed_hex::<64>(signature_hex)
            .map_err(|error| PlacesError::Storage(format!("invalid pack signature: {error}")))?;
        let key = VerifyingKey::from_bytes(&public_key)
            .map_err(|error| PlacesError::Storage(format!("invalid pack public key: {error}")))?;
        key.verify(
            self.canonical_payload().as_bytes(),
            &Signature::from_bytes(&signature),
        )
        .map_err(|error| {
            PlacesError::Storage(format!("data pack signature verification failed: {error}"))
        })?;
        Ok(DataPackVerification::Verified {
            compressed_sha256,
            uncompressed_sha256,
            signature_verified: true,
        })
    }
}

pub struct DataPackManager {
    root: PathBuf,
    agent: ureq::Agent,
    pub_key_hex: Option<String>,
    pub require_signature: bool,
}

/// Maximum compressed payload accepted by the signed release channel.  Taiwan
/// place snapshots are currently below 100 MiB, but the limit leaves room for
/// future regional packs without falling back to ureq's 10 MiB JSON default.
const MAX_DATA_PACK_BYTES: u64 = 2 * 1024 * 1024 * 1024;

impl DataPackManager {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, PlacesError> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|error| PlacesError::Storage(error.to_string()))?;
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .http_status_as_error(false)
            .build();
        Ok(Self {
            root,
            agent: config.new_agent(),
            pub_key_hex: None,
            require_signature: true,
        })
    }

    pub fn with_signature_policy(
        mut self,
        public_key_hex: Option<String>,
        require_signature: bool,
    ) -> Self {
        self.pub_key_hex = public_key_hex;
        self.require_signature = require_signature;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn target_path(&self, dataset: LocalDataset) -> PathBuf {
        self.root.join(dataset.file_name())
    }

    pub fn install_bytes(
        &self,
        manifest: &DataPackManifest,
        compressed: &[u8],
    ) -> Result<PathBuf, PlacesError> {
        let payload = if manifest.url.to_ascii_lowercase().ends_with(".zst") {
            let mut decoder = zstd::stream::read::Decoder::new(compressed)
                .map_err(|error| PlacesError::Storage(format!("zstd decoder: {error}")))?;
            let mut payload = Vec::new();
            decoder
                .read_to_end(&mut payload)
                .map_err(|error| PlacesError::Storage(format!("zstd decode: {error}")))?;
            payload
        } else {
            compressed.to_vec()
        };
        manifest.verify(
            compressed,
            Some(&payload),
            self.pub_key_hex.as_deref(),
            self.require_signature,
        )?;
        let target = self.target_path(manifest.dataset.into());
        let temporary = target.with_extension("sqlite3.tmp");
        std::fs::write(&temporary, &payload)
            .map_err(|error| PlacesError::Storage(error.to_string()))?;
        std::fs::rename(&temporary, &target)
            .map_err(|error| PlacesError::Storage(error.to_string()))?;
        Ok(target)
    }

    pub fn download_and_install(
        &self,
        manifest: &DataPackManifest,
    ) -> Result<PathBuf, PlacesError> {
        if manifest.url.trim().is_empty() {
            return Err(PlacesError::Storage("data pack URL is empty".into()));
        }
        let response = self
            .agent
            .get(&manifest.url)
            .header("User-Agent", "GPXAnimatorNative/2.0")
            .call()
            .map_err(crate::map_ureq_error)?;
        let status = response.status().as_u16();
        let bytes = response
            .into_body()
            .with_config()
            .limit(MAX_DATA_PACK_BYTES)
            .read_to_vec()
            .map_err(|error| PlacesError::Http(error.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(crate::provider_status(
                status,
                String::from_utf8_lossy(&bytes).into(),
            ));
        }
        self.install_bytes(manifest, &bytes)
    }

    pub fn download_manifest_and_install(
        &self,
        manifest_url: &str,
    ) -> Result<Vec<PathBuf>, PlacesError> {
        let response = self
            .agent
            .get(manifest_url)
            .header("User-Agent", "GPXAnimatorNative/2.0")
            .header("Accept", "application/json")
            .call()
            .map_err(crate::map_ureq_error)?;
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|error| PlacesError::Http(error.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(crate::provider_status(status, text));
        }
        let manifests = serde_json::from_str::<Vec<DataPackManifest>>(&text)
            .or_else(|_| serde_json::from_str::<DataPackManifest>(&text).map(|value| vec![value]))
            .map_err(|error| PlacesError::Parse(format!("data pack manifest: {error}")))?;
        manifests
            .iter()
            .map(|manifest| self.download_and_install(manifest))
            .collect()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], String> {
    if value.len() != N * 2 {
        return Err(format!("expected {} hex characters", N * 2));
    }
    let mut output = [0_u8; N];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = (chunk[0] as char)
            .to_digit(16)
            .ok_or_else(|| "invalid hex".to_owned())?;
        let low = (chunk[1] as char)
            .to_digit(16)
            .ok_or_else(|| "invalid hex".to_owned())?;
        output[index] = ((high << 4) | low) as u8;
    }
    Ok(output)
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .as_bytes()
            .iter()
            .zip(right.as_bytes())
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn manifest(bytes: &[u8]) -> DataPackManifest {
        DataPackManifest {
            schema_version: 1,
            pack_id: "taiwan-test".into(),
            version: "2026.07.14".into(),
            dataset: LocalDatasetWire::Overture,
            url: "https://example.test/overture.sqlite3".into(),
            sha256: sha256_hex(bytes),
            uncompressed_sha256: None,
            bytes: Some(bytes.len() as u64),
            signature_hex: None,
        }
    }

    #[test]
    fn hash_verification_rejects_tampering() {
        let data = b"sqlite bytes";
        let manifest = manifest(data);
        assert!(matches!(
            manifest.verify(b"tampered", None, None, false),
            Err(PlacesError::Storage(_))
        ));
        assert!(matches!(
            manifest.verify(data, None, None, false),
            Ok(DataPackVerification::HashOnly { .. })
        ));
    }

    #[test]
    fn ed25519_signature_verification_is_deterministic() {
        let data = b"signed sqlite bytes";
        let mut manifest = manifest(data);
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let signature = signing_key.sign(manifest.canonical_payload().as_bytes());
        manifest.signature_hex = Some(
            signature
                .to_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        );
        let public_key_hex = signing_key
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert!(matches!(
            manifest.verify(data, None, Some(&public_key_hex), true),
            Ok(DataPackVerification::Verified {
                signature_verified: true,
                ..
            })
        ));
    }

    #[test]
    fn zstd_install_writes_atomic_sqlite_payload() {
        let payload = b"SQLite format 3\0test";
        let compressed = zstd::stream::encode_all(payload.as_slice(), 1).unwrap();
        let mut manifest = manifest(&compressed);
        manifest.url = "https://example.test/overture.sqlite3.zst".into();
        manifest.uncompressed_sha256 = Some(sha256_hex(payload));
        let root = std::env::temp_dir().join(format!("gpx-pack-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let manager = DataPackManager::new(&root)
            .unwrap()
            .with_signature_policy(None, false);
        let target = manager.install_bytes(&manifest, &compressed).unwrap();
        assert_eq!(std::fs::read(target).unwrap(), payload);
        let _ = std::fs::remove_dir_all(root);
    }
}
