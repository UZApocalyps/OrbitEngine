//! Detection of the Android packaging tools: SDK (build-tools, platform),
//! NDK, JDK and bundletool. Nothing is downloaded or installed: when a piece
//! is missing the error says what to install and where to get it.
//!
//! Where each piece is looked for, in order:
//!
//! * SDK: `ANDROID_HOME`, `ANDROID_SDK_ROOT`, then the usual install
//!   folders (`%LOCALAPPDATA%\Android\Sdk`, `~/Android/Sdk`,
//!   `~/Library/Android/sdk`);
//! * NDK (only needed to *build* the player template, see `cargo xtask
//!   templates`): `ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT`, `<sdk>/ndk/<latest>`;
//! * JDK: `JAVA_HOME`, `java` on the `PATH`, the JDK bundled with Android
//!   Studio, the usual JDK folders;
//! * bundletool (AAB only): `BUNDLETOOL_JAR`, `bundletool.jar` in the
//!   templates' `android` folder or in the SDK, `bundletool` on the `PATH`.

use std::path::{Path, PathBuf};

use crate::error::{ExportError, Result};

/// Reads an environment variable (injected so tests do not touch the real
/// environment).
pub type EnvFn<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The real environment.
pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// The JDK: a folder with `bin/`, or the tools found on the `PATH`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Jdk {
    pub home: Option<PathBuf>,
}

impl Jdk {
    /// Path of a JDK tool (`keytool`, `jarsigner`, `java`).
    pub fn tool(&self, name: &str) -> PathBuf {
        match &self.home {
            Some(home) => find_program(&home.join("bin"), name)
                .unwrap_or_else(|| home.join("bin").join(name)),
            None => PathBuf::from(name),
        }
    }
}

/// How bundletool is run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bundletool {
    /// `java -jar <jar>`.
    Jar(PathBuf),
    /// A `bundletool` launcher.
    Program(PathBuf),
}

/// Everything the packaging needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toolchain {
    pub sdk: PathBuf,
    pub build_tools: PathBuf,
    pub android_jar: PathBuf,
    pub jdk: Jdk,
    pub bundletool: Option<Bundletool>,
    pub ndk: Option<PathBuf>,
}

impl Toolchain {
    pub fn aapt2(&self) -> PathBuf {
        find_program(&self.build_tools, "aapt2").unwrap_or_else(|| self.build_tools.join("aapt2"))
    }
    pub fn zipalign(&self) -> PathBuf {
        find_program(&self.build_tools, "zipalign")
            .unwrap_or_else(|| self.build_tools.join("zipalign"))
    }
    pub fn apksigner(&self) -> PathBuf {
        find_program(&self.build_tools, "apksigner")
            .unwrap_or_else(|| self.build_tools.join("apksigner"))
    }
}

/// What was found, piece by piece (what the export panel shows).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Detected {
    pub sdk: Option<PathBuf>,
    /// Newest `build-tools/<version>` with `aapt2`, `zipalign` and `apksigner`.
    pub build_tools: Option<PathBuf>,
    /// `platforms/android-<n>/android.jar` (newest).
    pub android_jar: Option<PathBuf>,
    pub ndk: Option<PathBuf>,
    pub jdk: Option<Jdk>,
    pub bundletool: Option<Bundletool>,
}

/// File of a program in `dir`: `name`, `name.exe`, `name.bat` or `name.cmd`.
pub(crate) fn find_program(dir: &Path, name: &str) -> Option<PathBuf> {
    ["", ".exe", ".bat", ".cmd"]
        .iter()
        .map(|ext| dir.join(format!("{name}{ext}")))
        .find(|p| p.is_file())
}

/// `1.2.3` -> `[1, 2, 3]` for ordering version folders; non-numeric parts
/// count as 0 and a `-rc` style suffix sorts before the final version.
fn version_key(name: &str) -> (Vec<u64>, bool) {
    // `android-34` (platforms) sorts as 34.
    let name = name.trim_start_matches(|c: char| !c.is_ascii_digit());
    let (numbers, suffix) = match name.split_once('-') {
        Some((n, _)) => (n, true),
        None => (name, false),
    };
    (
        numbers
            .split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect(),
        !suffix,
    )
}

