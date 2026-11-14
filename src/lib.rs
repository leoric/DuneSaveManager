//! Core library for DuneSaveManager: save + character manager for Dune: Awakening solo mode.
//!
//! Solo save layout (as documented by the community, patch 1.5+):
//!   %LOCALAPPDATA%\DuneSandbox\Saved\Cloud\PlayerClientStorage\FLS_retail\<steamid>\
//!     game.db        8-byte header + zlib stream containing a SQLite database
//!     autosave, game_prepatch.db, SOLO\ (Level.json, WornItems.json, fog of war ...)
//!
//! One Steam account = one solo slot, so this tool snapshots/swaps the whole
//! <steamid> folder to give you as many named characters as you like.

use rusqlite::{types::Value, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime},
};

pub type Res<T> = Result<T, Box<dyn std::error::Error>>;
#[macro_export]
macro_rules! bail {
    ($($t:tt)*) => { return Err(format!($($t)*).into()) };
}

pub const APP_ID: &str = "1172710";
pub const STEAM64_BASE: u64 = 76561197960265728;
pub const IMG_EXT: [&str; 5] = ["png", "jpg", "jpeg", "bmp", "webp"];

// ───────────────────────── config ─────────────────────────

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Config {
    pub saves_root: Option<PathBuf>,
    pub profile: Option<String>,
    pub vault: Option<PathBuf>,
    #[serde(default)]
    pub screenshot_dirs: Vec<PathBuf>,
    pub name_sql: Option<String>,
    pub level_sql: Option<String>,
}

pub fn config_path() -> Res<PathBuf> {
    Ok(dirs::config_dir().ok_or("no config dir")?.join("DuneSaveManager").join("config.json"))
}
pub fn load_config() -> Config {
    config_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
pub fn save_config(c: &Config) -> Res<()> {
    let p = config_path()?;
    fs::create_dir_all(p.parent().unwrap())?;
    fs::write(p, serde_json::to_string_pretty(c)?)?;
    Ok(())
}
pub fn vault_dir(vault: &Option<PathBuf>, cfg: &Config) -> Res<PathBuf> {
    let v = vault
        .clone()
        .or_else(|| cfg.vault.clone())
        .or_else(|| dirs::data_local_dir().map(|d| d.join("DuneSaveManager").join("vault")))
        .ok_or("cannot determine vault folder; pass --vault")?;
    fs::create_dir_all(&v)?;
    Ok(v)
}

// ───────────────────────── detection ─────────────────────────

pub fn steam_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = vec![];
    if cfg!(windows) {
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(p) = std::env::var_os(var) {
                v.push(PathBuf::from(p).join("Steam"));
            }
        }
        v.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    }
    if let Some(h) = dirs::home_dir() {
        for rel in [".steam/steam", ".local/share/Steam", ".var/app/com.valvesoftware.Steam/.local/share/Steam"] {
            v.push(h.join(rel));
        }
    }
    v.retain(|p| p.exists());
    v.dedup();
    v
}

pub fn steam_libraries() -> Vec<PathBuf> {
    let mut libs = steam_roots();
    let roots = libs.clone();
    for r in roots {
        if let Ok(txt) = fs::read_to_string(r.join("steamapps").join("libraryfolders.vdf")) {
            for line in txt.lines() {
                let t = line.trim();
                if t.starts_with("\"path\"") {
                    let parts: Vec<&str> = t.split('"').collect();
                    if parts.len() > 3 {
                        let p = PathBuf::from(parts[3].replace("\\\\", "\\"));
                        if p.exists() && !libs.contains(&p) {
                            libs.push(p);
                        }
                    }
                }
            }
        }
    }
    libs
}

