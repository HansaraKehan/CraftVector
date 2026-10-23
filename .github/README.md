# VectorCraft for Android (early preview)

<img width="2304" height="1440" alt="Screenshot_20261018_173204" src="https://github.com/user-attachments/assets/92ac85fe-0a0e-472c-9204-4a928c98e90a" />

An unofficial Android build of [VectorCraft](https://github.com/storytold/vectorcraft), the open-source vector
illustration app written in Rust by the ArtCraft team, a clean-room take on the Illustrator workflow. This build runs
it as a native Android app (egui + wgpu), not as a web page.

> **Early preview.** This is a first working port, not a finished product. See [Limitations](#limitations).

This repository does not contain VectorCraft's source. A GitHub Actions workflow downloads the upstream source,
adds a small Android layer, builds the APK and publishes it under [Releases](https://github.com/HansaraKehan/CraftVector/releases).

## Download

Get the latest `.apk` from the [Releases](https://github.com/HansaraKehan/CraftVector/releases) page, open it and allow the install when Android asks.
You need:

- an ARM64 phone or tablet (most devices from the last several years)
- Android 9 or newer
- a GPU with Vulkan

If an update won't install over an older build, uninstall the old one first.

### 📊 Application Downloads

| Application | Description | Total Downloads | Releases |
| :--- | :--- | :--- | :--- |
| **PhotoCraft** | Image editing app | [![Downloads](https://img.shields.io/github/downloads/HansaraKehan/CraftPhoto/total?style=flat-square&color=blue)](https://github.com/HansaraKehan/CraftPhoto/releases) | [View Releases](https://github.com/HansaraKehan/CraftPhoto/releases) |
| **LightCraft** | Light & theme utility | [![Downloads](https://img.shields.io/github/downloads/HansaraKehan/Craftlight/total?style=flat-square&color=orange)](https://github.com/HansaraKehan/Craftlight/releases) | [View Releases](https://github.com/HansaraKehan/Craftlight/releases) |
| **WordCraft** | Text utility app | [![Downloads](https://img.shields.io/github/downloads/HansaraKehan/CraftWord/total?style=flat-square&color=green)](https://github.com/HansaraKehan/CraftWord/releases) | [View Releases](https://github.com/HansaraKehan/CraftWord/releases) |
| **VectorCraft** | Vector graphic tool | [![Downloads](https://img.shields.io/github/downloads/HansaraKehan/CraftVector/total?style=flat-square&color=purple)](https://github.com/HansaraKehan/CraftVector/releases) | [View Releases](https://github.com/HansaraKehan/CraftVector/releases) |
| **PDFCraft** | PDF manipulation tool | [![Downloads](https://img.shields.io/github/downloads/HansaraKehan/CraftPdf/total?style=flat-square&color=red)](https://github.com/HansaraKehan/CraftPdf/releases) | [View Releases](https://github.com/HansaraKehan/CraftPdf/releases) |

## Using it

No storage permission is needed. File ▸ Open, Save As and Export use Android's own file picker, so you can choose
documents anywhere: internal storage, an SD card, or cloud apps such as Google Drive.

VectorCraft works on a private copy of the document you pick and writes every save back to the original location
within a couple of seconds. If the source is read-only (for example an email attachment), use **Save As** to choose a
place you can write to.

You can also open a file from another app: choose **VectorCraft** in the "Open with" list for `.svg`, `.ai`, `.eps`
and `.pdf` files.

Your preferences, libraries and Data Recovery copies are kept in the app's private folder. Links open in your browser;
the About and Home links include **Get the Android app / updates**, which opens this repository's Releases page.

## Limitations

- No system clipboard yet: copy and paste work inside VectorCraft, not with other apps.
- No printing.
- Commands that need a folder (such as relinking images to a folder) only work after you turn on
  Settings → Apps → VectorCraft → Permissions → Files → **Allow access to manage all files**.
- A hardware keyboard and mouse or pen are recommended; the interface is the desktop layout, not redesigned for small
  touch screens.
- Large documents and heavy effects can be slow on low-end devices.

## How builds are made

- The workflow in `.github/workflows/vectorcraft-android-release.yml` runs every day and builds when upstream has a new
  commit, or when the Android layer file changes.
- To build right away: **Actions → Build and release VectorCraft Android APK → Run workflow**. You can enter an
  upstream branch or tag, or tick *force* to rebuild a version that already has a release.
- To stay on one upstream version, add a repository variable named `UPSTREAM_REF` (Settings → Secrets and variables →
  Actions → Variables) set to a tag or commit.

The Android layer lives in `android-layer/vectorcraft-android-layer.tar.gz`. It adds an Android app crate
(`apps/vectorcraft-android`: app host, file pickers, link opener), a Java launcher activity, a patched `winit` for
hardware keyboard and mouse input, and an Android manifest. It changes VectorCraft's own code only to add the releases
link. If upstream changes in a way that breaks the layer, the build stops and the log says why.

## Signing

Builds are signed with a key the workflow creates and caches. For a permanent key, add the secrets
`ANDROID_KEYSTORE_B64` (a PKCS12 keystore, base64-encoded, with alias `vectorcraft`) and `ANDROID_KEYSTORE_PASSWORD`.
APKs signed with different keys can't update each other.

## Credits and licence

- VectorCraft is by the [ArtCraft](https://getartcraft.com/) team, under MIT or Apache-2.0.
- The Android approach follows the community PhotoCraft port,
  [SnowOwlLeader/photocraft-for-android](https://github.com/SnowOwlLeader/photocraft-for-android).
- `vendor/winit` is [winit](https://crates.io/crates/winit) 0.30.13 with a small local input patch (Apache-2.0).

This project is not affiliated with or supported by the VectorCraft or ArtCraft team.
