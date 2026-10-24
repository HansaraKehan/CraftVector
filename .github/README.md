<div align="center">

# 🖋️ VectorCraft · Android Pen & Touch Edition

**An open-source vector design editor on your Android tablet, built for a stylus.**

<img width="2304" height="1440" alt="Screenshot_20261018_173204" src="https://github.com/user-attachments/assets/92ac85fe-0a0e-472c-9204-4a928c98e90a" />

An unofficial native Android build (egui + wgpu) of [VectorCraft](https://github.com/storytold/vectorcraft). It runs full-screen with native touch, active pen, and keyboard/mouse support.

[What's new](WHATS-NEW.md) · [Get the app](#-get-the-app) · [Settings guide](#-settings-guide) · [App Suite Downloads](#-application-downloads)

</div>

---

## ✨ Highlights

| | Feature | What it does |
|---|---|---|
| 🖊️ | **Pen barrel buttons** | Press the button for one action, hold it for another. Both can be key combinations. |
| 🔁 | **Recent-tool switch** | Press the pen button once to jump to your previous tool, press again to come back. |
| ⌨️ | **Soft-key strip** | Undo, Redo, Ctrl, Alt, Shift, Enter, Esc and any shortcut you choose, always one tap away. Contracts when not needed. |
| 🔒 | **Latching modifiers** | Tap Ctrl/Alt/Shift once to hold it, tap again to release. Works from the pen button too. |
| ✋ | **Finger ⇄ Pen only** | One button switches modes. In Pen only mode, fingers never draw. |
| 🧭 | **Finger navigation** | In Pen only mode: one finger pans the canvas, two fingers pan, zoom, and rotate it. |
| 👆 | **Active-stylus mode** | On devices with an active stylus, a finger moves items and the pen draws. |
| 📜 | **Touch scrolling** | A quick vertical swipe scrolls panels and windows (Layers, Swatches, dialogs), not only the thin scroll bar. |
| 🔄 | **Update checker** | Checks for a new build at start, or on demand from the settings window. |
| ⚙️ | **Everything is switchable** | Each behavior has its own on/off switch in the Pen & Touch window. |

---

## 📲 Get the App
Get the latest `.apk` from the [Releases](https://github.com/HansaraKehan/CraftVector/releases) page, open it on your device and allow the
install when Android asks. You need:

- an ARM64 phone or tablet (most devices from the last several years)
- Android 9 or newer
- a GPU with Vulkan or OpenGL ES 3

If an update won't install over an older build, uninstall the old one first.

---

### 📊 Application Downloads

| Application | Description | Total Downloads | Releases |
| :--- | :--- | :--- | :--- |
| **PhotoCraft** | Image editing app | [![Downloads](https://img.shields.io/github/downloads/HansaraKehan/CraftPhoto/total?style=flat-square&color=blue)](https://github.com/HansaraKehan/CraftPhoto/releases) | [View Releases](https://github.com/HansaraKehan/CraftPhoto/releases) |
| **LightCraft** | Light & theme utility | [![Downloads](https://img.shields.io/github/downloads/HansaraKehan/Craftlight/total?style=flat-square&color=orange)](https://github.com/HansaraKehan/Craftlight/releases) | [View Releases](https://github.com/HansaraKehan/Craftlight/releases) |
| **WordCraft** | Text utility app | [![Downloads](https://img.shields.io/github/downloads/HansaraKehan/CraftWord/total?style=flat-square&color=green)](https://github.com/HansaraKehan/CraftWord/releases) | [View Releases](https://github.com/HansaraKehan/CraftWord/releases) |
| **VectorCraft** | Vector graphic tool | [![Downloads](https://img.shields.io/github/downloads/HansaraKehan/CraftVector/total?style=flat-square&color=purple)](https://github.com/HansaraKehan/CraftVector/releases) | [View Releases](https://github.com/HansaraKehan/CraftVector/releases) |
| **PDFCraft** | PDF manipulation tool | [![Downloads](https://img.shields.io/github/downloads/HansaraKehan/CraftPdf/total?style=flat-square&color=red)](https://github.com/HansaraKehan/CraftPdf/releases) | [View Releases](https://github.com/HansaraKehan/CraftPdf/releases) |

---

## 🚀 How Builds & Updates Work

- **Daily Auto-Updates**: Every day the workflow checks upstream `storytold/vectorcraft` for a new commit. When detected, it builds a new APK and publishes a release named `android-v<version>-<commit>-<layer>`.
- **Manual Trigger**: Open **Actions → Build and release Android APK → Run workflow**. You can enter an upstream branch or tag, or tick *force* to rebuild a version that already has a release.
- **In the app**: VectorCraft checks the Releases page when it starts and offers **Download**, **Remind me later** or **Skip this version**. **Pen & Touch → Check for updates now** checks on demand.
- **Staying Up to Date / Pinning Upstream**:
  - The layer follows upstream's newest code. If upstream changes the same lines the layer edits, the run stops with **"The Android patch no longer applies"**. Send the log and the layer gets ported again.
  - To freeze a known-good version, add a repository variable `UPSTREAM_REF` (Settings → Secrets and variables → Actions → Variables) set to an upstream commit hash or branch. Delete it to follow the latest upstream again.

---

## 🖐️ How it feels to use

### Pen button
| You do | Default action |
|---|---|
| **Press** pen button 1 | Switch to the previous tool. Press again to go back. |
| **Hold** pen button 1 | Holds **Space** (pan the canvas) for as long as you hold it. |
| Pen button 2 | Off by default. Press = `Ctrl+Z`, hold = `Alt` once enabled. |

Any action can be a single key (`V`), a combination (`Ctrl+Shift+Z`), a modifier latch (`Ctrl`), or the special **Switch between recent tools** (`@recent`). Each button also supports a double press and a triple press.

### Strip
```
« │ Finger │ ⚙ │ Undo │ Redo │ Ctrl │ Alt │ Shift │ Enter │ Esc │ …
```
* **«** contracts the strip; **»** brings it back. The Finger / Pen only button and the ⚙ gear always stay.
* The strip can sit at the **top** or the **bottom** of the screen.
* **Ctrl / Alt / Shift** latch: lit = held.
* Other keys are pressed while your finger is on them (hold Space to pan, quick tap for one press).
* Scrolls sideways when there are more keys than fit.

### Finger vs. Pen only
| Mode | Pen | One finger | Two fingers |
|---|---|---|---|
| **Finger** | Draws | Draws | Tool-dependent |
| **Pen only** | Draws | **Pans** the canvas | **Pan, zoom, rotate** the canvas |

In Pen only mode a **two-finger tap undoes** and a **three-finger tap redoes**. Fingers on panels, menus and windows always work normally in both modes.

---

## ⚙️ Settings guide

Tap the **⚙** in the strip to open the **Pen & Touch** window. Changes apply immediately.

| Setting | Meaning |
|---|---|
| **Device** | *With an active stylus* or *Without*. Asked at start-up (with **Don't ask again**). The finger-moves behaviour needs an active stylus. |
| **Pen button 1 / 2** | Enable, choose the press, double, triple and hold actions, set the hold delay. |
| **Input mode** | Finger or Pen only (same as the strip button). |
| **Two fingers zoom / rotate / pan** | Turn each canvas gesture on or off. |
| **Swipe to scroll panels** | A vertical swipe on a list row or slider scrolls the panel or window. |
| **Larger touch targets** | Slightly taller buttons, fields and rows, so a finger hits them. |
| **A finger moves the item** | Active-stylus devices: a finger borrows the Selection tool while it is down, the pen keeps drawing. |
| **Pen pointer** | A ring follows the pen so you see where it is. It can follow the Blob Brush or Eraser size. |
| **Soft keys** | Show/hide the strip, choose top or bottom; add, rename, remove or reset keys. Each key accepts a key combination. |
| **Check for updates now** | Looks for a newer build straight away. |

### Key combination cheat sheet
| Write | Result |
|---|---|
| `V` | Press V (Selection tool) |
| `Ctrl+Shift+Z` | Redo |
| `Ctrl` or `Alt+Shift` | Latch: first press holds, next press releases |
| `Space` | Pressed while the button is held |
| `@recent` | Switch between the last two tools |

`Ctrl` and `Cmd` are the same key on Android. You can also pick a key from the list of VectorCraft commands.

---

## 🔗 Links

The community-chat link of the original app has been replaced everywhere (Help menu, command palette, Home / About button) with the developer's GitHub: <https://github.com/HansaraKehan>.

---

## ℹ️ Status

This edition is under active testing. The code was written without a local compiler, so each build is the first real test — expect a few fix-up rounds. If the app stops, it shows a note with the reason the next time you open it. Bug reports with build logs or a short description of what happened are very welcome.

---

## 📜 Credits & Licence

- VectorCraft is © its authors, dual-licensed under **MIT** / **Apache-2.0** — see `LICENSE-MIT`, `LICENSE-APACHE`, and `ATTRIBUTION.md`.
- The Pen & Touch module is ported from the [PhotoCraft Android edition](https://github.com/HansaraKehan/CraftPhoto), which is based on [SnowOwlLeader/photocraft-for-android](https://github.com/SnowOwlLeader/photocraft-for-android).
- The Android layer and the Pen & Touch module are provided under the same terms. Licences are bundled inside the APK; see `ATTRIBUTION-ANDROID.md` in the built source for the third-party notice.

*This project is an unofficial community build and is not affiliated with or supported by the VectorCraft or ArtCraft team.*