/// Sub-folders of `dir` ordered newest version first.
fn versions_desc(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| Some((e.file_name().to_str()?.to_owned(), e.path())))
        .collect();
    found.sort_by_key(|(name, _)| std::cmp::Reverse(version_key(name)));
    found.into_iter().map(|(_, p)| p).collect()
}

fn dirs_from_path(env: EnvFn<'_>) -> Vec<PathBuf> {
    env("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

fn sdk_candidates(env: EnvFn<'_>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|v| env(v))
        .map(PathBuf::from)
        .collect();
    if let Some(local) = env("LOCALAPPDATA") {
        out.push(PathBuf::from(local).join("Android").join("Sdk"));
    }
    if let Some(home) = env("USERPROFILE").or_else(|| env("HOME")) {
        let home = PathBuf::from(home);
        out.push(home.join("Android").join("Sdk"));
        out.push(home.join("Library").join("Android").join("sdk"));
        out.push(home.join("AppData/Local/Android/Sdk"));
    }
    out
}

fn jdk_candidates(env: EnvFn<'_>) -> Vec<Jdk> {
    let mut out = Vec::new();
    if let Some(home) = env("JAVA_HOME") {
        out.push(Jdk {
            home: Some(PathBuf::from(home)),
        });
    }
    // `java` on the PATH: its folder's parent is the JDK home when it has
    // `jarsigner`/`keytool` next to it, else the bare names are used.
    if dirs_from_path(env)
        .iter()
        .any(|d| find_program(d, "java").is_some())
    {
        out.push(Jdk { home: None });
    }
    let mut studio: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramW6432"] {
        if let Some(pf) = env(var) {
            studio.push(PathBuf::from(&pf).join("Android/Android Studio/jbr"));
            let java = PathBuf::from(&pf).join("Java");
            studio.extend(versions_desc(&java));
            let eclipse = PathBuf::from(&pf).join("Eclipse Adoptium");
            studio.extend(versions_desc(&eclipse));
        }
    }
    studio.extend(
        [
            "/opt/android-studio/jbr",
            "/usr/local/android-studio/jbr",
            "/Applications/Android Studio.app/Contents/jbr/Contents/Home",
            "/usr/lib/jvm/default-java",
        ]
        .map(PathBuf::from),
    );
    studio.extend(versions_desc(Path::new("/usr/lib/jvm")));
    out.extend(studio.into_iter().map(|home| Jdk { home: Some(home) }));
    out
}

fn is_jdk(jdk: &Jdk) -> bool {
    match &jdk.home {
        Some(home) => find_program(&home.join("bin"), "java").is_some(),
        None => true,
    }
}

/// Looks for the tools. `extra_dirs` are folders also searched for
/// `bundletool.jar` (the templates' `android` folders).
pub fn detect(env: EnvFn<'_>, extra_dirs: &[PathBuf]) -> Detected {
    let mut d = Detected {
        sdk: sdk_candidates(env)
            .into_iter()
            .find(|p| p.join("build-tools").is_dir() || p.join("platforms").is_dir()),
        ..Detected::default()
    };
    if let Some(sdk) = &d.sdk {
        d.build_tools = versions_desc(&sdk.join("build-tools")).into_iter().find(|bt| {
            ["aapt2", "zipalign", "apksigner"]
                .iter()
                .all(|t| find_program(bt, t).is_some())
        });
        d.android_jar = versions_desc(&sdk.join("platforms"))
            .into_iter()
            .map(|p| p.join("android.jar"))
            .find(|p| p.is_file());
    }
    let mut ndks: Vec<PathBuf> = ["ANDROID_NDK_HOME", "ANDROID_NDK_ROOT"]
        .iter()
        .filter_map(|v| env(v))
        .map(PathBuf::from)
        .collect();
    if let Some(sdk) = &d.sdk {
        ndks.extend(versions_desc(&sdk.join("ndk")));
        ndks.push(sdk.join("ndk-bundle"));
    }
    d.ndk = ndks.into_iter().find(|p| p.join("toolchains").is_dir());
    d.jdk = jdk_candidates(env).into_iter().find(is_jdk);
    d.bundletool = bundletool_candidates(env, extra_dirs, d.sdk.as_deref());
    d
}

fn bundletool_candidates(
    env: EnvFn<'_>,
    extra_dirs: &[PathBuf],
    sdk: Option<&Path>,
) -> Option<Bundletool> {
    if let Some(jar) = env("BUNDLETOOL_JAR").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(Bundletool::Jar(jar));
    }
    let jar_dirs = extra_dirs
        .iter()
        .cloned()
        .chain(sdk.map(Path::to_owned))
        .chain(sdk.map(|s| s.join("bundletool")));
    for dir in jar_dirs {
        let jar = dir.join("bundletool.jar");
        if jar.is_file() {
            return Some(Bundletool::Jar(jar));
        }
        // `bundletool-all-1.15.6.jar`
        if let Some(found) = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.extension().is_some_and(|e| e == "jar")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("bundletool"))
            })
        {
            return Some(Bundletool::Jar(found));
        }
    }
    dirs_from_path(env)
        .iter()
        .find_map(|d| find_program(d, "bundletool"))
        .map(Bundletool::Program)
}

