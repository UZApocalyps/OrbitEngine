# Exporting your game

An export produces a game that players run **without installing anything**: a `.zip` for Windows, a `.tar.gz` for Linux, a folder to put on a website, or an `.apk` / `.aab` for Android.

## From the editor

Choose **File > Import, export and translations > Export game...** to open the Export panel.

1. **Export profile**: the game name, an **identifier** (the folder where players' saves are kept: lowercase letters, digits, `-` and `_`; do not change it after publishing), the version, an icon (a square PNG, 256x256 recommended; type its path or use **Browse...** to pick a PNG of the project), the default language (a code such as `fr`; empty means the system's) and the compression of the game pack. A **Web** option writes precompressed `.br` and `.gz` files next to the web files, and **3D quality (desktop)** (Low or High) sets the rendering quality of 3D scenes. Use **New** to add a profile and the delete button to remove one; **Save profiles** keeps them with the project. You can have several (for example `demo` and `release`) and pick the one to use before exporting; on the command line, `--profile <name>` does the same.
2. **Targets**: Windows, Linux, Web and Android (see below). A target whose player is not found is flagged; the **Templates** field points to another folder of player templates (empty: the `templates` folder next to the editor). A **Plugins** box appears when the project has Rust plugins (see *Rust plugins*).
3. **Output folder**: `export/` in the project by default.
4. **Export**: a progress bar and a log show each step and any warnings (unused assets, incomplete translations). **Cancel** stops it. The story is saved first, and errors in the Problems panel block the export.
5. When it is done, use **Open folder**, and for the web use **Test in the browser** (see *Hosting a web game*).

## What the player gets

- **Windows**: a single `.exe` with the game inside, and a short README. The icon of the profile becomes the icon of the `.exe` itself (in the file explorer and the taskbar), not only of the window.
- **Linux**: a single executable; a graphics driver (Vulkan or OpenGL) is needed.
- **Web**: a folder to upload as is to any static host (itch.io, GitHub Pages, Netlify...). The game loads scene by scene so it starts fast. Fullscreen is in the preferences or on F11. The game is saved when the tab is hidden.

Saves are stored under the game identifier in the player's user data folder (the browser storage for the web version).

## Hosting a web game

The web export is a folder: `index.html`, the player (`orbit_player.js`, `orbit_player_bg.wasm`) and the game pack, plus a `HOSTING.txt` reminder.

- **It must be served over http(s).** Browsers refuse WebAssembly and file loading on `file://` pages, so opening `index.html` by double click shows a blank or stuck page.
- **To try it locally**, use **Test in the browser** in the Export panel: it starts a small local server (shown as *Local server: http://127.0.0.1:...*, with a **Stop the server** button) and opens your browser. From a terminal: `orbit export web --project my_game --out dist --serve` (add `--port <n>` and `--no-open` if needed), or serve an existing folder with `cargo xtask serve --dir dist` (default `http://127.0.0.1:8080`, stop with Ctrl+C).
- **To publish**, upload the whole folder to any static host (itch.io HTML5, GitHub Pages, Netlify, an S3 bucket, nginx...). The host must serve `.wasm` files as `application/wasm` (all common hosts do). It should answer HTTP Range requests for the game pack, which lets the game load scene by scene; a server that ignores them still works, but downloads the whole game once.
- The precompressed `.br` and `.gz` files save bandwidth on hosts that serve them for `Accept-Encoding` (nginx `brotli_static` / `gzip_static`, Caddy...); on other hosts you can delete them, or untick the profile option.
- A web game always uses the web quality profile for 3D.

## Android

Android is an export target like the others: tick **Android** in the Targets of the Export panel and an **Android** section appears with the options below. The game player has an Android version with touch controls, and the game is saved automatically when the app is paused.

- **Format**: an **APK** (an installable file, to test on a device or distribute yourself) or an **AAB** (the app bundle the Play Store asks for; it needs *bundletool*).
- **Application id**: the unique id of the app (`com.studio.game`). Left empty it is derived from the game identifier. Do not change it once published.
- **Version code**: a whole number that must grow with every store release. Left empty it is derived from the version (1.2.3 gives 10203).
- **Orientation**: landscape, portrait or automatic.
- **Signing**: an Android package must be signed. Either choose **your keystore** (the file and the key alias are remembered in the editor's preferences, never in the project; the **password is asked for every export** and never saved), or the **debug keystore** (the standard Android one, created if missing) to try the game on a device. The Play Store refuses the debug keystore, so keep your own keystore safe: you need it for every update of the game.
- The result is `<identifier>-<version>-android.apk` (or `.aab`) in the output folder. The game needs Android 8.0 (API 26) or later.

**Tools to install.** Nothing is downloaded for you. The editor looks for them in the usual places and lists what it found in the **Tools found** box (use **Search the tools again** after an install); if one is missing, the export says what to install:

- the **Android SDK**, with its *build-tools* (`aapt2`, `zipalign`, `apksigner`) and an Android *platform*: install Android Studio, or the command-line tools. Set `ANDROID_HOME` if the SDK is not detected;
- a **JDK 17 or later** (`keytool`, `jarsigner`): the JDK bundled with Android Studio works. Set `JAVA_HOME` if it is not detected;
- **bundletool**, for an AAB only: set `BUNDLETOOL_JAR` to its `.jar`, or put it in the SDK or on the `PATH`.

The **NDK** is *not* needed to export a game: it is only used to build the Android player itself (`cargo xtask templates`). An export uses the precompiled Android player, shipped with the editor in its `templates/android` folder like the Windows, Linux and Web ones.

## Command line

The `orbit` tool does the same from a terminal. `orbit build --project my_game` validates the project and writes the game pack (`<identifier>.pak`) in `build/` to play it with `orbit-player --pak <file>`; `orbit export` writes the platform packages: `orbit export windows --project my_game --out dist`, also `linux`, `web`, `android` and `all` (the targets of the profile). For Android, sign with `--android-keystore <file> --android-alias <name>` (the password is read from the `ORBIT_ANDROID_KEYSTORE_PASSWORD` environment variable, never stored) or with `--android-debug` for tests; the format (APK or AAB) comes from the profile.