pub fn fls_candidates() -> Vec<PathBuf> {
    let rel = ["DuneSandbox", "Saved", "Cloud", "PlayerClientStorage", "FLS_retail"];
    let mut out = vec![];
    if cfg!(windows) {
        if let Some(l) = dirs::data_local_dir() {
            out.push(rel.iter().fold(l, |p, s| p.join(s)));
        }
    }
    for lib in steam_libraries() {
        let mut p = lib
            .join("steamapps")
            .join("compatdata")
            .join(APP_ID)
            .join("pfx/drive_c/users/steamuser/AppData/Local");
        for s in rel {
            p.push(s);
        }
        out.push(p);
    }
    out.retain(|p| p.exists());
    out
}

pub fn subdirs(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(p)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

pub fn resolve_live(saves: &Option<PathBuf>, cfg: &Config) -> Res<PathBuf> {
    let root = saves
        .clone()
        .or_else(|| cfg.saves_root.clone())
        .or_else(|| fls_candidates().into_iter().next())
        .ok_or("No Dune save folder found. Run `detect`, or set it with `config saves <path>` / --saves")?;
    if root.join("game.db").exists() {
        return Ok(root);
    }
    if let Some(id) = &cfg.profile {
        let p = root.join(id);
        if p.exists() {
            return Ok(p);
        }
        bail!("Configured profile {} not found under {}", id, root.display());
    }
    let mut profs: Vec<PathBuf> = subdirs(&root).into_iter().filter(|p| p.join("game.db").exists()).collect();
    match profs.len() {
        0 => bail!("No game.db under {} (have you created a solo character yet?)", root.display()),
        1 => Ok(profs.remove(0)),
        _ => {
            let names: Vec<String> = profs.iter().map(|p| p.file_name().unwrap().to_string_lossy().into()).collect();
            bail!("Several profiles found ({}). Choose one: config profile <id>", names.join(", "))
        }
    }
}

/// Build a Command that never pops up a console window on Windows.
pub fn quiet_cmd(prog: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(prog);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

pub fn game_running() -> bool {
    if cfg!(windows) {
        quiet_cmd("tasklist")
            .args(["/FO", "CSV", "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("dunesandbox"))
            .unwrap_or(false)
    } else if let Ok(rd) = fs::read_dir("/proc") {
        rd.filter_map(|e| e.ok()).any(|e| {
            fs::read(e.path().join("cmdline"))
                .map(|b| String::from_utf8_lossy(&b).to_lowercase().contains("dunesandbox"))
                .unwrap_or(false)
        })
    } else {
        false
    }
}

// ───────────────────────── save database ─────────────────────────

/// Decompress game.db (8-byte header + zlib SQLite) into a temp file and open it.
pub struct TempDb {
    conn: Option<Connection>,
    path: PathBuf,
}
impl TempDb {
    fn open(game_db: &Path) -> Res<TempDb> {
        let bytes = fs::read(game_db)?;
        let raw = if bytes.starts_with(b"SQLite format 3\0") {
            bytes
        } else {
            if bytes.len() < 9 {
                bail!("game.db too small");
            }
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(&bytes[8..]).read_to_end(&mut out)?;
            if !out.starts_with(b"SQLite format 3\0") {
                bail!("game.db decompressed but is not SQLite (format changed?)");
            }
            out
        };
        let path = std::env::temp_dir().join(format!("DuneSaveManager-{}.db", std::process::id()));
        fs::write(&path, raw)?;
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        Ok(TempDb { conn: Some(conn), path })
    }
    fn c(&self) -> &Connection {
        self.conn.as_ref().unwrap()
    }
}
impl Drop for TempDb {
    fn drop(&mut self) {
        self.conn.take();
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let _ = fs::remove_file(format!("{}{}", self.path.display(), suffix));
        }
    }
}

pub fn qi(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub fn first_value(c: &Connection, sql: &str) -> Option<Value> {
    c.query_row(sql, [], |r| r.get::<_, Value>(0)).ok()
}
pub fn v_str(v: Value) -> Option<String> {
    match v {
        Value::Text(s) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}
pub fn v_int(v: Value) -> Option<i64> {
    match v {
        Value::Integer(i) => Some(i),
        Value::Real(f) => Some(f as i64),
        Value::Text(s) => s.trim().parse().ok(),
        _ => None,
    }
}
pub fn tables(c: &Connection) -> Vec<String> {
    let mut st = match c.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name") {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    st.query_map([], |r| r.get::<_, String>(0)).map(|it| it.filter_map(|x| x.ok()).collect()).unwrap_or_default()
}
pub fn columns(c: &Connection, t: &str) -> Vec<String> {
    let mut st = match c.prepare(&format!("PRAGMA table_info({})", qi(t))) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    st.query_map([], |r| r.get::<_, String>(1)).map(|it| it.filter_map(|x| x.ok()).collect()).unwrap_or_default()
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct CharInfo {
    pub name: Option<String>,
    pub level: Option<i64>,
    pub name_src: Option<String>,
    pub level_src: Option<String>,
}

pub fn table_score(t: &str) -> i32 {
    let l = t.to_lowercase();
    if ["faction", "guild", "landsraad", "permission", "simulated", "item", "inventor"].iter().any(|b| l.contains(b)) {
        return -1;
    }
    (l.contains("character") as i32) * 3 + (l.contains("player") as i32) * 2 + (l.contains("profile") as i32)
}

pub fn find_json_level(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::Object(m) => {
            for (k, val) in m {
                if k.to_lowercase().contains("level") {
                    if let Some(n) = val.as_i64().or_else(|| val.as_f64().map(|f| f as i64)) {
                        return Some(n);
                    }
                }
            }
            m.values().find_map(find_json_level)
        }
        serde_json::Value::Array(a) => a.iter().find_map(find_json_level),
        _ => None,
    }
}

// ───────────────── property-container decoder (character blobs) ─────────────────
// Format (reverse-engineered by community editors): tag byte, hi nibble = length (or length-of-length),
// lo nibble = type: 3 int, 5 float, 7 string, 8 long string (all decimal/UTF-8 text), 11 array, 12 struct,
// 1/2 bool. Structs are key,value,key,value...; arrays are values.

enum PNode {
    Scalar(String),
    Struct(Vec<(String, PNode)>),
    Array(Vec<PNode>),
}

fn p_len(b: &[u8], i: &mut usize, hi: usize) -> Option<usize> {
    Some(match hi {
        0..=11 => hi,
        12 => {
            let v = *b.get(*i)? as usize;
            *i += 1;
            v
        }
        13 => {
            let v = u16::from_be_bytes(b.get(*i..*i + 2)?.try_into().ok()?) as usize;
            *i += 2;
            v
        }
        14 => {
            let v = u32::from_be_bytes(b.get(*i..*i + 4)?.try_into().ok()?) as usize;
            *i += 4;
            v
        }
        _ => return None,
    })
}

fn p_decode(b: &[u8], i: &mut usize, depth: u32) -> Option<PNode> {
    if depth > 64 {
        return None;
    }
    let t = *b.get(*i)?;
    *i += 1;
    let (hi, lo) = ((t >> 4) as usize, t & 15);
    match lo {
        1 | 2 if hi == 0 => Some(PNode::Scalar(lo.to_string())),
        3 | 5 | 7 | 8 => {
            let n = p_len(b, i, hi)?;
            let raw = b.get(*i..*i + n)?;
            *i += n;
            Some(PNode::Scalar(String::from_utf8_lossy(raw).into_owned()))
        }
        11 | 12 => {
            let n = p_len(b, i, hi)?;
            let end = i.checked_add(n)?;
            if end > b.len() {
                return None;
            }
            if lo == 12 {
                let mut items = vec![];
                while *i < end {
                    let k = match p_decode(b, i, depth + 1)? {
                        PNode::Scalar(k) => k,
                        _ => return None,
                    };
                    items.push((k, p_decode(b, i, depth + 1)?));
                }
                Some(PNode::Struct(items))
            } else {
                let mut items = vec![];
                while *i < end {
                    items.push(p_decode(b, i, depth + 1)?);
                }
                Some(PNode::Array(items))
            }
        }
        _ => None,
    }
}

/// Root struct -> FLevelComponent (array, element [1] is a struct) -> TotalSkillPoints.
pub fn total_skill_points(blob: &[u8]) -> Option<i64> {
    let mut i = 0;
    let root = p_decode(blob, &mut i, 0)?;
    let PNode::Struct(items) = root else { return None };
    let (_, lc) = items.into_iter().find(|(k, _)| k == "FLevelComponent")?;
    let PNode::Array(mut a) = lc else { return None };
    if a.len() < 2 {
        return None;
    }
    let PNode::Struct(st) = a.swap_remove(1) else { return None };
    match st.into_iter().find(|(k, _)| k == "TotalSkillPoints")?.1 {
        PNode::Scalar(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Best-effort extraction. Column names for the solo DB aren't publicly documented,
/// so this is heuristic; use `probe` + `config name-sql / level-sql` to pin it down.
pub fn inspect(profile_dir: &Path, cfg: &Config) -> Res<CharInfo> {
    let db = TempDb::open(&profile_dir.join("game.db"))?;
    let c = db.c();
    let mut info = CharInfo::default();

    if let Some(sql) = cfg.name_sql.as_deref().filter(|s| !s.is_empty()) {
        info.name = first_value(c, sql).and_then(v_str);
        info.name_src = Some("custom SQL".into());
    }
    if let Some(sql) = cfg.level_sql.as_deref().filter(|s| !s.is_empty()) {
        info.level = first_value(c, sql).and_then(v_int);
        info.level_src = Some("custom SQL".into());
    }

    // Exact path (verified by community save editors): the character's name lives in player_state,
    // and its level is TotalSkillPoints + 1 inside the FLevelComponent blob of the pawn's
    // "DuneCharacter" entity (fgl_entities.components).
    if let Some((name, pawn)) = c
        .query_row("SELECT character_name, player_pawn_id FROM player_state ORDER BY id LIMIT 1", [], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<i64>>(1)?))
        })
        .ok()
    {
        if info.name.is_none() {
            if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
                info.name = Some(n);
                info.name_src = Some("player_state.character_name".into());
            }
        }
        if info.level.is_none() {
            if let Some(pawn) = pawn {
                let blob: Option<Vec<u8>> = c
                    .query_row(
                        "SELECT e.components FROM fgl_entities e JOIN actor_fgl_entities a ON a.entity_id = e.entity_id \
                         WHERE a.actor_id = ?1 AND a.slot_name = 'DuneCharacter' LIMIT 1",
                        [pawn],
                        |r| r.get(0),
                    )
                    .ok();
                if let Some(pts) = blob.as_deref().and_then(total_skill_points) {
                    info.level = Some(pts + 1);
                    info.level_src = Some("FLevelComponent.TotalSkillPoints + 1".into());
                }
            }
        }
    }

    let mut ts: Vec<(i32, String)> = tables(c).into_iter().map(|t| (table_score(&t), t)).filter(|(s, _)| *s > 0).collect();
    ts.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for (_, t) in &ts {
        let cols = columns(c, t);
        let pick = |prefs: &[&str]| prefs.iter().find_map(|p| cols.iter().find(|c| c.to_lowercase() == *p).cloned());
        if info.name.is_none() {
            if let Some(col) = pick(&["char_name", "character_name", "charactername", "player_name", "playername", "name"]) {
                let sql = format!("SELECT {c} FROM {t} WHERE typeof({c})='text' AND {c}<>'' LIMIT 1", c = qi(&col), t = qi(t));
                if let Some(s) = first_value(c, &sql).and_then(v_str) {
                    info.name = Some(s);
                    info.name_src = Some(format!("{t}.{col}"));
                }
            }
        }
    }
    Ok(info)
}

pub fn probe_text(profile_dir: &Path) -> Res<String> {
    use std::fmt::Write as _;
    let db = TempDb::open(&profile_dir.join("game.db"))?;
    let c = db.c();
    let ts = tables(c);
    let mut o = String::new();
    writeln!(o, "{} tables. Candidate columns (table.column = sample):\n", ts.len())?;
    let kw = ["name", "level", "xp", "exp", "char"];
    for t in &ts {
        for col in columns(c, t) {
            let l = col.to_lowercase();
            if kw.iter().any(|k| l.contains(k)) {
                let sample = first_value(c, &format!("SELECT {} FROM {} WHERE {} IS NOT NULL LIMIT 1", qi(&col), qi(t), qi(&col)))
                    .map(|v| match v {
                        Value::Text(s) => format!("{:?}", s.chars().take(40).collect::<String>()),
                        Value::Integer(i) => i.to_string(),
                        Value::Real(f) => f.to_string(),
                        Value::Blob(b) => format!("<blob {} bytes>", b.len()),
                        Value::Null => "NULL".into(),
                    })
                    .unwrap_or_else(|| "-".into());
                writeln!(o, "  {t}.{col} = {sample}")?;
            }
        }
    }
    writeln!(o, "\nAll tables:\n  {}", ts.join(", "))?;
    let solo = profile_dir.join("SOLO");
    if solo.exists() {
        writeln!(o, "\nSOLO/ files:")?;
        for e in fs::read_dir(&solo)?.filter_map(|e| e.ok()) {
            writeln!(o, "  {}", e.file_name().to_string_lossy())?;
        }
    }
    Ok(o)
}

// ───────────────────────── screenshots ─────────────────────────

pub fn screenshot_dirs(profile_dir: &Path, cfg: &Config) -> Vec<PathBuf> {
    let mut v = cfg.screenshot_dirs.clone();
    let acct = profile_dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|id| *id >= STEAM64_BASE)
        .map(|id| (id - STEAM64_BASE).to_string());
    for root in steam_roots() {
        let ud = root.join("userdata");
        let accts: Vec<PathBuf> = match &acct {
            Some(a) => vec![ud.join(a)],
            None => subdirs(&ud),
        };
        for a in accts {
            v.push(a.join("760").join("remote").join(APP_ID).join("screenshots"));
        }
    }
    // Unreal Engine default screenshot location
    if let Some(l) = dirs::data_local_dir() {
        v.push(l.join("DuneSandbox").join("Saved").join("Screenshots"));
    }
    if let Some(d) = dirs::document_dir() {
        v.push(d.join("DuneAwakening"));
    }
    v.retain(|p| p.exists());
    v
}

pub fn collect_images(dir: &Path, depth: u32, out: &mut Vec<(SystemTime, PathBuf)>) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                if depth > 0 && p.file_name().map(|n| n != "thumbnails").unwrap_or(true) {
                    collect_images(&p, depth - 1, out);
                }
            } else if p.extension().and_then(|x| x.to_str()).map(|x| IMG_EXT.contains(&x.to_lowercase().as_str())).unwrap_or(false) {
                if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                    out.push((m, p));
                }
            }
        }
    }
}

