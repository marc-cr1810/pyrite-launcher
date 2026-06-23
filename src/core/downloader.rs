use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use sha1::{Sha1, Digest};
use tokio::sync::mpsc::Sender;
use futures_util::StreamExt;
use reqwest::Client;

use crate::core::api::{VersionDetails, AssetIndex, Rule};

#[derive(Clone, Debug)]
pub enum ProgressUpdate {
    Started { total: usize, message: String },
    Progress { completed: usize, total: usize, current_file: String },
    Message(String),
    Finished,
    #[allow(dead_code)]
    Error(String),
}

pub struct Downloader {
    client: Client,
    progress_tx: Sender<ProgressUpdate>,
    /// Maximum number of asset files fetched in parallel.
    concurrency: usize,
}

/// Result of a check-only pass over an instance's game files. Counts files that
/// are missing or fail their SHA-1, so the caller can decide whether to repair
/// (re-download) and report exactly what was wrong.
#[derive(Default, Debug, Clone)]
pub struct VerifyReport {
    /// The client JAR is missing or corrupt.
    pub bad_client: bool,
    /// Number of library/native jars missing or corrupt.
    pub bad_libraries: usize,
    /// Number of asset objects missing or corrupt (includes a missing/unreadable
    /// asset index, counted as one).
    pub missing_assets: usize,
    /// Total asset objects examined (0 if the index couldn't be read).
    pub total_assets: usize,
}

impl VerifyReport {
    /// Total number of problem files found across all categories.
    pub fn total_bad(&self) -> usize {
        self.bad_client as usize + self.bad_libraries + self.missing_assets
    }

    /// True when nothing needs repairing.
    pub fn is_ok(&self) -> bool {
        self.total_bad() == 0
    }

    /// A short human summary like "client JAR, 3 libraries, 12 assets".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.bad_client {
            parts.push("client JAR".to_string());
        }
        if self.bad_libraries > 0 {
            parts.push(format!("{} librar{}", self.bad_libraries, if self.bad_libraries == 1 { "y" } else { "ies" }));
        }
        if self.missing_assets > 0 {
            parts.push(format!("{} asset{}", self.missing_assets, if self.missing_assets == 1 { "" } else { "s" }));
        }
        parts.join(", ")
    }
}

impl Downloader {
    pub fn new(progress_tx: Sender<ProgressUpdate>) -> Self {
        Self::with_concurrency(progress_tx, crate::core::config::default_download_concurrency())
    }

