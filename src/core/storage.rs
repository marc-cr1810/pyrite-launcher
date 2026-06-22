//! Disk-footprint reporting and conservative cache cleanup.
//!
//! The launcher keeps `versions/`, `libraries/`, and `assets/` shared across all
//! instances under the game directory. Over time these accumulate files that no
//! installed instance references anymore (old Minecraft versions, superseded
//! loader libraries, stale asset objects). This module measures that footprint
//! and finds the safely-removable subset via mark-and-sweep against the set of
//! versions actually used by existing instances.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::api::{AssetIndex, VersionDetails};
use crate::core::instance::Instance;

/// Recursively sum the byte size of every file under `path`. Missing paths and
/// unreadable entries contribute zero rather than erroring, so a partial tree
/// still yields a usable estimate.
pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            total += dir_size(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}

/// Footprint of a single instance folder.
#[derive(Clone, Debug)]
pub struct InstanceSize {
    pub id: String,
    pub name: String,
    pub bytes: u64,
}

/// Disk-usage breakdown for the whole game directory.
#[derive(Clone, Debug, Default)]
pub struct StorageReport {
    pub instances: Vec<InstanceSize>,
    pub assets_bytes: u64,
    pub libraries_bytes: u64,
    pub versions_bytes: u64,
    pub instances_bytes: u64,
    pub total_bytes: u64,
}

/// Walk the game directory and tally per-instance and shared-cache sizes.
pub fn compute_report(game_dir: &Path) -> StorageReport {
    let mut report = StorageReport::default();

    for inst in Instance::load_all(game_dir) {
        let bytes = dir_size(&inst.path);
        report.instances_bytes += bytes;
        report.instances.push(InstanceSize {
            id: inst.id,
            name: inst.config.name,
            bytes,
        });
    }
    report.instances.sort_by(|a, b| b.bytes.cmp(&a.bytes));

    report.assets_bytes = dir_size(&game_dir.join("assets"));
    report.libraries_bytes = dir_size(&game_dir.join("libraries"));
    report.versions_bytes = dir_size(&game_dir.join("versions"));
    report.total_bytes =
        report.instances_bytes + report.assets_bytes + report.libraries_bytes + report.versions_bytes;
    report
}

/// Files identified as unreferenced by any installed instance, grouped by cache.
#[derive(Clone, Debug, Default)]
pub struct OrphanScan {
    pub versions: Vec<PathBuf>,
    pub libraries: Vec<PathBuf>,
    pub assets: Vec<PathBuf>,
    pub total_bytes: u64,
}

impl OrphanScan {
    pub fn count(&self) -> usize {
        self.versions.len() + self.libraries.len() + self.assets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }
}

/// Read a version JSON, folding `mavenFiles` into `libraries` the same way the
/// launcher does at load time.
fn read_version_details(game_dir: &Path, version_id: &str) -> Option<VersionDetails> {
    let json_path = game_dir
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));
    let content = fs::read_to_string(json_path).ok()?;
    let mut details: VersionDetails = serde_json::from_str(&content).ok()?;
    if let Some(maven) = details.maven_files.take() {
        details.libraries.extend(maven);
    }
    Some(details)
}

/// Resolve a version id plus its `inheritsFrom` ancestry into the full set of
/// version ids it depends on. Guards against cycles via the visited set.
fn collect_version_chain(game_dir: &Path, version_id: &str, live: &mut HashSet<String>) {
    if !live.insert(version_id.to_string()) {
        return; // already visited
    }
    if let Some(details) = read_version_details(game_dir, version_id)
        && let Some(parent) = details.inheritsFrom.as_deref()
    {
        collect_version_chain(game_dir, parent, live);
    }
}