pub fn newest_screenshot(profile_dir: &Path, cfg: &Config, max_age_hours: u64) -> Option<PathBuf> {
    let mut imgs = vec![];
    for d in screenshot_dirs(profile_dir, cfg) {
        collect_images(&d, 2, &mut imgs);
    }
    let cutoff = SystemTime::now().checked_sub(Duration::from_secs(max_age_hours * 3600))?;
    imgs.into_iter().filter(|(t, _)| *t >= cutoff).max_by_key(|(t, _)| *t).map(|(_, p)| p)
}

// ───────────────────────── vault ─────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct Meta {
    pub name: String,
    pub created: String,
    pub unix: i64,
    pub note: Option<String>,
    pub character: CharInfo,
    pub screenshot: Option<String>,
    pub bytes: u64,
    pub source_profile: String,
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && !n.starts_with('.')
        && n.chars().all(|c| c.is_alphanumeric() || " ._-".contains(c))
}

pub fn copy_dir(src: &Path, dst: &Path) -> Res<u64> {
    fs::create_dir_all(dst)?;
    let mut total = 0;
    for e in fs::read_dir(src)? {
        let e = e?;
        let (s, d) = (e.path(), dst.join(e.file_name()));
        if s.is_dir() {
            total += copy_dir(&s, &d)?;
        } else {
            total += fs::copy(&s, &d)?;
        }
    }
    Ok(total)
}
pub fn clear_dir(dir: &Path) -> Res<()> {
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            fs::remove_dir_all(&p)?;
        } else {
            fs::remove_file(&p)?;
        }
    }
    Ok(())
}

