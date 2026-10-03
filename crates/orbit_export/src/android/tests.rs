//! Tests of the Android export: command lines, file layout, signing rules.
//! No Android tool is run: a [`FakeRunner`] records the commands and writes
//! the files the real tools would.

use std::io::{Read, Write};

use orbit_pak::WriteStats;
use orbit_project::{AndroidFormat, ExportProfile};

use super::toolchain::{Bundletool, Jdk};
use super::*;
use crate::report::Reporter;
use crate::templates::AndroidAbi;
use crate::testing::TempDir;

const PASSWORD: &str = "s3cret-pass";

#[derive(Default)]
struct FakeRunner {
    commands: Vec<Cmd>,
}

fn arg_after(cmd: &Cmd, flag: &str) -> PathBuf {
    let pos = cmd.args.iter().position(|a| a == flag).expect(flag);
    PathBuf::from(&cmd.args[pos + 1])
}

fn arg_prefixed(cmd: &Cmd, prefix: &str) -> PathBuf {
    cmd.args
        .iter()
        .find_map(|a| a.to_str()?.strip_prefix(prefix).map(PathBuf::from))
        .expect(prefix)
}

fn last_two(cmd: &Cmd) -> (PathBuf, PathBuf) {
    let n = cmd.args.len();
    (PathBuf::from(&cmd.args[n - 2]), PathBuf::from(&cmd.args[n - 1]))
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
    for (name, data) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

impl Runner for FakeRunner {
    fn run(&mut self, cmd: &Cmd) -> Result<()> {
        match cmd.label {
            "aapt2 compile" => std::fs::write(arg_after(cmd, "-o"), b"compiled").unwrap(),
            "aapt2 link" => {
                let proto = cmd.args.iter().any(|a| a == "--proto-format");
                let resources: &str = if proto { "resources.pb" } else { "resources.arsc" };
                write_zip(
                    &arg_after(cmd, "-o"),
                    &[
                        ("AndroidManifest.xml", b"binary manifest"),
                        (resources, b"table"),
                        ("res/mipmap-xxxhdpi/ic_launcher.png", b"png"),
                    ],
                );
            }
            "zipalign" => {
                let (from, to) = last_two(cmd);
                std::fs::copy(from, to).unwrap();
            }
            "apksigner sign" => {
                let out = arg_after(cmd, "--out");
                let input = PathBuf::from(cmd.args.last().unwrap());
                std::fs::copy(input, out).unwrap();
            }
            "bundletool" => {
                std::fs::copy(arg_prefixed(cmd, "--modules="), arg_prefixed(cmd, "--output="))
                    .unwrap();
            }
            "keytool" => std::fs::write(arg_after(cmd, "-keystore"), b"keystore").unwrap(),
            _ => {}
        }
        self.commands.push(cmd.clone());
        Ok(())
    }
}

struct Fixture {
    _dir: TempDir,
    out: PathBuf,
    setup: Setup,
    pak: BuiltPak,
    profile: ExportProfile,
    keystore: PathBuf,
}

fn fixture(name: &str, format: AndroidFormat, signing: impl Fn(&Path) -> Signing) -> Fixture {
    let dir = TempDir::new(name);
    let root = dir.path().to_owned();
    let tpl = root.join("templates/android");
    for abi in ["arm64-v8a", "x86_64"] {
        std::fs::create_dir_all(tpl.join(abi)).unwrap();
        std::fs::write(tpl.join(abi).join("liborbit_player.so"), abi.as_bytes()).unwrap();
    }
    std::fs::write(tpl.join("classes.dex"), b"dex data").unwrap();
    let keystore = root.join("my.keystore");
    std::fs::write(&keystore, b"ks").unwrap();
    let templates = Templates::new(vec![root.join("templates")]);
    let mut profile = ExportProfile::new("release", "Mon jeu");
    profile.version = "1.2.3".into();
    profile.android_format = format;
    let options = AndroidOptions {
        signing: signing(&keystore),
        toolchain: Some(Toolchain {
            sdk: root.join("sdk"),
            build_tools: root.join("sdk/build-tools/34.0.0"),
            android_jar: root.join("sdk/platforms/android-34/android.jar"),
            jdk: Jdk {
                home: Some(root.join("jdk")),
            },
            bundletool: Some(Bundletool::Jar(root.join("bundletool.jar"))),
            ndk: None,
        }),
        debug_keystore: Some(root.join("home/.android/debug.keystore")),
    };
    let setup = preflight(&templates, &options, &profile).expect("preflight");
    Fixture {
        out: root.join("out"),
        _dir: dir,
        setup,
        pak: BuiltPak {
            bytes: (0..=255u8).cycle().take(10_000).collect(),
            stats: WriteStats {
                entries: 1,
                raw_bytes: 0,
                stored_bytes: 0,
                pak_bytes: 10_000,
            },
            crc: 0,
        },
        profile,
        keystore,
    }
}

fn keystore_signing(path: &Path) -> Signing {
    Signing::Keystore {
        path: path.to_owned(),
        alias: "release".into(),
        password: Secret::new(PASSWORD),
    }
}

fn read_zip(path: &Path) -> zip::ZipArchive<File> {
    zip::ZipArchive::new(File::open(path).unwrap()).unwrap()
}

fn entry(zip: &mut zip::ZipArchive<File>, name: &str) -> (zip::CompressionMethod, Vec<u8>) {
    let mut file = zip.by_name(name).unwrap_or_else(|_| panic!("entry {name}"));
    let method = file.compression();
    let mut data = Vec::new();
    file.read_to_end(&mut data).unwrap();
    (method, data)
}

fn names(zip: &mut zip::ZipArchive<File>) -> Vec<String> {
    (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_owned())
        .collect()
}

fn export_with(f: &Fixture, runner: &mut FakeRunner, icon: Option<&[u8]>) -> Written {
    export(
        &f.pak,
        icon,
        &f.profile,
        &f.setup,
        &f.out,
        runner,
        &mut Reporter::silent(),
    )
    .expect("export")
}

#[test]
fn apk_layout_commands_and_uncompressed_pak() {
    let f = fixture("android_apk", AndroidFormat::Apk, keystore_signing);
    let mut runner = FakeRunner::default();
    let written = export_with(&f, &mut runner, Some(b"png bytes"));
    assert!(written.output.ends_with("mon-jeu-1.2.3-android.apk"));
    assert!(written.output.is_file());

    let labels: Vec<&str> = runner.commands.iter().map(|c| c.label).collect();
    assert_eq!(
        labels,
        [
            "aapt2 compile",
            "aapt2 link",
            "zipalign",
            "apksigner sign",
            "apksigner verify"
        ]
    );
    // aapt2 link: manifest, android.jar, sdk levels, icon resources, binary format.
    let link = &runner.commands[1];
    let line = link.display();
    assert!(line.contains("link -o"), "{line}");
    assert!(line.contains("android.jar"), "{line}");
    assert!(line.contains("--min-sdk-version 26"), "{line}");
    assert!(line.contains("--target-sdk-version 34"), "{line}");
    assert!(line.contains("-R "), "{line}");
    assert!(!line.contains("--proto-format"), "{line}");
    // zipalign pages the libraries.
    let align = runner.commands[2].display();
    assert!(align.contains("-p -f 4"), "{align}");
    // apksigner: keystore and alias on the command line, the password on stdin only.
    let sign = &runner.commands[3];
    let line = sign.display();
    assert!(line.contains("--ks ") && line.contains("my.keystore"), "{line}");
    assert!(line.contains("--ks-key-alias release"), "{line}");
    assert!(line.contains("--ks-pass stdin"), "{line}");
    assert_eq!(sign.stdin.as_ref().map(Secret::expose), Some(PASSWORD));
    assert!(!format!("{:?}", runner.commands).contains(PASSWORD));
    assert!(sign.env.iter().any(|(k, _)| k == "JAVA_HOME"));

    // The package: stored pak identical to the input, stored libraries.
    let mut zip = read_zip(&written.output);
    let (pak_method, pak) = entry(&mut zip, "assets/game.pak");
    assert_eq!(pak_method, zip::CompressionMethod::Stored);
    assert_eq!(pak, f.pak.bytes);
    let (method, data) = entry(&mut zip, "lib/arm64-v8a/liborbit_player.so");
    assert_eq!((method, data.as_slice()), (zip::CompressionMethod::Stored, &b"arm64-v8a"[..]));
    assert!(names(&mut zip).contains(&"lib/x86_64/liborbit_player.so".to_owned()));
    assert_eq!(entry(&mut zip, "classes.dex").1, b"dex data");
    assert_eq!(entry(&mut zip, "AndroidManifest.xml").1, b"binary manifest");
    assert!(names(&mut zip).contains(&"resources.arsc".to_owned()));
    // The keystore was left alone and the work folder is gone.
    assert_eq!(std::fs::read(&f.keystore).unwrap(), b"ks");
    let leftovers: Vec<_> = std::fs::read_dir(&f.out)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers.len(), 1, "{leftovers:?}");
}

