use std::path::{Path, PathBuf};
use std::fs;

#[derive(Debug, Clone)]
pub struct CrashAnalysis {
    pub title: String,
    pub description: String,
    pub possible_solutions: Vec<String>,
    /// Coarse category for UI accenting/grouping: "java", "memory", "graphics",
    /// "mod", or "unknown".
    pub category: String,
    /// Absolute path of the crash-report file that informed this analysis, if one
    /// was found. Lets the UI offer an "Open crash report" action.
    pub report_path: Option<String>,
    /// A few of the most relevant log/report lines, for the UI to show verbatim.
    pub excerpt: Vec<String>,
}

impl CrashAnalysis {
    fn new(
        category: &str,
        title: &str,
        description: String,
        solutions: Vec<String>,
    ) -> Self {
        Self {
            title: title.to_string(),
            description,
            possible_solutions: solutions,
            category: category.to_string(),
            report_path: None,
            excerpt: Vec::new(),
        }
    }
}

pub fn analyze_crash(instance_path: &Path, latest_log_content: &str) -> Option<CrashAnalysis> {
    // 1. Try to read the latest crash report
    let report_path = get_latest_crash_report(instance_path);
    let crash_report_content = report_path
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok());
    let report_path_str = report_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    // Combine contents for scanning, prioritizing the crash report if present
    let combined_content = if let Some(ref report) = crash_report_content {
        format!("{}\n\n=== LATEST LOG ===\n{}", report, latest_log_content)
    } else {
        latest_log_content.to_string()
    };

    let mut analysis = classify(&combined_content)?;
    analysis.report_path = report_path_str;
    analysis.excerpt = excerpt_for(&analysis.category, &combined_content);
    Some(analysis)
}

