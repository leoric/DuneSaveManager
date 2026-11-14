use clap::{Parser, Subcommand};
use dune_save_manager::{bail, *};
use std::{fs, path::PathBuf};

// ───────────────────────── CLI ─────────────────────────

#[derive(Parser)]
#[command(name = "DuneSaveManager", version, about = "Save + character manager for Dune: Awakening solo mode")]
struct Cli {
    /// Use this save location for this run (the <steamid> folder containing game.db, or its FLS_retail parent)
    #[arg(long, global = true)]
    saves: Option<PathBuf>,
    /// Use this vault (backup) folder for this run
    #[arg(long, global = true)]
    vault: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Find Dune save folders (Steam on Windows, Steam/Proton on Linux)
    Detect,
    /// View or change settings
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// Show the character currently in your live save
    Status,
    /// Snapshot the live save under a name
    Save {
        /// Snapshot name; omitted = "<character>-NN"
        name: Option<String>,
        #[arg(short, long)]
        note: Option<String>,
        /// Attach this image instead of auto-picking a recent screenshot
        #[arg(short, long)]
        screenshot: Option<PathBuf>,
        /// Don't attach any screenshot
        #[arg(long)]
        no_screenshot: bool,
        /// Only auto-attach screenshots newer than this many hours
        #[arg(long, default_value_t = 24)]
        max_age_hours: u64,
        /// If the game is running, copy immediately instead of waiting for a safe moment after its
        /// database save (risky: the copy may be torn)
        #[arg(long)]
        force: bool,
    },
    /// List snapshots
    List {
        /// Include automatic pre-load backups
        #[arg(short, long)]
        all: bool,
    },
    /// Show details of a snapshot
    Info { name: String },
    /// Replace the live save with a snapshot (live save is auto-backed-up first)
    Load {
        name: String,
        #[arg(short, long)]
        yes: bool,
    },
    /// Delete a snapshot
    Delete {
        name: String,
        #[arg(short, long)]
        yes: bool,
    },
    /// Attach/replace a screenshot on an existing snapshot
    Shot { name: String, file: PathBuf },
    /// Generate an HTML gallery of snapshots (with screenshots)
    Gallery {
        #[arg(long)]
        open: bool,
    },
    /// Dump save database structure to help locate name/level columns
    Probe {
        /// Snapshot name; omit for the live save
        name: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print current config and resolved paths
    Show,
    /// Set the saves location (FLS_retail dir or the <steamid> dir)
    Saves { path: PathBuf },
    /// Pick which <steamid> folder to use when several exist
    Profile { id: String },
    /// Set the vault (backup) folder
    Vault { path: PathBuf },
    /// Add an extra folder to search for screenshots
    Shots { path: PathBuf },
    /// SQL returning the character name (one text value). Empty string clears it
    NameSql { sql: String },
    /// SQL returning the character level (one integer value). Empty string clears it
    LevelSql { sql: String },
    /// Forget all settings (back to auto-detect, Steam default)
    Reset,
}

fn resolve_live(cli: &Cli, cfg: &Config) -> Res<PathBuf> {
    dune_save_manager::resolve_live(&cli.saves, cfg)
}
fn vault_dir(cli: &Cli, cfg: &Config) -> Res<PathBuf> {
    dune_save_manager::vault_dir(&cli.vault, cfg)
}

// ───────────────────────── main ─────────────────────────

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Res<()> {
    let cli = Cli::parse();
    let mut cfg = load_config();

    match &cli.cmd {
        Cmd::Detect => {
            let found = fls_candidates();
            if found.is_empty() {
                println!("No Dune save folder auto-detected (looked for Steam on Windows and Steam/Proton on Linux).");
                println!("If you use another launcher, set it manually: config saves <path>");
            }
            for root in found {
                println!("Saves root: {}", root.display());
                for p in subdirs(&root) {
                    let has = p.join("game.db").exists();
                    println!("  profile {}  {}", p.file_name().unwrap().to_string_lossy(), if has { "(has game.db)" } else { "(no game.db)" });
                }
            }
        }
        Cmd::Config { action } => {
            match action {
                ConfigCmd::Show => {
                    println!("config file : {}", config_path()?.display());
                    println!("vault       : {}", vault_dir(&cli, &cfg)?.display());
                    println!("saves root  : {}", cfg.saves_root.as_ref().map(|p| p.display().to_string()).unwrap_or("(auto-detect, Steam)".into()));
                    println!("profile     : {}", cfg.profile.clone().unwrap_or("(auto)".into()));
                    println!("name sql    : {}", cfg.name_sql.clone().unwrap_or("(heuristic)".into()));
                    println!("level sql   : {}", cfg.level_sql.clone().unwrap_or("(heuristic)".into()));
                    match resolve_live(&cli, &cfg) {
                        Ok(p) => println!("live save   : {}", p.display()),
                        Err(e) => println!("live save   : unresolved ({e})"),
                    }
                    return Ok(());
                }
                ConfigCmd::Saves { path } => {
                    if !path.exists() {
                        bail!("{} does not exist", path.display());
                    }
                    cfg.saves_root = Some(path.clone());
                    cfg.profile = None;
                }
                ConfigCmd::Profile { id } => cfg.profile = Some(id.clone()),
                ConfigCmd::Vault { path } => cfg.vault = Some(path.clone()),
                ConfigCmd::Shots { path } => {
                    if !cfg.screenshot_dirs.contains(path) {
                        cfg.screenshot_dirs.push(path.clone());
                    }
                }
                ConfigCmd::NameSql { sql } => cfg.name_sql = Some(sql.clone()).filter(|s| !s.is_empty()),
                ConfigCmd::LevelSql { sql } => cfg.level_sql = Some(sql.clone()).filter(|s| !s.is_empty()),
                ConfigCmd::Reset => cfg = Config::default(),
            }
            save_config(&cfg)?;
            println!("saved.");
        }
        Cmd::Status => {
            let live = resolve_live(&cli, &cfg)?;
            let i = inspect(&live, &cfg)?;
            let (n, l) = fmt_char(&i);
            println!("Live save : {}", live.display());
            println!("Character : {n}  (from {})", i.name_src.as_deref().unwrap_or("not found"));
            println!("Level     : {l}  (from {})", i.level_src.as_deref().unwrap_or("not found"));
            if i.name.is_none() || i.level.is_none() {
                println!("\nName/level not found automatically. Run `probe` to see the database layout, then use `config name-sql` / `config level-sql`.");
            }
            if game_running() {
                println!("\nNote: the game appears to be running (it rewrites the save every ~20s).");
            }
        }
        Cmd::Save { name, note, screenshot, no_screenshot, max_age_hours, force } => {
            let live = resolve_live(&cli, &cfg)?;
            let vault = vault_dir(&cli, &cfg)?;
            let name = match name {
                Some(n) => n.trim().to_string(),
                None => suggest_name(&vault, inspect(&live, &cfg).ok().and_then(|i| i.name).as_deref()),
            };
            let name = &name;
            if !valid_name(name) {
                bail!("Name may only contain letters, digits, space, . _ -");
            }
            if name.starts_with("_auto-") {
                bail!("'_auto-' is reserved for automatic backups");
            }
            let mut tmp_shot: Option<std::path::PathBuf> = None;
            let shot = if *no_screenshot {
                Shot::None
            } else if let Some(f) = screenshot {
                Shot::File(f.clone())
            } else {
                let (shot, tmp, warn) = auto_shot(*max_age_hours);
                if let Some(w) = warn {
                    println!("{w}");
                }
                tmp_shot = tmp;
                shot
            };
            let res = safe_snapshot(&vault, name, &live, &cfg, note.clone(), shot, !*force, &mut |s| println!("{s}"));
            if let Some(t) = tmp_shot {
                let _ = fs::remove_file(t);
            }
            let m = res?;
            let (n, l) = fmt_char(&m.character);
            println!("Saved '{}'  [{} | level {} | {}]", m.name, n, l, mb(m.bytes));
            match &m.screenshot {
                Some(s) => println!("Screenshot attached: {s}"),
                None if !no_screenshot => println!("No recent screenshot found (take one in-game with F12, or use --screenshot <file>)."),
                None => {}
            }
        }
        Cmd::List { all } => {
            let vault = vault_dir(&cli, &cfg)?;
            let list: Vec<Meta> = all_meta(&vault).into_iter().filter(|m| *all || !m.name.starts_with("_auto-")).collect();
            if list.is_empty() {
                println!("No snapshots yet. Create one with: save <name>");
            } else {
                println!("{:<24} {:<22} {:>5}  {:<16} {:>9}  {}", "SNAPSHOT", "CHARACTER", "LVL", "SAVED", "SIZE", "SHOT");
                for m in list {
                    let (n, l) = fmt_char(&m.character);
                    println!("{:<24} {:<22} {:>5}  {:<16} {:>9}  {}", m.name, n, l, m.created, mb(m.bytes), if m.screenshot.is_some() { "yes" } else { "-" });
                }
            }
        }
        Cmd::Info { name } => {
            let vault = vault_dir(&cli, &cfg)?;
            let m = read_meta(&vault, name)?;
            let (n, l) = fmt_char(&m.character);
            println!("Snapshot  : {}", m.name);
            println!("Character : {n} (from {})", m.character.name_src.as_deref().unwrap_or("-"));
            println!("Level     : {l} (from {})", m.character.level_src.as_deref().unwrap_or("-"));
            println!("Saved     : {}", m.created);
            println!("Size      : {}", mb(m.bytes));
            println!("Profile   : {}", m.source_profile);
            if let Some(nt) = &m.note {
                println!("Note      : {nt}");
            }
            if let Some(s) = &m.screenshot {
                println!("Screenshot: {}", vault.join(&m.name).join(s).display());
            }
        }
        Cmd::Load { name, yes } => {
            let vault = vault_dir(&cli, &cfg)?;
            let m = read_meta(&vault, name)?;
            if game_running() {
                bail!("The game is running. Loading a snapshot is only possible while the game is closed.");
            }
            let live = resolve_live(&cli, &cfg)?;
            let (n, l) = fmt_char(&m.character);
            if !yes && !confirm(&format!("Replace live save with '{}' ({} / level {})? Current save is backed up first.", m.name, n, l)) {
                println!("Cancelled.");
                return Ok(());
            }
            let auto = load_snapshot(&vault, name, &live, &cfg)?;
            println!("Loaded '{name}'. Previous live save kept as '{auto}'.");
            println!("Reminder: Steam Cloud may overwrite local files when you launch. Disable Steam Cloud for Dune (Properties > General) while using multiple characters, or Steam can sync the wrong one back.");
        }
        Cmd::Delete { name, yes } => {
            let vault = vault_dir(&cli, &cfg)?;
            read_meta(&vault, name)?;
            if !yes && !confirm(&format!("Delete snapshot '{name}' permanently?")) {
                println!("Cancelled.");
                return Ok(());
            }
            fs::remove_dir_all(vault.join(name))?;
            println!("Deleted.");
        }
        Cmd::Shot { name, file } => {
            let vault = vault_dir(&cli, &cfg)?;
            let mut m = read_meta(&vault, name)?;
            m.screenshot = Some(attach_shot(&vault.join(name), file)?);
            fs::write(vault.join(name).join("meta.json"), serde_json::to_string_pretty(&m)?)?;
            println!("Screenshot attached to '{name}'.");
        }
        Cmd::Gallery { open } => {
            let vault = vault_dir(&cli, &cfg)?;
            let p = gallery(&vault)?;
            println!("Wrote {}", p.display());
            if *open {
                open_path(&p);
            }
        }
        Cmd::Probe { name } => {
            let dir = match name {
                Some(n) => vault_dir(&cli, &cfg)?.join(n).join("data"),
                None => resolve_live(&cli, &cfg)?,
            };
            println!("{}", probe_text(&dir)?);
            println!("When you find the right column:\n  config name-sql  \"SELECT col FROM tbl WHERE ... LIMIT 1\"\n  config level-sql \"SELECT col FROM tbl WHERE ... LIMIT 1\"");
        }
    }
    Ok(())
}