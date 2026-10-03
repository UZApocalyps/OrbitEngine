//! `AndroidManifest.xml` generation and the identity values derived from the
//! export profile (application id, version code).

use orbit_project::{AndroidOrientation, ExportProfile, is_valid_android_package};

/// Lowest Android version the player supports (ADR 0017: Android 8.0).
pub const MIN_SDK: u32 = 26;
/// Android version the game is built against (Play Store policy).
pub const TARGET_SDK: u32 = 34;
/// Name of the native library of the player (`liborbit_player.so`), as the
/// GameActivity wants it (without `lib` and `.so`).
pub const LIB_NAME: &str = "orbit_player";
/// The activity class shipped in the template's `classes.dex`.
pub const ACTIVITY: &str = "com.google.androidgamesdk.GameActivity";
/// Resource name of the launcher icon (`res/mipmap-xxxhdpi/ic_launcher.png`).
pub const ICON_RESOURCE: &str = "@mipmap/ic_launcher";

/// What the manifest says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestInput {
    pub package: String,
    pub version_name: String,
    pub version_code: u32,
    pub label: String,
    pub orientation: AndroidOrientation,
    pub has_icon: bool,
}

impl ManifestInput {
    pub fn from_profile(profile: &ExportProfile, has_icon: bool) -> Self {
        Self {
            package: package_id(profile),
            version_name: profile.version.trim().to_owned(),
            version_code: version_code(profile),
            label: profile.game_name.trim().to_owned(),
            orientation: profile.android_orientation,
            has_icon,
        }
    }
}

/// The application id: the profile's, or `com.orbit.<game id>` (dashes of the
/// id become underscores, and a segment starting with a digit gets a `g`).
pub fn package_id(profile: &ExportProfile) -> String {
    if let Some(id) = profile
        .android_package
        .as_deref()
        .map(str::trim)
        .filter(|id| is_valid_android_package(id))
    {
        return id.to_owned();
    }
    let mut segment: String = profile
        .game_id
        .chars()
        .map(|c| if c == '-' { '_' } else { c })
        .collect();
    if !segment.starts_with(|c: char| c.is_ascii_alphabetic()) {
        segment.insert(0, 'g');
    }
    format!("com.orbit.{segment}")
}

/// The version code: the profile's, or `major * 10000 + minor * 100 + patch`
/// from the version text (`1.2.3` gives 10203; unreadable gives 1).
pub fn version_code(profile: &ExportProfile) -> u32 {
    if let Some(code) = profile.android_version_code.filter(|c| *c > 0) {
        return code;
    }
    let mut parts = profile
        .version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['.', '-', '+'])
        .map(|p| p.parse::<u32>().ok());
    let Some(major) = parts.next().flatten() else {
        return 1;
    };
    let minor = parts.next().flatten().unwrap_or(0).min(99);
    let patch = parts.next().flatten().unwrap_or(0).min(99);
    (major.min(200_000) * 10_000 + minor * 100 + patch).max(1)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn orientation_value(o: AndroidOrientation) -> &'static str {
    match o {
        AndroidOrientation::Landscape => "sensorLandscape",
        AndroidOrientation::Portrait => "sensorPortrait",
        AndroidOrientation::Sensor => "fullSensor",
    }
}

/// The text of `AndroidManifest.xml`.
pub fn manifest_xml(m: &ManifestInput) -> String {
    let label = escape(&m.label);
    let icon = if m.has_icon {
        format!("\n        android:icon=\"{ICON_RESOURCE}\"")
    } else {
        String::new()
    };
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="{package}"
    android:versionCode="{code}"
    android:versionName="{version}">
    <uses-sdk android:minSdkVersion="{MIN_SDK}" android:targetSdkVersion="{TARGET_SDK}" />
    <uses-feature android:glEsVersion="0x00030000" android:required="true" />
    <application
        android:label="{label}"{icon}
        android:hasCode="true"
        android:allowBackup="false"
        android:extractNativeLibs="false"
        android:theme="@android:style/Theme.NoTitleBar.Fullscreen">
        <activity
            android:name="{ACTIVITY}"
            android:label="{label}"
            android:exported="true"
            android:launchMode="singleTask"
            android:screenOrientation="{orientation}"
            android:configChanges="orientation|screenSize|screenLayout|keyboardHidden|keyboard|navigation|uiMode|smallestScreenSize|density">
            <meta-data android:name="android.app.lib_name" android:value="{LIB_NAME}" />
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
"#,
        package = escape(&m.package),
        code = m.version_code,
        version = escape(&m.version_name),
        orientation = orientation_value(m.orientation),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> ExportProfile {
        ExportProfile::new("release", "Mon jeu <&>")
    }

    #[test]
    fn package_ids_are_derived_or_taken_from_the_profile() {
        let mut p = profile();
        assert_eq!(package_id(&p), "com.orbit.mon_jeu");
        p.game_id = "2048-x".into();
        assert_eq!(package_id(&p), "com.orbit.g2048_x");
        p.android_package = Some(" fr.studio.game ".into());
        assert_eq!(package_id(&p), "fr.studio.game");
        p.android_package = Some("bad".into());
        assert_eq!(package_id(&p), "com.orbit.g2048_x");
    }

    #[test]
    fn version_codes() {
        let mut p = profile();
        assert_eq!(version_code(&p), 10_000);
        p.version = "v1.2.3-beta".into();
        assert_eq!(version_code(&p), 10_203);
        p.version = "demo".into();
        assert_eq!(version_code(&p), 1);
        p.version = "0.0.0".into();
        assert_eq!(version_code(&p), 1);
        p.android_version_code = Some(42);
        assert_eq!(version_code(&p), 42);
    }

    #[test]
    fn manifest_carries_identity_and_escapes_text() {
        let mut p = profile();
        p.version = "1.2.3".into();
        p.android_orientation = AndroidOrientation::Portrait;
        let xml = manifest_xml(&ManifestInput::from_profile(&p, true));
        assert!(xml.contains(r#"package="com.orbit.mon_jeu""#), "{xml}");
        assert!(xml.contains(r#"android:versionCode="10203""#));
        assert!(xml.contains(r#"android:versionName="1.2.3""#));
        assert!(xml.contains("Mon jeu &lt;&amp;&gt;"));
        assert!(!xml.contains("Mon jeu <&>"));
        assert!(xml.contains(r#"android:screenOrientation="sensorPortrait""#));
        assert!(xml.contains(r#"android:icon="@mipmap/ic_launcher""#));
        assert!(xml.contains(r#"android:minSdkVersion="26""#));
        assert!(xml.contains(r#"android:value="orbit_player""#));
        assert!(xml.contains("com.google.androidgamesdk.GameActivity"));
        let no_icon = manifest_xml(&ManifestInput::from_profile(&p, false));
        assert!(!no_icon.contains("android:icon"));
    }
}