/// Match the combined crash text against known signatures, most specific first.
fn classify(content: &str) -> Option<CrashAnalysis> {
    // Signature A: Java version mismatch
    if content.contains("UnsupportedClassVersionError")
        || content.contains("has been compiled by a more recent version of the Java Runtime")
    {
        return Some(CrashAnalysis::new(
            "java",
            "Incompatible Java Version",
            "The game exited because it was run with a JRE version that is older than what the game or one of your mods requires.".to_string(),
            vec![
                "Open the Settings tab and select or download a newer Java version (e.g. Java 25 or 21) under the Java Runtimes card.".to_string(),
                "Select 'Auto-detect (Recommended)' in Settings to automatically download and run the correct Java version for each instance.".to_string(),
                "If using a custom Java path, ensure it points to a compatible Java installation.".to_string(),
            ],
        ));
    }

    // Signature B: Out of Memory
    if content.contains("OutOfMemoryError")
        || content.contains("GC overhead limit exceeded")
    {
        return Some(CrashAnalysis::new(
            "memory",
            "Out of Memory (Heap Exhausted)",
            "The Java Virtual Machine ran out of allocated memory (RAM) while running the game.".to_string(),
            vec![
                "Increase the maximum memory for this instance in its Settings tab (e.g. set Max memory to 4096 or 6144 MB).".to_string(),
                "If you run shaders or large modpacks, 6–8 GB is often needed.".to_string(),
                "Close other memory-intensive applications running on your machine.".to_string(),
            ],
        ));
    }

    // Signature C: Graphics Driver OpenGL failure
    if content.contains("GLFW error 65542")
        || content.contains("WGL: The driver does not appear to support OpenGL")
        || content.contains("Pixel format not accelerated")
        || content.contains("Failed to create window")
        || content.contains("OpenGL 1.1 or higher")
    {
        return Some(CrashAnalysis::new(
            "graphics",
            "Graphics Driver / OpenGL Error",
            "Minecraft was unable to initialize OpenGL. This usually means the system lacks appropriate graphics drivers or hardware acceleration.".to_string(),
            vec![
                "Update your graphics card drivers (Intel, AMD, or NVIDIA) to the latest version.".to_string(),
                "If you are running in a virtual machine or remote desktop, ensure GPU acceleration is enabled and passed through.".to_string(),
            ],
        ));
    }

    // Signature D: Mixin apply failure (a mod's mixin couldn't transform a class —
    // usually an outdated mod against a newer/other mod or MC version).
    if content.contains("Mixin apply failed")
        || content.contains("MixinApplyError")
        || content.contains("InvalidMixinException")
        || content.contains("MixinTransformerError")
    {
        let mod_hint = find_mixin_owner(content);
        let description = match &mod_hint {
            Some(m) => format!("A mod's mixin failed to apply, which usually means an outdated or conflicting mod. The failing mixin belongs to: {m}"),
            None => "A mod's mixin failed to apply to a game class. This usually means a mod is outdated or conflicts with another mod or the Minecraft version.".to_string(),
        };
        return Some(CrashAnalysis::new(
            "mod",
            "Mod Mixin Failure",
            description,
            vec![
                "Update the mod named above (or all mods) to a build matching this Minecraft version.".to_string(),
                "Temporarily disable the offending mod from the Mods tab to confirm it's the cause.".to_string(),
            ],
        ));
    }

    // Signature E: Missing/incompatible mod dependency. Try to name the mod.
    for line in content.lines() {
        let l = line.trim();
        let is_dep = (l.contains("requires") && (l.contains("version") || l.contains("of mod") || l.contains("any version of")))
            || l.contains("is missing")
            || l.contains("Missing or unsupported mandatory dependencies")
            || l.contains("requires the following mods");
        if is_dep && (l.to_lowercase().contains("mod") || l.contains("dependenc")) {
            return Some(CrashAnalysis::new(
                "mod",
                "Missing Mod Dependency",
                format!("A required mod dependency was not satisfied:\n{l}"),
                vec![
                    "Install the dependency named in the message (search it in the Modrinth browser).".to_string(),
                    "Make sure every mod matches this instance's Minecraft version and loader.".to_string(),
                    "Use 'Check for updates' in the Mods tab to bring mods in line.".to_string(),
                ],
            ));
        }
    }

    // Signature F: Duplicate / incompatible mod set.
    if content.contains("Duplicate mods")
        || content.contains("found a duplicate mod")
        || content.contains("Incompatible mods found")
        || content.contains("incompatible mod set")
    {
        return Some(CrashAnalysis::new(
            "mod",
            "Duplicate or Incompatible Mods",
            "Two copies of the same mod, or mutually incompatible mods, were found in this instance.".to_string(),
            vec![
                "Open the Mods tab and remove duplicate or conflicting jars.".to_string(),
                "If you imported a modpack, avoid adding mods that it already bundles.".to_string(),
            ],
        ));
    }

    // Signature G: ClassNotFound / NoSuchMethod mod conflicts
    if content.contains("java.lang.ClassNotFoundException")
        || content.contains("java.lang.NoSuchMethodError")
        || content.contains("java.lang.NoClassDefFoundError")
        || content.contains("java.lang.NoSuchFieldError")
    {
        let context = content.lines()
            .find(|l| l.contains("ClassNotFoundException") || l.contains("NoSuchMethodError") || l.contains("NoClassDefFoundError") || l.contains("NoSuchFieldError"))
            .map(|l| l.trim())
            .unwrap_or("Mod linkage error detected");

        return Some(CrashAnalysis::new(
            "mod",
            "Mod Compatibility Conflict",
            format!("A mod referenced code that does not exist in the current runtime:\n{context}"),
            vec![
                "Update your mods so they match this loader and Minecraft version.".to_string(),
                "Remove or replace incompatible/outdated mods from the Mods tab.".to_string(),
            ],
        ));
    }

    // Signature H: Native library load failure.
    if content.contains("UnsatisfiedLinkError")
        || content.contains("Failed to locate library")
        || content.contains("no lwjgl")
    {
        return Some(CrashAnalysis::new(
            "graphics",
            "Native Library Load Failure",
            "A required native library (such as LWJGL) failed to load. The game files may be incomplete or corrupted.".to_string(),
            vec![
                "Press Play again — missing game files are re-downloaded automatically.".to_string(),
                "If it persists, delete this version under the game's 'versions' folder so it is re-fetched.".to_string(),
            ],
        ));
    }

    // General fallback: a crash report exists but matched no known signature.
    if content.contains("---- Minecraft Crash Report ----") {
        return Some(CrashAnalysis::new(
            "unknown",
            "Minecraft Crashed",
            "The game crashed and generated a crash report, but the cause didn't match a known pattern.".to_string(),
            vec![
                "Open the crash report below and read the 'Description' line near the top.".to_string(),
                "Check the game log for the first red error line.".to_string(),
            ],
        ));
    }

    None
}

