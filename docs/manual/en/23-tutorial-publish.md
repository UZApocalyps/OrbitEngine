# Tutorial: publish your game

Your game runs in the editor. This tutorial turns it into something other people can play: a Windows or Linux download, a web page you can put on itch.io or any website, or an Android app. It also shows how to test each result before you share it.

## Goal

Export the game for several platforms from one profile, test the web version on your own computer, and upload it to itch.io.

## Before you start

- A finished game with no red error in the **Problems** panel (errors block the export; warnings do not).
- A square PNG icon, 256x256 recommended, imported in your project's library (for example `assets/icon.png`).
- For Android only: the tools listed in the *Exporting your game* chapter (Android SDK, a JDK 17 or later and, for the Play Store, bundletool). Windows, Linux and web exports need nothing more than the editor.

## Steps

### 1. Open the Export panel

1. Choose **File > Import, export and translations > Export game…**. The **Export** panel opens.
2. The story is saved first if it has unsaved changes. A message reminds you of it.

### 2. Fill in the export profile

1. Under **Export profile**, write the **Game name**. It becomes the window title and the file names.
2. Choose the **Identifier**: lowercase letters, digits, `-` and `_`, for example `my-first-game`. It names the folder where players' saves live, so never change it after publishing: players would lose their saves.
3. Set the **Version** (for example `1.0.0`) and raise it for every release.
4. For the **Icon (PNG)**, click **Browse…** and pick your icon among the PNG files of the project.
5. Set the **Default language**: the language code (`en`, `fr`...) the game uses when the player has not chosen one. Left empty, it follows the player's system.
6. Click **Save profiles** to keep the profile in the project. You can create several profiles with **New** (for example `demo` and `release`) and select the one to export.

### 3. Choose the targets

Under **Targets**, tick what you want:

- **Windows (zip)**: one `.exe` with the game inside, in a `.zip`.
- **Linux (tar.gz)**: a single executable in a `.tar.gz`. The player's computer needs a graphics driver (Vulkan or OpenGL).
- **Web (navigateur)**: a folder to put on a website. Under **Web**, **Precompressed .br and .gz files** adds compressed copies for hosts that know how to serve them.
- **Android (APK / AAB)**: an Android section appears, see step 7.

A target followed by *(template not found)* has no player installed on this computer: the hint explains where to find it.

### 4. Export

1. Check the **Output folder** (`export/` in the project by default).
2. Click **Export**. A progress bar and the **Log** show every step and the warnings (unused assets, incomplete translations). **Cancel** stops the export.
3. When it is finished the panel lists one line per target, with the file and its size. Click **Open folder** to see the results, named after the identifier and version (for example `my-first-game-1.0.0-windows.zip`).

### 5. Test the web version locally

A web game cannot be opened by double-clicking `index.html`: browsers refuse WebAssembly on `file://` pages. Let the editor serve it:

1. After a web export, click **Test in the browser**.
2. The editor starts a small local server on a free port, shows **Local server:** with its address, and opens your browser.
3. Play, resize the window, press F11 for fullscreen. Close the tab, then click **Stop the server**.

From a terminal the same test is `orbit export web --project <project folder> --out <folder> --serve`.

### 6. Publish on itch.io

1. In the output folder, open the web folder (`<name>-<version>-web`). `index.html` must be directly inside it.
2. Select everything **inside** that folder (not the folder itself) and compress it into a `.zip`. If you ticked **Precompressed .br and .gz files**, you can delete the `.br` and `.gz` files, or export again with the box unticked: itch.io does not use them, and the archive gets smaller.
3. On itch.io create a new project. Set **Kind of project** to **HTML**, upload the `.zip` and tick **This file will be played in the browser**.
4. Set the viewport size (1280 by 720 suits most games), enable the fullscreen button, then save and look at the page. The game loads scene by scene, so it starts quickly.
5. For any other static host (GitHub Pages, Netlify, your own server), upload the whole web folder as it is. The file `HOSTING.txt` next to `index.html` lists what the server must do.

For a Windows or Linux release, upload the `.zip` or `.tar.gz` to itch.io as a downloadable file, tagged for its platform.

### 7. Publish on Android

1. Tick **Android (APK / AAB)**. In the **Android** section, choose the **Format**: **APK** is an installable file to test or distribute yourself; **AAB** is the bundle the Play Store requires.
2. Leave the **Application id** empty to derive it from the identifier, or set your own (`com.studio.game`). Never change it after publishing. The **Version code** must grow with every store release; empty, it is derived from the version.
3. Choose the **Orientation**: **Landscape**, **Portrait** or **Automatic**.
4. Under **Signing** choose **Debug keystore (tests)** to try the game on your own device, or **My keystore** with the keystore file, the key alias and the password, which is asked for every export and never saved. The Play Store refuses the debug keystore: keep your keystore safe, you need it for every update.
5. Look at **Tools found**. If something says *not found*, install it and click **Search the tools again**.
6. Export. Copy the `.apk` to your phone and open it (the phone will ask you to allow installing from this source), or upload the `.aab` to the Play Console.

## Result

One profile, one click, and your game exists as a `.zip` for Windows, a `.tar.gz` for Linux, a web folder tested locally and live on itch.io, and optionally an Android package.

## Going further

- Keep one profile per release channel (`demo` with fewer scenes or another icon, `release`), and export `all` from the command line: `orbit export all --project my_game --out dist`.
- The 3D quality of desktop games is set in the profile (**3D quality (desktop)**); web and Android games always use the lighter web profile.
- For an update, raise the **Version**, export again and upload the new files: players keep their saves as long as the identifier is the same.

## If something goes wrong

- **The Export button is blocked or the export fails at once**: open the **Problems** panel and fix the errors; warnings do not block.
- **"Choose at least one target"**: tick a target.
- **A target says (template not found)**: the player for that platform is not installed. Use the editor from its installer, or point **Templates** to the right folder.
- **The web page stays blank on my website**: it must be served over `http://` or `https://`, and `.wasm` files must be served as `application/wasm` (every common host does). Compare with **Test in the browser**.
- **The game starts slowly on a web host**: make sure the server answers HTTP Range requests for `game.pak`; without them the player downloads the whole file once.
- **Android export says a tool is missing**: read the *Tools found* box and the message of the log; set `ANDROID_HOME` or `JAVA_HOME` if a tool is installed but not detected.
- **Players lost their saves after an update**: the **Identifier** (or the Android **Application id**) was changed. Put the old value back.