    /// Construct a downloader with an explicit parallel-download limit. Values
    /// below 1 are clamped to 1 so the asset stream always makes progress.
    pub fn with_concurrency(progress_tx: Sender<ProgressUpdate>, concurrency: usize) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap(),
            progress_tx,
            concurrency: concurrency.max(1),
        }
    }

    async fn send_progress(&self, completed: usize, total: usize, current_file: String) {
        let _ = self.progress_tx.send(ProgressUpdate::Progress {
            completed,
            total,
            current_file,
        }).await;
    }

    async fn send_message(&self, msg: String) {
        let _ = self.progress_tx.send(ProgressUpdate::Message(msg)).await;
    }

    async fn send_started(&self, total: usize, message: String) {
        let _ = self.progress_tx.send(ProgressUpdate::Started { total, message }).await;
    }

    pub fn verify_sha1(path: &Path, expected_sha1: &str) -> bool {
        if !path.exists() {
            return false;
        }
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let mut hasher = Sha1::new();
        let mut buffer = [0; 8192];
        loop {
            match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => hasher.update(&buffer[..n]),
                Err(_) => return false,
            }
        }
        let hex_result = format!("{:x}", hasher.finalize());
        hex_result.to_lowercase() == expected_sha1.to_lowercase()
    }

    pub async fn download_file(&self, url: &str, path: &Path, expected_sha1: &str) -> Result<(), String> {
        if expected_sha1.is_empty() {
            if path.exists() {
                return Ok(());
            }
        } else if Self::verify_sha1(path, expected_sha1) {
            return Ok(());
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create directories: {}", e))?;
        }

        let res = self.client.get(url)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !res.status().is_success() {
            return Err(format!("Failed to download {}, HTTP Status {}", url, res.status()));
        }

        let bytes = res.bytes().await.map_err(|e| format!("Failed to read body: {}", e))?;
        fs::write(path, &bytes).map_err(|e| format!("Failed to write file: {}", e))?;

        if !expected_sha1.is_empty() && !Self::verify_sha1(path, expected_sha1) {
            return Err(format!("SHA-1 mismatch for downloaded file: {}", path.display()));
        }

        Ok(())
    }

    pub fn extract_natives(&self, zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
        let file = File::open(zip_path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
            let file_name = file.name().to_string();

            if file_name.starts_with("META-INF/") || file_name.ends_with('/') {
                continue;
            }

            let outpath = dest_dir.join(&file_name);
            let ext = outpath.extension().and_then(|s| s.to_str()).unwrap_or("");
            if ext != "so" && ext != "dll" && ext != "dylib" && ext != "jnilib" {
                continue;
            }

            if let Some(p) = outpath.parent() {
                fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }

            let mut outfile = File::create(&outpath).map_err(|e| e.to_string())?;
            io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Check-only counterpart of [`download_version`]: re-hash the client JAR,
    /// libraries, and assets that *should* be present and report what's missing
    /// or corrupt, without touching the network. Recurses into `inheritsFrom`
    /// parents (Fabric/Forge profiles) just like the downloader does.
    pub fn verify_version(game_dir: &Path, version_details: &VersionDetails) -> VerifyReport {
        let mut report = VerifyReport::default();
        let version_id = version_details.id();
        let version_dir = game_dir.join("versions").join(&version_id);

        // Verify the parent profile first when this is a modded (inherited) one.
        if let Some(ref parent_id) = version_details.inheritsFrom {
            let parent_json = game_dir
                .join("versions")
                .join(parent_id)
                .join(format!("{parent_id}.json"));
            if let Ok(content) = fs::read_to_string(&parent_json)
                && let Ok(parent) = serde_json::from_str::<VersionDetails>(&content)
            {
                let pr = Self::verify_version(game_dir, &parent);
                report.bad_client |= pr.bad_client;
                report.bad_libraries += pr.bad_libraries;
                report.missing_assets += pr.missing_assets;
                report.total_assets += pr.total_assets;
            }
        }

        // 1. Client JAR.
        if let Some(ref downloads) = version_details.downloads {
            let client_jar = version_dir.join(format!("{version_id}.jar"));
            if !Self::verify_sha1(&client_jar, &downloads.client.sha1) {
                report.bad_client = true;
            }
        }

        // 2. Libraries (and native classifiers), filtered by the same OS rules
        //    the downloader applies.
        let libraries_dir = game_dir.join("libraries");
        let current_os = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "osx"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else {
            "unknown"
        };
        for lib in &version_details.libraries {
            if let Some(ref rules) = lib.rules
                && !Rule::evaluate(rules)
            {
                continue;
            }
            if let Some(art) = lib.get_artifact()
                && !art.sha1.is_empty()
                && !Self::verify_sha1(&libraries_dir.join(&art.path), &art.sha1)
            {
                report.bad_libraries += 1;
            }
            if let Some(ref natives_map) = lib.natives
                && let Some(classifier) = natives_map.get(current_os)
                && let Some(ref downloads) = lib.downloads
                && let Some(ref classifiers) = downloads.classifiers
                && let Some(art) = classifiers.get(classifier)
                && !art.sha1.is_empty()
                && !Self::verify_sha1(&libraries_dir.join(&art.path), &art.sha1)
            {
                report.bad_libraries += 1;
            }
        }

        // 3. Assets. A missing/unreadable index counts as one problem (repair
        //    re-fetches it and everything under it).
        if let Some(ref asset_index_ref) = version_details.assetIndex {
            let index_path = game_dir
                .join("assets")
                .join("indexes")
                .join(format!("{}.json", asset_index_ref.id));
            match fs::read_to_string(&index_path)
                .ok()
                .and_then(|c| serde_json::from_str::<AssetIndex>(&c).ok())
            {
                Some(asset_index) => {
                    let objects_dir = game_dir.join("assets").join("objects");
                    for obj in asset_index.objects.values() {
                        report.total_assets += 1;
                        let first_two = &obj.hash[0..2];
                        let path = objects_dir.join(first_two).join(&obj.hash);
                        if !Self::verify_sha1(&path, &obj.hash) {
                            report.missing_assets += 1;
                        }
                    }
                }
                None => report.missing_assets += 1,
            }
        }

        report
    }

    pub async fn download_version(&self, game_dir: &Path, version_details: &VersionDetails) -> Result<(), String> {
        let details_id = version_details.id();
        let version_id = &details_id;
        self.send_message(format!("Resolving files for Minecraft {}...", version_id)).await;

        let version_dir = game_dir.join("versions").join(version_id);
        fs::create_dir_all(&version_dir).map_err(|e| e.to_string())?;

        // 0. Ensure parent is downloaded first if inheritsFrom is present
        if let Some(ref parent_id) = version_details.inheritsFrom {
            let parent_json_path = game_dir
                .join("versions")
                .join(parent_id)
                .join(format!("{}.json", parent_id));

            if !parent_json_path.exists() {
                self.send_message(format!("Parent profile details missing for {}. Fetching details...", parent_id)).await;
                let api = crate::core::api::ApiClient::new();
                match api.fetch_version_manifest().await {
                    Ok(manifest) => {
                        if let Some(brief) = manifest.versions.iter().find(|v| &v.id == parent_id) {
                            match api.fetch_version_details(&brief.url).await {
                                Ok(parent_details) => {
                                    if let Some(parent) = parent_json_path.parent() {
                                        let _ = fs::create_dir_all(parent);
                                    }
                                    if let Ok(content) = serde_json::to_string_pretty(&parent_details) {
                                        let _ = fs::write(&parent_json_path, content);
                                    }
                                }
                                Err(e) => return Err(format!("Failed to fetch details for parent {}: {}", parent_id, e)),
                            }
                        } else {
                            return Err(format!("Parent Minecraft version '{}' not found in Mojang manifest.", parent_id));
                        }
                    }
                    Err(e) => return Err(format!("Failed to fetch version manifest for parent: {}", e)),
                }
            }

            if parent_json_path.exists()
                && let Ok(parent_content) = fs::read_to_string(&parent_json_path)
                    && let Ok(parent_details) = serde_json::from_str::<VersionDetails>(&parent_content) {
                        self.send_message(format!("Parent profile detected ({}). Ensuring parent is downloaded...", parent_id)).await;
                        Box::pin(self.download_version(game_dir, &parent_details)).await?;
                    }
        }

        // 1. Download Client JAR
        if let Some(ref downloads) = version_details.downloads {
            let client_jar_path = version_dir.join(format!("{}.jar", version_id));
            let client_art = &downloads.client;
            self.send_started(1, "Downloading client JAR...".to_string()).await;
            self.download_file(&client_art.url, &client_jar_path, &client_art.sha1).await?;
            self.send_progress(1, 1, "client.jar".to_string()).await;
        }

        // 2. Download Libraries
        let libraries_dir = game_dir.join("libraries");
        let natives_dir = version_dir.join("natives");

        let mut libs_to_download = Vec::new();
        let current_os = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "osx"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else {
            "unknown"
        };

        for lib in &version_details.libraries {
            if let Some(ref rules) = lib.rules
                && !Rule::evaluate(rules) {
                    continue;
                }

            if let Some(art) = lib.get_artifact() {
                libs_to_download.push((art, false));
            }

            if let Some(ref natives_map) = lib.natives
                && let Some(classifier) = natives_map.get(current_os)
                    && let Some(ref downloads) = lib.downloads
                        && let Some(ref classifiers) = downloads.classifiers
                            && let Some(art) = classifiers.get(classifier) {
                                libs_to_download.push((art.clone(), true));
                            }
        }

        let total_libs = libs_to_download.len();
        self.send_started(total_libs, "Downloading game libraries...".to_string()).await;

        for (idx, (art, is_native)) in libs_to_download.into_iter().enumerate() {
            let lib_path = libraries_dir.join(&art.path);
            let filename = lib_path.file_name().and_then(|f| f.to_str()).unwrap_or("library").to_string();

            self.download_file(&art.url, &lib_path, &art.sha1).await?;

            if is_native {
                self.send_message(format!("Extracting natives from {}...", filename)).await;
                if let Err(e) = self.extract_natives(&lib_path, &natives_dir) {
                    self.send_message(format!("Warning: failed to extract natives for {}: {}", filename, e)).await;
                }
            }

            self.send_progress(idx + 1, total_libs, filename).await;
        }

        // 3. Download Assets
        if let Some(ref asset_index_ref) = version_details.assetIndex {
            let index_dir = game_dir.join("assets").join("indexes");
            fs::create_dir_all(&index_dir).map_err(|e| e.to_string())?;

            let index_path = index_dir.join(format!("{}.json", asset_index_ref.id));
            self.send_started(1, "Downloading asset index...".to_string()).await;
            self.download_file(&asset_index_ref.url, &index_path, &asset_index_ref.sha1).await?;
            self.send_progress(1, 1, format!("{}.json", asset_index_ref.id)).await;

            let index_content = fs::read_to_string(&index_path).map_err(|e| e.to_string())?;
            let asset_index: AssetIndex = serde_json::from_str(&index_content)
                .map_err(|e| format!("Failed to parse asset index: {}", e))?;

            let objects_dir = game_dir.join("assets").join("objects");

            let mut assets_to_download = Vec::new();
            for (name, obj) in &asset_index.objects {
                let hash = &obj.hash;
                let first_two = &hash[0..2];
                let url = format!("https://resources.download.minecraft.net/{}/{}", first_two, hash);
                let path = objects_dir.join(first_two).join(hash);
                assets_to_download.push((url, path, hash.clone(), name.clone()));
            }

            let mut missing_assets = Vec::new();
            self.send_message("Verifying existing assets...".to_string()).await;
            for item in assets_to_download {
                if !Self::verify_sha1(&item.1, &item.2) {
                    missing_assets.push(item);
                }
            }

            let total_assets = missing_assets.len();
            if total_assets > 0 {
                self.send_started(total_assets, "Downloading missing assets...".to_string()).await;

                let client = self.client.clone();
                let completed = Arc::new(AtomicUsize::new(0));
                let progress_tx = self.progress_tx.clone();

                let futures = missing_assets.into_iter().map(|(url, path, sha1, name)| {
                    let client = client.clone();
                    let completed = completed.clone();
                    let progress_tx = progress_tx.clone();
                    async move {
                        let result = if let Some(parent) = path.parent() {
                            fs::create_dir_all(parent).map_err(|e| e.to_string())
                        } else {
                            Ok(())
                        };

                        if result.is_ok()
                            && let Ok(res) = client.get(&url).send().await
                                && res.status().is_success()
                                    && let Ok(bytes) = res.bytes().await
                                        && fs::write(&path, &bytes).is_ok() {
                                            let verified = Self::verify_sha1(&path, &sha1);
                                            if verified {
                                                let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                                                let filename = Path::new(&name).file_name()
                                                    .and_then(|f| f.to_str())
                                                    .unwrap_or("asset")
                                                    .to_string();

                                                let _ = progress_tx.send(ProgressUpdate::Progress {
                                                    completed: count,
                                                    total: total_assets,
                                                    current_file: filename,
                                                }).await;
                                                return Ok(());
                                            }
                                        }
                        Err(format!("Failed to download asset: {}", name))
                    }
                });

                let mut stream = futures_util::stream::iter(futures).buffer_unordered(self.concurrency);
                while let Some(res) = stream.next().await {
                    if let Err(e) = res {
                        self.send_message(format!("Warning: {}", e)).await;
                    }
                }
            }
        }

        self.send_message("All downloads completed successfully!".to_string()).await;
        let _ = self.progress_tx.send(ProgressUpdate::Finished).await;
        Ok(())
    }
}