/// Best-effort extraction of the mod that owns a failing mixin, e.g. from a
/// config name like "sodium.mixins.json" or "mixins.create.json".
fn find_mixin_owner(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(idx) = line.find(".mixins.json").or_else(|| line.find(".mixin.json")) {
            // Walk back to the start of the token holding the config filename.
            let prefix = &line[..idx];
            let start = prefix.rfind(|c: char| c == ' ' || c == '/' || c == '[' || c == '(').map(|i| i + 1).unwrap_or(0);
            let owner = prefix[start..].trim_start_matches("mixins.").trim();
            if !owner.is_empty() && owner.len() < 40 {
                return Some(owner.to_string());
            }
        }
    }
    None
}

/// Collect a handful of the most relevant lines to show verbatim under the
/// analysis, biased toward the signature's category.
fn excerpt_for(category: &str, content: &str) -> Vec<String> {
    let keywords: &[&str] = match category {
        "java" => &["UnsupportedClassVersionError", "compiled by a more recent"],
        "memory" => &["OutOfMemoryError", "GC overhead"],
        "graphics" => &["GLFW", "OpenGL", "Pixel format", "UnsatisfiedLinkError"],
        "mod" => &["Mixin", "requires", "ClassNotFoundException", "NoSuchMethodError", "NoClassDefFoundError", "Duplicate", "Incompatible"],
        _ => &["Exception", "Error", "Caused by"],
    };
    let mut out = Vec::new();
    for line in content.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if keywords.iter().any(|k| l.contains(k)) {
            out.push(l.chars().take(200).collect::<String>());
            if out.len() >= 4 {
                break;
            }
        }
    }
    out
}

fn get_latest_crash_report(instance_path: &Path) -> Option<PathBuf> {
    let reports_dir = instance_path.join("crash-reports");
    if !reports_dir.exists() {
        return None;
    }
    
    let entries = fs::read_dir(reports_dir).ok()?;
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_file() && e.path().extension().map(|ext| ext == "txt").unwrap_or(false))
        .map(|e| e.path())
        .collect();

    // Sort by modification time, newest first
    files.sort_by(|a, b| {
        let meta_a = a.metadata().and_then(|m| m.modified()).ok();
        let meta_b = b.metadata().and_then(|m| m.modified()).ok();
        meta_b.cmp(&meta_a)
    });

    files.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsupported_class_version() {
        let log = "java.lang.UnsupportedClassVersionError: net/minecraft/client/main/Main has been compiled by a more recent version of the Java Runtime";
        let path = Path::new("/tmp");
        let analysis = analyze_crash(path, log).unwrap();
        assert_eq!(analysis.title, "Incompatible Java Version");
    }

    #[test]
    fn test_out_of_memory() {
        let log = "java.lang.OutOfMemoryError: Java heap space\n\tat net.minecraft.client.renderer.texture.TextureAtlas.init(TextureAtlas.java:150)";
        let path = Path::new("/tmp");
        let analysis = analyze_crash(path, log).unwrap();
        assert_eq!(analysis.title, "Out of Memory (Heap Exhausted)");
        assert_eq!(analysis.category, "memory");
        assert!(!analysis.excerpt.is_empty());
    }

    #[test]
    fn test_mixin_failure_names_owner() {
        let log = "[main/ERROR]: Mixin apply failed sodium.mixins.json:features.SomeMixin";
        let analysis = analyze_crash(Path::new("/tmp"), log).unwrap();
        assert_eq!(analysis.title, "Mod Mixin Failure");
        assert_eq!(analysis.category, "mod");
        assert!(analysis.description.contains("sodium"));
    }

    #[test]
    fn test_missing_dependency() {
        let log = "Mod 'Some Mod' (somemod) 1.0 requires version 2.0 or later of mod fabric-api, which is missing!";
        let analysis = analyze_crash(Path::new("/tmp"), log).unwrap();
        assert_eq!(analysis.title, "Missing Mod Dependency");
        assert_eq!(analysis.category, "mod");
    }

    #[test]
    fn test_clean_log_no_analysis() {
        let log = "[main/INFO]: Stopping! Game closed normally.";
        assert!(analyze_crash(Path::new("/tmp"), log).is_none());
    }
}