/// Mark-and-sweep the shared caches against the versions referenced by every
/// installed instance. A file is an orphan only when no live version needs it,
/// so this is always safe to delete (re-downloaded on next launch if needed).
pub fn find_orphans(game_dir: &Path) -> OrphanScan {
    // 1. Live version ids: each instance's version plus inherited parents.
    let mut live_versions: HashSet<String> = HashSet::new();
    for inst in Instance::load_all(game_dir) {
        collect_version_chain(game_dir, &inst.config.version, &mut live_versions);
    }

    // 2. Referenced library paths and asset-index ids from live versions.
    let mut live_libraries: HashSet<PathBuf> = HashSet::new();
    let mut live_asset_indexes: HashSet<String> = HashSet::new();
    for version_id in &live_versions {
        let Some(details) = read_version_details(game_dir, version_id) else {
            continue;
        };
        for lib in &details.libraries {
            // Conservatively keep every path a live version mentions, ignoring OS
            // rules and including native classifiers for all platforms, so we
            // never delete a jar some installed version still references.
            if let Some(art) = lib.get_artifact() {
                live_libraries.insert(PathBuf::from(art.path));
            }
            if let Some(downloads) = &lib.downloads
                && let Some(classifiers) = &downloads.classifiers
            {
                for art in classifiers.values() {
                    live_libraries.insert(PathBuf::from(&art.path));
                }
            }
        }
        if let Some(index) = details.assetIndex {
            live_asset_indexes.insert(index.id);
        }
    }

    // 3. Referenced asset object hashes from the live asset indexes.
    let mut live_objects: HashSet<String> = HashSet::new();
    for index_id in &live_asset_indexes {
        let index_path = game_dir
            .join("assets")
            .join("indexes")
            .join(format!("{index_id}.json"));
        if let Ok(content) = fs::read_to_string(&index_path)
            && let Ok(index) = serde_json::from_str::<AssetIndex>(&content)
        {
            for obj in index.objects.values() {
                live_objects.insert(obj.hash.clone());
            }
        }
    }

    let mut scan = OrphanScan::default();

    // Orphan version directories.
    if let Ok(entries) = fs::read_dir(game_dir.join("versions")) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let id = entry.file_name().to_string_lossy().to_string();
            if !live_versions.contains(&id) {
                scan.total_bytes += dir_size(&entry.path());
                scan.versions.push(entry.path());
            }
        }
    }

    // Orphan library files (relative path not referenced by any live version).
    let libraries_dir = game_dir.join("libraries");
    collect_orphan_files(&libraries_dir, &libraries_dir, &mut |rel, full| {
        if !live_libraries.contains(rel) {
            scan.total_bytes += fs::metadata(full).map(|m| m.len()).unwrap_or(0);
            scan.libraries.push(full.to_path_buf());
        }
    });

    // Orphan asset objects (named by their sha1 hash).
    let objects_dir = game_dir.join("assets").join("objects");
    collect_orphan_files(&objects_dir, &objects_dir, &mut |_rel, full| {
        let hash = full
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or_default();
        if !live_objects.contains(hash) {
            scan.total_bytes += fs::metadata(full).map(|m| m.len()).unwrap_or(0);
            scan.assets.push(full.to_path_buf());
        }
    });

    scan
}

/// Recursively visit every file under `dir`, invoking `f` with its path relative
/// to `base` and its absolute path.
fn collect_orphan_files(base: &Path, dir: &Path, f: &mut impl FnMut(&Path, &Path)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            collect_orphan_files(base, &path, f);
        } else if let Ok(rel) = path.strip_prefix(base) {
            f(rel, &path);
        }
    }
}

/// Delete everything in `scan`, returning the number of bytes reclaimed. Removal
/// errors are ignored per-file so one locked file doesn't abort the whole sweep.
pub fn prune(scan: &OrphanScan) -> u64 {
    for dir in &scan.versions {
        let _ = fs::remove_dir_all(dir);
    }
    for file in scan.libraries.iter().chain(scan.assets.iter()) {
        let _ = fs::remove_file(file);
    }
    scan.total_bytes
}

/// Human-readable byte size, e.g. `1.4 GB`.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}
