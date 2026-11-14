# DuneSaveManager

Save + character manager for **Dune: Awakening solo mode** (patch 1.5+). Written in Rust.

Two programs are built from this project:

* **`DuneSaveManager-gui`**: the graphical app (start here, double-click it).
* `DuneSaveManager`: the same features as a command line tool (documented below).

## GUI

* Left: the character currently in your live save (name, level, where they were read from), the game-running
  indicator, and the *Save snapshot* form (name, note, screenshot: newest Steam screenshot / pick a file / none).
* Right: a card per snapshot with screenshot thumbnail, character, level, date, size and note,
  with **Load**, **Screenshot** and **Delete** buttons. Load asks for confirmation and backs up your live save first.
* **Settings**: save location (auto-detected Steam path offered as a button; paste any other path), Steam account
  picker if several exist, vault folder, extra screenshot folders, and **Probe database** to inspect the save
  when name/level don't show up, plus SQL overrides for them.

Solo mode gives you one save slot per Steam account. This tool snapshots the whole slot under a name
(with character name, level and a screenshot), and lets you swap any snapshot back in, so you can keep
as many solo characters as you like and never lose one to a bad update or corruption.

## Getting the Windows .exe

Easiest: double-click **`build-windows.bat`**. It installs Rust if needed and builds
`target\release\DuneSaveManager-gui.exe` (first build takes a few minutes).
If it complains about a linker, install "Build Tools for Visual Studio" (workload: Desktop development with C++).

No local tooling at all: push this folder to a GitHub repo, open the **Actions** tab, run
**build-windows**, and download the `DuneSaveManager-windows` artifact (it contains both .exe files).

## Install / build

1. Install Rust: https://rustup.rs  (on Windows it also asks for the MSVC "C++ build tools"; accept that)
2. `cargo build --release`
3. Binaries: `target/release/DuneSaveManager-gui(.exe)` and `DuneSaveManager(.exe)`

Linux note: the GUI is built for X11 (works under Wayland via XWayland).

SQLite is bundled, nothing else to install.

## Command line quick start

```
DuneSaveManager detect                      # find your save folder (Steam default)
DuneSaveManager status                      # who is in the live save right now?
DuneSaveManager save atreides-run -n "pre-Arrakeen"
DuneSaveManager list
DuneSaveManager load harkonnen              # live save is auto-backed-up first
DuneSaveManager gallery --open              # HTML page with screenshots
```

Close the game before `save` / `load` (it rewrites the save every ~20 s; `--force` overrides).

## Save location

Default (Steam, Windows):
`%LOCALAPPDATA%\DuneSandbox\Saved\Cloud\PlayerClientStorage\FLS_retail\<steamid>\`

Auto-detected: Windows Steam, and Steam/Proton on Linux/Steam Deck (all Steam libraries).
Anything else (Game Pass, custom location):

```
DuneSaveManager config saves "D:\path\to\FLS_retail"   # or the <steamid> folder itself
DuneSaveManager config profile 7656119xxxxxxxxxx       # if several <steamid> folders exist
DuneSaveManager config vault "E:\DuneBackups"
DuneSaveManager config show
```
Per-run overrides: `--saves <path>` and `--vault <path>`.

## Screenshots

`save` attaches the newest image from the last 24 h (`--max-age-hours N`) found in:
Steam's screenshot folder for your account (press **F12** in-game), `%LOCALAPPDATA%\DuneSandbox\Saved\Screenshots`,
`Documents\DuneAwakening`, plus anything added with `config shots <dir>`.
Or pass `--screenshot file.png`, `--no-screenshot`, or `shot <snapshot> <file>` later.

## Character name / level

The solo DB layout isn't officially documented, so name/level are found heuristically
(tables/columns that look like character name and level, then `SOLO/Level.json`). `status` and `info` show
which column each value came from. If it's wrong or missing:

```
DuneSaveManager probe                       # lists candidate columns with sample values
DuneSaveManager config name-sql  "SELECT col FROM tbl WHERE ... LIMIT 1"
DuneSaveManager config level-sql "SELECT col FROM tbl WHERE ... LIMIT 1"
```

## Steam Cloud warning

Steam Cloud syncs this folder. When switching characters, turn it off for Dune
(Steam > Dune > Properties > General), or Steam may sync the wrong slot back over your loaded one.
The tool never modifies anything outside the save folder and its own vault.