#[test]
fn the_manifest_given_to_aapt2_describes_the_game() {
    let f = fixture("android_manifest", AndroidFormat::Apk, keystore_signing);
    struct Capture(Option<String>);
    impl Runner for Capture {
        fn run(&mut self, cmd: &Cmd) -> Result<()> {
            if cmd.label == "aapt2 link" {
                self.0 = Some(std::fs::read_to_string(arg_after(cmd, "--manifest")).unwrap());
            }
            FakeRunner::default().run(cmd)
        }
    }
    let mut capture = Capture(None);
    export(
        &f.pak,
        None,
        &f.profile,
        &f.setup,
        &f.out,
        &mut capture,
        &mut Reporter::silent(),
    )
    .unwrap();
    let xml = capture.0.unwrap();
    assert!(xml.contains(r#"package="com.orbit.mon_jeu""#));
    assert!(xml.contains(r#"android:versionCode="10203""#));
    assert!(xml.contains(r#"android:label="Mon jeu""#));
    assert!(!xml.contains("android:icon"), "no icon in the profile");
}

#[test]
fn aab_module_bundle_config_and_signing() {
    let f = fixture("android_aab", AndroidFormat::Aab, keystore_signing);
    let mut runner = FakeRunner::default();
    let written = export_with(&f, &mut runner, None);
    assert!(written.output.ends_with("mon-jeu-1.2.3-android.aab"));
    let labels: Vec<&str> = runner.commands.iter().map(|c| c.label).collect();
    assert_eq!(labels, ["aapt2 link", "bundletool", "jarsigner"]);
    assert!(runner.commands[0].display().contains("--proto-format"));

    let bundletool = &runner.commands[1];
    let line = bundletool.display();
    assert!(line.contains("-jar"), "{line}");
    assert!(line.contains("build-bundle"), "{line}");
    assert!(line.contains("--modules="), "{line}");
    assert!(line.contains("--config="), "{line}");
    assert!(bundletool.program.to_string_lossy().contains("java"));

    // jarsigner gets the password through the environment, not the command line.
    let jarsigner = &runner.commands[2];
    assert!(!jarsigner.display().contains(PASSWORD));
    assert!(jarsigner.display().contains("-storepass:env"));
    assert_eq!(
        jarsigner
            .secret_env
            .iter()
            .map(|(k, v)| (k.as_str(), v.expose()))
            .collect::<Vec<_>>(),
        [(STOREPASS_ENV, PASSWORD)]
    );
    assert!(!format!("{:?}", runner.commands).contains(PASSWORD));

    // Module layout (the fake bundletool copied the module as the bundle).
    let mut zip = read_zip(&written.output);
    let listing = names(&mut zip);
    for expected in [
        "manifest/AndroidManifest.xml",
        "resources.pb",
        "dex/classes.dex",
        "lib/arm64-v8a/liborbit_player.so",
        "assets/game.pak",
    ] {
        assert!(listing.contains(&expected.to_owned()), "{expected}: {listing:?}");
    }
    assert!(!listing.contains(&"AndroidManifest.xml".to_owned()));
    assert_eq!(
        entry(&mut zip, "assets/game.pak").0,
        zip::CompressionMethod::Stored
    );
}

#[test]
fn the_bundle_config_keeps_the_pak_uncompressed() {
    let config: serde_json::Value = serde_json::from_str(&bundle_config()).unwrap();
    assert_eq!(
        config["compression"]["uncompressedGlob"],
        serde_json::json!(["assets/game.pak"])
    );
}

#[test]
fn debug_signing_creates_the_debug_keystore_with_the_public_password() {
    let f = fixture("android_debug", AndroidFormat::Apk, |_| Signing::Debug);
    assert!(f.setup.key.create);
    let mut runner = FakeRunner::default();
    export_with(&f, &mut runner, None);
    let labels: Vec<&str> = runner.commands.iter().map(|c| c.label).collect();
    assert_eq!(labels[..2], ["aapt2 link", "keytool"]);
    let keytool = runner.commands[1].display();
    assert!(keytool.contains("-genkeypair"), "{keytool}");
    assert!(keytool.contains("debug.keystore"), "{keytool}");
    assert!(keytool.contains("-alias androiddebugkey"), "{keytool}");
    assert!(f.out.parent().unwrap().join("home/.android/debug.keystore").is_file());
    // A second export reuses it.
    let options = AndroidOptions {
        signing: Signing::Debug,
        toolchain: Some(f.setup.toolchain.clone()),
        debug_keystore: Some(f.setup.key.path.clone()),
    };
    let templates = Templates::new(vec![f.out.parent().unwrap().join("templates")]);
    let again = preflight(&templates, &options, &f.profile).unwrap();
    assert!(!again.key.create);
}

#[test]
fn signing_problems_are_refused_before_any_work() {
    let dir = TempDir::new("android_signing");
    let tpl = dir.path().join("templates/android");
    std::fs::create_dir_all(tpl.join("arm64-v8a")).unwrap();
    std::fs::write(tpl.join("arm64-v8a/liborbit_player.so"), b"so").unwrap();
    std::fs::write(tpl.join("classes.dex"), b"dex").unwrap();
    let templates = Templates::new(vec![dir.path().join("templates")]);
    let profile = ExportProfile::new("release", "G");
    let toolchain = Some(Toolchain {
        sdk: PathBuf::new(),
        build_tools: PathBuf::new(),
        android_jar: PathBuf::new(),
        jdk: Jdk { home: None },
        bundletool: None,
        ndk: None,
    });
    let check = |signing: Signing| {
        preflight(
            &templates,
            &AndroidOptions {
                signing,
                toolchain: toolchain.clone(),
                debug_keystore: None,
            },
            &profile,
        )
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default()
    };
    assert!(check(Signing::Unset).contains("choose how to sign"));
    let keystore = dir.path().join("k.jks");
    let with = |path: &Path, alias: &str, password: &str| Signing::Keystore {
        path: path.to_owned(),
        alias: alias.into(),
        password: Secret::new(password),
    };
    assert!(check(with(&keystore, "a", "p")).contains("does not exist"));
    std::fs::write(&keystore, b"k").unwrap();
    assert!(check(with(&keystore, "a", "")).contains("password is needed"));
    assert!(check(with(&keystore, " ", "p")).contains("alias"));
    assert!(check(with(&keystore, "a", "p")).is_empty());
    // An AAB needs bundletool.
    let mut aab = profile.clone();
    aab.android_format = AndroidFormat::Aab;
    let err = preflight(
        &templates,
        &AndroidOptions {
            signing: with(&keystore, "a", "p"),
            toolchain,
            debug_keystore: None,
        },
        &aab,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(err.contains("bundletool"), "{err}");
}

#[test]
fn a_missing_template_is_reported_with_its_search_path() {
    let dir = TempDir::new("android_notemplate");
    let templates = Templates::new(vec![dir.path().to_owned()]);
    let err = preflight(
        &templates,
        &AndroidOptions::default(),
        &ExportProfile::new("r", "G"),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(err.contains("no player template for android"), "{err}");
    assert!(err.contains("cargo xtask templates"), "{err}");
}

#[test]
fn secrets_never_print() {
    let options = AndroidOptions {
        signing: keystore_signing(Path::new("k")),
        ..AndroidOptions::default()
    };
    assert!(!format!("{options:?}").contains(PASSWORD));
    assert_eq!(format!("{:?}", Secret::new("x")), "Secret(<hidden>)");
}

#[test]
fn abi_names_and_triples() {
    assert_eq!(AndroidAbi::Arm64.id(), "arm64-v8a");
    assert_eq!(AndroidAbi::Arm64.triple(), "aarch64-linux-android");
    assert_eq!(AndroidAbi::Arm32.triple(), "armv7-linux-androideabi");
}

#[test]
fn a_failing_tool_reports_its_name_and_output() {
    let mut runner = SystemRunner;
    let missing = Cmd::new("aapt2 link", "definitely-not-an-installed-android-tool-xyz");
    let err = runner.run(&missing).unwrap_err().to_string();
    assert!(err.contains("aapt2 link"), "{err}");
    assert!(err.contains("does not exist"), "{err}");
}