pub enum Shot {
    None,
    Auto(u64),
    File(PathBuf),
}

/// Small JPEG next to the screenshot, so the GUI never uploads a full 4K texture per card.
pub fn ensure_thumb(snap: &Path, shot: &str) -> Option<PathBuf> {
    let t = snap.join("thumb.jpg");
    if t.exists() {
        return Some(t);
    }
    let img = image::open(snap.join(shot)).ok()?;
    img.thumbnail(560, 315).to_rgb8().save_with_format(&t, image::ImageFormat::Jpeg).ok()?;
    Some(t)
}

pub fn attach_shot(snap: &Path, file: &Path) -> Res<String> {
    let _ = fs::remove_file(snap.join("thumb.jpg"));
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
    for e in IMG_EXT {
        let _ = fs::remove_file(snap.join(format!("screenshot.{e}")));
    }
    let name = format!("screenshot.{ext}");
    fs::copy(file, snap.join(&name))?;
    Ok(name)
}

pub fn take_snapshot(vault: &Path, name: &str, live: &Path, cfg: &Config, note: Option<String>, shot: Shot) -> Res<Meta> {
    let snap = vault.join(name);
    if snap.exists() {
        bail!("Snapshot '{}' already exists (delete it first or pick another name)", name);
    }
    let character = match inspect(live, cfg) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("warning: couldn't read game.db ({e}); saving files anyway without character info");
            CharInfo::default()
        }
    };
    fs::create_dir_all(&snap)?;
    let bytes = copy_dir(live, &snap.join("data"))?;
    let screenshot = match shot {
        Shot::None => None,
        Shot::File(f) => Some(attach_shot(&snap, &f)?),
        Shot::Auto(h) => newest_screenshot(live, cfg, h).and_then(|f| attach_shot(&snap, &f).ok()),
    };
    let now = chrono::Local::now();
    let meta = Meta {
        name: name.into(),
        created: now.format("%Y-%m-%d %H:%M").to_string(),
        unix: now.timestamp(),
        note,
        character,
        screenshot,
        bytes,
        source_profile: live.file_name().map(|n| n.to_string_lossy().into()).unwrap_or_default(),
    };
    fs::write(snap.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
    Ok(meta)
}

