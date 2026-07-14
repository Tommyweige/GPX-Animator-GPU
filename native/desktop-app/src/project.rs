//! Per-route landmark projects.
//!
//! The original GPX remains untouched.  A small sidecar JSON stores only the
//! user-selected open-data landmarks and their visual settings.  If the GPX
//! directory is read-only, the same project is stored under AppData keyed by
//! the GPX content hash.

use gpx_core::Track;
use scene_core::{RouteLandmark, anchor_landmark_to_route};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const PROJECT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectFile {
    schema_version: u32,
    gpx_sha256: String,
    landmarks: Vec<RouteLandmark>,
}

#[derive(Debug, Clone)]
pub struct LoadedProject {
    pub landmarks: Vec<RouteLandmark>,
    pub path: PathBuf,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectSaveLocation {
    Sidecar(PathBuf),
    AppData(PathBuf),
}

fn source_hash(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(hex_lower(&Sha256::digest(bytes)))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push_str(&format!("{byte:02x}"));
    }
    result
}

pub fn sidecar_path(gpx_path: &Path) -> PathBuf {
    gpx_path.with_extension("gpxanimator.json")
}

fn app_data_path(hash: &str) -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("GPX Animator")
        .join("projects")
        .join(format!("{hash}.gpxanimator.json"))
}

fn read_project(path: &Path) -> io::Result<ProjectFile> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "project has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(first_error) if path.exists() => {
            fs::remove_file(path)?;
            fs::rename(&temporary, path).map_err(|_| first_error)
        }
        Err(error) => Err(error),
    }
}

fn reanchor(mut landmarks: Vec<RouteLandmark>, track: &Track) -> Vec<RouteLandmark> {
    for landmark in &mut landmarks {
        if let Some(anchor) = anchor_landmark_to_route(track, landmark.latitude, landmark.longitude)
        {
            landmark.anchor_distance_m = anchor.anchor_distance_m;
            landmark.anchor_progress = anchor.anchor_progress;
            landmark.distance_from_route_m = anchor.distance_from_route_m;
        }
    }
    landmarks.sort_by(|a, b| {
        a.anchor_distance_m
            .total_cmp(&b.anchor_distance_m)
            .then_with(|| a.id.cmp(&b.id))
    });
    landmarks
}

pub fn load_for_route(gpx_path: &Path, track: &Track) -> io::Result<LoadedProject> {
    let hash = source_hash(gpx_path)?;
    let sidecar = sidecar_path(gpx_path);
    let fallback = app_data_path(&hash);
    let mut selected = None;
    let mut warning = None;
    for candidate in [&sidecar, &fallback] {
        if !candidate.exists() {
            continue;
        }
        match read_project(candidate) {
            Ok(project) if project.schema_version == PROJECT_SCHEMA_VERSION => {
                selected = Some((candidate.clone(), project));
                break;
            }
            Ok(_) => {
                warning = Some("Project schema is newer than this app; ignored.".to_owned());
            }
            Err(error) => {
                let corrupt = candidate.with_extension(format!(
                    "corrupt-{}.json",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |value| value.as_secs())
                ));
                let _ = fs::rename(candidate, corrupt);
                warning = Some(format!(
                    "Project file was corrupt and was backed up: {error}"
                ));
            }
        }
    }
    let Some((path, project)) = selected else {
        return Ok(LoadedProject {
            landmarks: Vec::new(),
            path: sidecar,
            warning,
        });
    };
    if project.gpx_sha256 != hash {
        warning = Some(
            "GPX content changed; saved landmarks were re-anchored to the new route.".to_owned(),
        );
    }
    Ok(LoadedProject {
        landmarks: reanchor(project.landmarks, track),
        path,
        warning,
    })
}

pub fn save_for_route(
    gpx_path: &Path,
    landmarks: &[RouteLandmark],
    track: &Track,
) -> io::Result<ProjectSaveLocation> {
    let hash = source_hash(gpx_path)?;
    let project = ProjectFile {
        schema_version: PROJECT_SCHEMA_VERSION,
        gpx_sha256: hash.clone(),
        landmarks: reanchor(landmarks.to_vec(), track),
    };
    let bytes = serde_json::to_vec_pretty(&project).map_err(io::Error::other)?;
    let sidecar = sidecar_path(gpx_path);
    match write_atomic(&sidecar, &bytes) {
        Ok(()) => Ok(ProjectSaveLocation::Sidecar(sidecar)),
        Err(sidecar_error) => {
            let fallback = app_data_path(&hash);
            write_atomic(&fallback, &bytes).map_err(|fallback_error| {
                io::Error::other(format!(
                    "sidecar failed: {sidecar_error}; AppData fallback failed: {fallback_error}"
                ))
            })?;
            Ok(ProjectSaveLocation::AppData(fallback))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpx_core::{ParseOptions, parse_gpx};
    use scene_core::{LandmarkSource, LandmarkStyle};

    fn track() -> Track {
        parse_gpx(
            r#"<gpx><trk><trkseg>
            <trkpt lat="25" lon="121"/><trkpt lat="25" lon="121.01"/>
            <trkpt lat="25.01" lon="121.02"/>
            </trkseg></trk></gpx>"#,
            ParseOptions::default(),
        )
        .unwrap()
    }

    fn landmark() -> RouteLandmark {
        RouteLandmark {
            id: "overture:demo".into(),
            source: LandmarkSource::Overture,
            source_id: Some("demo".into()),
            name: "Demo place".into(),
            category: Some("park".into()),
            latitude: 25.0,
            longitude: 121.005,
            anchor_distance_m: 0.0,
            anchor_progress: 0.0,
            distance_from_route_m: 0.0,
            enabled: true,
            style: LandmarkStyle::default(),
        }
    }

    #[test]
    fn project_round_trips_and_reanchors() {
        let root = std::env::temp_dir().join(format!("gpx-project-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let gpx = root.join("route.gpx");
        fs::write(&gpx, "<gpx/>\n").unwrap();
        let route = track();
        let places = vec![landmark()];
        let saved = save_for_route(&gpx, &places, &route).unwrap();
        assert!(matches!(saved, ProjectSaveLocation::Sidecar(_)));
        let loaded = load_for_route(&gpx, &route).unwrap();
        assert_eq!(loaded.landmarks.len(), 1);
        assert!(loaded.landmarks[0].anchor_progress > 0.0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_project_is_backed_up() {
        let root = std::env::temp_dir().join(format!("gpx-project-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let gpx = root.join("route.gpx");
        fs::write(&gpx, "<gpx/>\n").unwrap();
        fs::write(sidecar_path(&gpx), b"not json").unwrap();
        let loaded = load_for_route(&gpx, &track()).unwrap();
        assert!(loaded.landmarks.is_empty());
        assert!(loaded.warning.is_some());
        assert!(
            fs::read_dir(&root)
                .unwrap()
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().contains("corrupt-"))
        );
        let _ = fs::remove_dir_all(root);
    }
}
