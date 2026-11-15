# DuneSaveManager

Save + character manager for **Dune: Awakening solo mode** (patch 1.5+). Written in Rust.

Two programs are built from this project:

* **`DuneSaveManager`**: the graphical app (start here, double-click it).
* `DuneSaveManagerCLI`: the same features as a command line tool (documented below).

## GUI

* Left: the character currently in your live save (name, level, where they were read from), the game-running
  indicator, and the *Save snapshot* form (name, note, screenshot: automatic / pick a file / none).
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
`target\release\DuneSaveManager.exe` (first build takes a few minutes).
If it complains about a linker, install "Build Tools for Visual Studio" (workload: Desktop development with C++).

No local tooling at all: push this folder to a GitHub repo, open the **Actions** tab, run
**build-windows**, and download the `DuneSaveManager-windows` artifact (it contains both .exe files).

## Install / build

1. Install Rust: https://rustup.rs  (on Windows it also asks for the MSVC "C++ build tools"; accept that)
2. `cargo build --release`
3. Binaries: `target/release/DuneSaveManager(.exe)` (GUI) and `DuneSaveManagerCLI(.exe)` (command line)

Linux note: the GUI is built for X11 (works under Wayland via XWayland).

SQLite is bundled, nothing else to install.

## Command line quick start

```
DuneSaveManagerCLI detect                      # find your save folder (Steam default)
DuneSaveManagerCLI status                      # who is in the live save right now?
DuneSaveManagerCLI save atreides-run -n "pre-Arrakeen"
DuneSaveManagerCLI list
DuneSaveManagerCLI load harkonnen              # live save is auto-backed-up first
DuneSaveManagerCLI gallery --open              # HTML page with screenshots
```

**Saving while the game runs is supported.** The game writes `game.db` every 60 s and logs
`SaveDatabase - Database saved to disk` in `%LOCALAPPDATA%\DuneSandbox\Saved\Logs\DuneSandbox.log`. `save` reads
that log and only copies when the last database save was less than 45 s ago (or more than 75 s ago, i.e. saving has
stalled); otherwise it waits. After copying it checks that no database save landed meanwhile and that the copy is a
valid database, and retries if not. `--force` skips the waiting (risky). The log location can be set in Settings.

**Loading requires the game to be closed**, always.

## Save location

Default (Steam, Windows):
`%LOCALAPPDATA%\DuneSandbox\Saved\Cloud\PlayerClientStorage\FLS_retail\<steamid>\`

Auto-detected: Windows Steam, and Steam/Proton on Linux/Steam Deck (all Steam libraries).
Anything else (Game Pass, custom location):

```
DuneSaveManagerCLI config saves "D:\path\to\FLS_retail"   # or the <steamid> folder itself
DuneSaveManagerCLI config profile 7656119xxxxxxxxxx       # if several <steamid> folders exist
DuneSaveManagerCLI config vault "E:\DuneBackups"
DuneSaveManagerCLI config show
```
Per-run overrides: `--saves <path>` and `--vault <path>`.

## Screenshots

With the game running (Windows), *Automatic* captures the game window at the moment you press Save (borderless or
windowed mode works best; a minimized window or exclusive fullscreen can't be captured). If that fails, or the game
is closed, the newest Steam screenshot is used.

`save` attaches the newest image from the last 24 h (`--max-age-hours N`) found in:
Steam's screenshot folder for your account (press **F12** in-game), `%LOCALAPPDATA%\DuneSandbox\Saved\Screenshots`,
`Documents\DuneAwakening`, plus anything added with `config shots <dir>`.
Or pass `--screenshot file.png`, `--no-screenshot`, or `shot <snapshot> <file>` later.

## Character name / level

The solo DB layout isn't officially documented, so name/level are found heuristically
(tables/columns that look like character name and level, then `SOLO/Level.json`). `status` and `info` show
which column each value came from. If it's wrong or missing:

```
DuneSaveManagerCLI probe                       # lists candidate columns with sample values
DuneSaveManagerCLI config name-sql  "SELECT col FROM tbl WHERE ... LIMIT 1"
DuneSaveManagerCLI config level-sql "SELECT col FROM tbl WHERE ... LIMIT 1"
```

## Steam Cloud warning

Steam Cloud syncs this folder. When switching characters, turn it off for Dune
(Steam > Dune > Properties > General), or Steam may sync the wrong slot back over your loaded one.
The tool never modifies anything outside the save folder and its own vault.