/// The tools of `detected`, or an error naming everything missing, with
/// where to get it. `need_bundletool` is true for an AAB.
pub fn resolve(detected: &Detected, need_bundletool: bool) -> Result<Toolchain> {
    let mut missing: Vec<String> = Vec::new();
    if detected.sdk.is_none() {
        missing.push(
            "Android SDK not found: install it with Android Studio or the command-line tools \
             (https://developer.android.com/studio#command-line-tools-only), \
             then set ANDROID_HOME to its folder"
                .to_owned(),
        );
    } else {
        if detected.build_tools.is_none() {
            missing.push(
                "Android SDK build-tools (aapt2, zipalign, apksigner) not found: \
                 `sdkmanager \"build-tools;34.0.0\"`"
                    .to_owned(),
            );
        }
        if detected.android_jar.is_none() {
            missing.push(
                "Android SDK platform not found: `sdkmanager \"platforms;android-34\"`".to_owned(),
            );
        }
    }
    if detected.jdk.is_none() {
        missing.push(
            "JDK not found (apksigner, keytool and bundletool need Java 17 or newer): \
             install https://adoptium.net and set JAVA_HOME"
                .to_owned(),
        );
    }
    if need_bundletool && detected.bundletool.is_none() {
        missing.push(
            "bundletool not found (needed for an AAB): download bundletool-all.jar from \
             https://github.com/google/bundletool/releases and set BUNDLETOOL_JAR to it, \
             or export an APK instead"
                .to_owned(),
        );
    }
    if !missing.is_empty() {
        return Err(ExportError::Package(format!(
            "the Android tools needed to package the game are missing:\n{}\n\
             Nothing is downloaded automatically.",
            missing
                .iter()
                .map(|m| format!("  - {m}"))
                .collect::<Vec<_>>()
                .join("\n")
        )));
    }
    Ok(Toolchain {
        sdk: detected.sdk.clone().unwrap_or_default(),
        build_tools: detected.build_tools.clone().unwrap_or_default(),
        android_jar: detected.android_jar.clone().unwrap_or_default(),
        jdk: detected.jdk.clone().unwrap_or(Jdk { home: None }),
        bundletool: detected.bundletool.clone(),
        ndk: detected.ndk.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    fn fake_sdk(root: &Path) {
        for v in ["33.0.2", "34.0.0", "9.0.0"] {
            for tool in ["aapt2.exe", "zipalign.exe", "apksigner.bat"] {
                touch(&root.join("build-tools").join(v).join(tool));
            }
        }
        // An incomplete newer version is skipped.
        touch(&root.join("build-tools/35.0.0/aapt2"));
        touch(&root.join("platforms/android-33/android.jar"));
        touch(&root.join("platforms/android-34/android.jar"));
        touch(&root.join("ndk/26.1.10909125/toolchains/x"));
    }

    fn env_of<'a>(pairs: &'a [(&'a str, String)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn picks_the_newest_complete_tools() {
        let dir = TempDir::new("android_sdk");
        let sdk = dir.path().join("sdk");
        fake_sdk(&sdk);
        let jdk = dir.path().join("jdk");
        touch(&jdk.join("bin/java.exe"));
        let jar = dir.path().join("bundletool-all-1.15.6.jar");
        touch(&jar);
        let pairs = [
            ("ANDROID_HOME", sdk.to_string_lossy().into_owned()),
            ("JAVA_HOME", jdk.to_string_lossy().into_owned()),
            ("BUNDLETOOL_JAR", jar.to_string_lossy().into_owned()),
        ];
        let d = detect(&env_of(&pairs), &[]);
        assert_eq!(d.sdk.as_deref(), Some(sdk.as_path()));
        assert!(d.build_tools.unwrap().ends_with("34.0.0"));
        assert!(d.android_jar.unwrap().ends_with("android-34/android.jar"));
        assert!(d.ndk.is_some());
        assert_eq!(d.jdk.unwrap().home.as_deref(), Some(jdk.as_path()));
        assert_eq!(d.bundletool, Some(Bundletool::Jar(jar)));
    }

    #[test]
    fn nothing_installed_gives_a_message_listing_everything() {
        let dir = TempDir::new("android_none");
        let pairs = [("HOME", dir.path().to_string_lossy().into_owned())];
        let d = detect(&env_of(&pairs), &[]);
        assert!(d.sdk.is_none() && d.build_tools.is_none());
        let message = resolve(&d, true).unwrap_err().to_string();
        for needle in ["Android SDK not found", "JDK not found", "bundletool not found", "ANDROID_HOME"] {
            assert!(message.contains(needle), "{needle}: {message}");
        }
        // An APK does not need bundletool.
        let message = resolve(&d, false).unwrap_err().to_string();
        assert!(!message.contains("bundletool not found"), "{message}");
    }

    #[test]
    fn a_sdk_without_build_tools_says_which_package_to_install() {
        let dir = TempDir::new("android_partial");
        let sdk = dir.path().join("sdk");
        std::fs::create_dir_all(sdk.join("platforms")).unwrap();
        let d = Detected {
            sdk: Some(sdk),
            jdk: Some(Jdk { home: None }),
            ..Detected::default()
        };
        let message = resolve(&d, false).unwrap_err().to_string();
        assert!(message.contains("build-tools"), "{message}");
        assert!(message.contains("platform"), "{message}");
    }

    #[test]
    fn complete_detection_resolves_and_finds_the_program_files() {
        let dir = TempDir::new("android_ok");
        let sdk = dir.path().join("sdk");
        fake_sdk(&sdk);
        let d = Detected {
            sdk: Some(sdk.clone()),
            build_tools: Some(sdk.join("build-tools/34.0.0")),
            android_jar: Some(sdk.join("platforms/android-34/android.jar")),
            jdk: Some(Jdk { home: None }),
            ..Detected::default()
        };
        let t = resolve(&d, false).unwrap();
        assert!(t.apksigner().ends_with("apksigner.bat"));
        assert!(t.aapt2().ends_with("aapt2.exe"));
        assert_eq!(t.jdk.tool("keytool"), PathBuf::from("keytool"));
    }

    #[test]
    fn version_folders_sort_numerically_with_releases_first() {
        let dir = TempDir::new("android_versions");
        for v in ["9.0.0", "34.0.0", "34.0.0-rc1", "100.0.0"] {
            std::fs::create_dir_all(dir.path().join(v)).unwrap();
        }
        let names: Vec<String> = versions_desc(dir.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["100.0.0", "34.0.0", "34.0.0-rc1", "9.0.0"]);
    }
}