pub fn read_meta(vault: &Path, name: &str) -> Res<Meta> {
    let p = vault.join(name).join("meta.json");
    let s = fs::read_to_string(&p).map_err(|_| format!("No snapshot named '{name}'"))?;
    Ok(serde_json::from_str(&s)?)
}
pub fn all_meta(vault: &Path) -> Vec<Meta> {
    let mut v: Vec<Meta> = subdirs(vault)
        .iter()
        .filter_map(|d| fs::read_to_string(d.join("meta.json")).ok())
        .filter_map(|s| serde_json::from_str(&s).ok())
        .collect();
    v.sort_by_key(|m| std::cmp::Reverse(m.unix));
    v
}

/// Replace the live save with a snapshot; returns the name of the auto-backup.
pub fn load_snapshot(vault: &Path, name: &str, live: &Path, cfg: &Config) -> Res<String> {
    read_meta(vault, name)?;
    let data = vault.join(name).join("data");
    if !data.exists() {
        bail!("Snapshot data folder is missing");
    }
    let auto = format!("_auto-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    take_snapshot(vault, &auto, live, cfg, Some(format!("auto-backup before loading '{name}'")), Shot::None)?;
    clear_dir(live)?;
    copy_dir(&data, live)?;
    Ok(auto)
}

pub fn delete_snapshot(vault: &Path, name: &str) -> Res<()> {
    read_meta(vault, name)?;
    fs::remove_dir_all(vault.join(name))?;
    Ok(())
}

pub fn set_screenshot(vault: &Path, name: &str, file: &Path) -> Res<()> {
    let mut m = read_meta(vault, name)?;
    m.screenshot = Some(attach_shot(&vault.join(name), file)?);
    fs::write(vault.join(name).join("meta.json"), serde_json::to_string_pretty(&m)?)?;
    Ok(())
}

pub fn confirm(q: &str) -> bool {
    print!("{q} [y/N] ");
    let _ = io::stdout().flush();
    let mut s = String::new();
    io::stdin().read_line(&mut s).is_ok() && s.trim().eq_ignore_ascii_case("y")
}
pub fn fmt_char(c: &CharInfo) -> (String, String) {
    (
        c.name.clone().unwrap_or_else(|| "?".into()),
        c.level.map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
    )
}
pub fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1_048_576.0)
}
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn gallery(vault: &Path) -> Res<PathBuf> {
    let mut cards = String::new();
    for m in all_meta(vault) {
        let (n, l) = fmt_char(&m.character);
        let img = match &m.screenshot {
            Some(s) => format!("<img src=\"{}/{}\" loading=\"lazy\">", esc(&m.name), esc(s)),
            None => "<div class=\"noshot\">no screenshot</div>".into(),
        };
        cards.push_str(&format!(
            "<div class=\"card\">{img}<div class=\"b\"><h2>{}</h2><p class=\"c\">{} &middot; Level {}</p><p class=\"m\">{} &middot; {}</p>{}</div></div>\n",
            esc(&m.name), esc(&n), esc(&l), esc(&m.created), mb(m.bytes),
            m.note.as_deref().map(|x| format!("<p class=\"n\">{}</p>", esc(x))).unwrap_or_default()
        ));
    }
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>Dune solo saves</title><style>\
        body{{margin:0;padding:2rem;background:#1b1612;color:#e9dcc3;font:15px system-ui,sans-serif}}\
        h1{{margin:0 0 1.5rem;color:#e0a458;letter-spacing:.05em}}\
        .grid{{display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:1.2rem}}\
        .card{{background:#2a221b;border:1px solid #4a3b2c;border-radius:10px;overflow:hidden}}\
        img,.noshot{{width:100%;aspect-ratio:16/9;object-fit:cover;display:block;background:#120e0b}}\
        .noshot{{display:flex;align-items:center;justify-content:center;color:#7d6c58}}\
        .b{{padding:.8rem 1rem}}h2{{margin:0;font-size:1.1rem}}.c{{margin:.3rem 0;color:#e0a458}}\
        .m{{margin:0;color:#9c8a74;font-size:.85rem}}.n{{margin:.5rem 0 0;font-style:italic}}\
        </style><h1>Dune: Awakening &mdash; solo characters</h1><div class=grid>\n{cards}</div>"
    );
    let p = vault.join("gallery.html");
    fs::write(&p, html)?;
    Ok(p)
}

pub fn open_path(p: &Path) {
    let _ = if cfg!(windows) {
        quiet_cmd("explorer").arg(p).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(p).spawn()
    } else {
        Command::new("xdg-open").arg(p).spawn()
    };
}