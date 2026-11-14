#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use dune_save_manager::*;
use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Vec2};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

const SAND: Color32 = Color32::from_rgb(224, 164, 88);
/// Small "note" glyph (plain lines, not a button) drawn with the painter, so it needs no emoji font.
fn note_icon(ui: &mut egui::Ui) -> egui::Response {
    let size = Vec2::splat(ui.spacing().interact_size.y);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let color = if resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
    let r = rect.shrink(size.x * 0.2);
    let stroke = egui::Stroke::new(1.5_f32, color);
    for i in 0..4 {
        let y = r.top() + r.height() * (i as f32 / 3.0);
        let x2 = if i == 3 { r.center().x } else { r.right() };
        ui.painter().line_segment([egui::pos2(r.left(), y), egui::pos2(x2, y)], stroke);
    }
    resp
}

fn char_label(m: &Meta) -> String {
    m.character.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Unknown".into())
}

const THUMB: Vec2 = Vec2::new(280.0, 157.0);

#[derive(PartialEq, Clone, Copy)]
enum ShotMode {
    Auto,
    Off,
    File,
}

enum Pending {
    Load(String),
    Delete(String),
}

enum Msg {
    Status(String),
    Warn(String),
    Done(Result<Meta, String>),
}

struct Job {
    rx: mpsc::Receiver<Msg>,
    status: String,
    warn: Option<String>,
}

struct App {
    cfg: Config,
    live: Result<PathBuf, String>,
    info: Option<CharInfo>,
    info_err: Option<String>,
    vault: PathBuf,
    metas: Vec<Meta>,
    status: (String, bool),
    // save form
    new_name: String,
    new_note: String,
    shot_mode: ShotMode,
    shot_file: String,
    max_age: u64,
    show_auto: bool,
    // dialogs
    pending: Option<Pending>,
    attach_for: Option<(String, String)>,
    settings_open: bool,
    probe_out: Option<String>,
    // game process
    running: Arc<AtomicBool>,
    thumbs: std::collections::HashMap<String, PathBuf>,
    job: Option<Job>,
    /// Selected character tab in the snapshots list (None = all characters).
    char_tab: Option<String>,
    log_edit: String,
    log_found: Option<PathBuf>,
    // cached so Settings doesn't hit the disk every frame
    cands: Vec<PathBuf>,
    profiles: Vec<String>,
    // settings edit buffers
    saves_edit: String,
    vault_edit: String,
    shots_edit: String,
    name_sql: String,
    level_sql: String,
}

impl App {
    fn new() -> Self {
        let cfg = load_config();
        let mut a = App {
            saves_edit: cfg.saves_root.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            vault_edit: cfg.vault.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            shots_edit: String::new(),
            name_sql: cfg.name_sql.clone().unwrap_or_default(),
            level_sql: cfg.level_sql.clone().unwrap_or_default(),
            log_edit: cfg.log_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            cfg,
            live: Err("not loaded".into()),
            info: None,
            info_err: None,
            vault: PathBuf::new(),
            metas: vec![],
            status: (String::new(), false),
            new_name: String::new(),
            new_note: String::new(),
            shot_mode: ShotMode::Auto,
            shot_file: String::new(),
            max_age: 24,
            show_auto: false,
            pending: None,
            attach_for: None,
            settings_open: false,
            probe_out: None,
            thumbs: Default::default(),
            job: None,
            char_tab: None,
            log_found: None,
            running: Arc::new(AtomicBool::new(false)),
            cands: vec![],
            profiles: vec![],
        };
        a.refresh();
        a
    }

    fn start_watcher(&self, ctx: egui::Context) {
        let flag = self.running.clone();
        std::thread::spawn(move || loop {
            let now = game_running();
            if flag.swap(now, Ordering::Relaxed) != now {
                ctx.request_repaint();
            }
            std::thread::sleep(Duration::from_secs(3));
        });
    }

    fn refresh_settings_cache(&mut self) {
        self.cands = fls_candidates();
        self.profiles = self.profile_choices();
        self.log_found = self.live_path().and_then(|l| find_log(&l, &self.cfg));
    }

    fn say(&mut self, msg: impl ToString, err: bool) {
        self.status = (msg.to_string(), err);
    }

    fn refresh(&mut self) {
        self.live = resolve_live(&None, &self.cfg).map_err(|e| e.to_string());
        self.info = None;
        self.info_err = None;
        if let Ok(p) = &self.live {
            match inspect(p, &self.cfg) {
                Ok(i) => self.info = Some(i),
                Err(e) => self.info_err = Some(e.to_string()),
            }
        }
        match vault_dir(&None, &self.cfg) {
            Ok(v) => {
                self.metas = all_meta(&v);
                self.thumbs.clear();
                for m in &self.metas {
                    if let Some(shot) = &m.screenshot {
                        if let Some(t) = ensure_thumb(&v.join(&m.name), shot) {
                            self.thumbs.insert(m.name.clone(), t);
                        }
                    }
                }
                self.vault = v;
            }
            Err(e) => self.say(e, true),
        }
    }

    fn live_path(&self) -> Option<PathBuf> {
        self.live.as_ref().ok().cloned()
    }

    fn do_save(&mut self, ctx: &egui::Context) {
        let name = self.new_name.trim().to_string();
        if !valid_name(&name) {
            return self.say("Name may only contain letters, digits, spaces, . _ -", true);
        }
        if name.starts_with("_auto-") {
            return self.say("'_auto-' is reserved for automatic backups", true);
        }
        let Some(live) = self.live_path() else {
            return self.say("No live save found; open Settings.", true);
        };
        let (cfg, vault) = (self.cfg.clone(), self.vault.clone());
        let note = Some(self.new_note.trim().to_string()).filter(|s| !s.is_empty());
        let (mode, max_age, shot_file) = (self.shot_mode, self.max_age, PathBuf::from(self.shot_file.trim()));
        let (tx, rx) = mpsc::channel::<Msg>();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let say = |s: String| {
                let _ = tx.send(Msg::Status(s));
                ctx2.request_repaint();
            };
            // Screenshot first, at the moment the user pressed Save.
            let (shot, tmp, warn) = match mode {
                ShotMode::Off => (Shot::None, None, None),
                ShotMode::File => (Shot::File(shot_file), None, None),
                ShotMode::Auto => {
                    say("Taking screenshot…".into());
                    auto_shot(max_age)
                }
            };
            if let Some(w) = warn {
                let _ = tx.send(Msg::Warn(w.clone()));
                say(format!("{w} - using the newest Steam screenshot instead."));
            }
            let mut progress = |s: String| say(s);
            let res = safe_snapshot(&vault, &name, &live, &cfg, note, shot, true, &mut progress).map_err(|e| e.to_string());
            if let Some(t) = tmp {
                let _ = std::fs::remove_file(t);
            }
            let _ = tx.send(Msg::Done(res));
            ctx2.request_repaint();
        });
        self.job = Some(Job { rx, status: "Starting…".into(), warn: None });
    }

    fn poll_job(&mut self) {
        let mut done = None;
        let mut warn = None;
        if let Some(job) = self.job.as_mut() {
            while let Ok(m) = job.rx.try_recv() {
                match m {
                    Msg::Status(s) => job.status = s,
                    Msg::Warn(w) => job.warn = Some(w),
                    Msg::Done(r) => done = Some(r),
                }
            }
        }
        let Some(r) = done else { return };
        warn = warn.or_else(|| self.job.as_ref().and_then(|j| j.warn.clone()));
        self.job = None;
        match r {
            Ok(m) => {
                let msg = if m.screenshot.is_some() || self.shot_mode == ShotMode::Off {
                    format!("Saved '{}'.", m.name)
                } else {
                    format!("Saved '{}' without a screenshot. {}; no recent Steam screenshot was found either.", m.name, warn.unwrap_or_else(|| "No screenshot source available".into()))
                };
                self.say(msg, false);
                self.new_name.clear();
                self.new_note.clear();
                self.refresh();
            }
            Err(e) => self.say(e, true),
        }
    }

    fn do_pending(&mut self, ctx: &egui::Context) {
        let Some(p) = self.pending.take() else { return };
        match p {
            Pending::Load(name) => {
                if game_running() {
                    return self.say("The game is running. Exit it before loading a character.", true);
                }
                let Some(live) = self.live_path() else {
                    return self.say("No live save found.", true);
                };
                match load_snapshot(&self.vault, &name, &live, &self.cfg) {
                    Ok(auto) => {
                        self.say(
                            format!("Loaded '{name}'. Previous save kept as '{auto}'. Disable Steam Cloud for Dune or it may sync the wrong slot back."),
                            false,
                        );
                        self.refresh();
                    }
                    Err(e) => self.say(e, true),
                }
            }
            Pending::Delete(name) => match delete_snapshot(&self.vault, &name) {
                Ok(()) => {
                    self.say(format!("Deleted '{name}'."), false);
                    self.refresh();
                    ctx.request_repaint();
                }
                Err(e) => self.say(e, true),
            },
        }
    }

    fn apply_settings(&mut self) {
        let t = |s: &str| Some(s.trim().to_string()).filter(|x| !x.is_empty());
        let new_saves = t(&self.saves_edit).map(PathBuf::from);
        if new_saves != self.cfg.saves_root {
            self.cfg.profile = None;
        }
        self.cfg.saves_root = new_saves;
        self.cfg.vault = t(&self.vault_edit).map(PathBuf::from);
        self.cfg.log_path = t(&self.log_edit).map(PathBuf::from);
        self.cfg.name_sql = t(&self.name_sql);
        self.cfg.level_sql = t(&self.level_sql);
        if let Some(s) = t(&self.shots_edit) {
            let p = PathBuf::from(s);
            if !self.cfg.screenshot_dirs.contains(&p) {
                self.cfg.screenshot_dirs.push(p);
            }
            self.shots_edit.clear();
        }
        match save_config(&self.cfg) {
            Ok(()) => self.say("Settings saved.", false),
            Err(e) => self.say(e, true),
        }
        self.refresh();
    }

    fn profile_choices(&self) -> Vec<String> {
        let root = self.cfg.saves_root.clone().or_else(|| fls_candidates().into_iter().next());
        match root {
            Some(r) if !r.join("game.db").exists() => subdirs(&r)
                .into_iter()
                .filter(|p| p.join("game.db").exists())
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect(),
            _ => vec![],
        }
    }
}

fn img_uri(p: &Path) -> String {
    format!("file://{}", p.display().to_string().replace('\\', "/"))
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let running = self.running.load(Ordering::Relaxed);
        self.poll_job();

        // ── top bar
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("Dune: Awakening  |  Solo Save Manager").color(SAND));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Settings").clicked() {
                        self.settings_open = true;
                        self.refresh_settings_cache();
                    }
                    if ui.button("Refresh").clicked() {
                        self.refresh();
                    }
                    let (c, t) = if running {
                        (Color32::from_rgb(220, 150, 60), "Game running - saving is timed to its DB save; loading is disabled")
                    } else {
                        (Color32::from_rgb(90, 190, 110), "Game closed")
                    };
                    ui.colored_label(c, format!("● {t}"));
                });
            });
        });

        // ── status bar
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let (msg, err) = &self.status;
            if msg.is_empty() {
                ui.weak(format!("Vault: {}", self.vault.display()));
            } else if *err {
                ui.colored_label(Color32::from_rgb(230, 100, 80), msg);
            } else {
                ui.colored_label(Color32::from_rgb(120, 200, 130), msg);
            }
        });

        // ── left: live save + save form
        let mut want_save = false;
        let mut enter_save = false;
        egui::SidePanel::left("left").resizable(false).exact_width(330.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("LIVE SAVE").strong().color(SAND));
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                match (&self.live, &self.info, &self.info_err) {
                    (Err(e), _, _) => {
                        ui.colored_label(Color32::from_rgb(230, 100, 80), e);
                        ui.label("Open Settings to point at your save folder.");
                    }
                    (Ok(p), info, err) => {
                        if let Some(i) = info {
                            let (n, l) = (i.name.clone().unwrap_or("Unknown".into()), i.level.map(|x| x.to_string()).unwrap_or("?".into()));
                            ui.label(RichText::new(n).size(22.0).strong());
                            ui.label(RichText::new(format!("Level {l}")).size(16.0).color(SAND));
                            if i.name.is_none() || i.level.is_none() {
                                ui.weak("Name/level not found automatically - Settings > Probe.");
                            } else {
                                ui.weak(format!(
                                    "from {} / {}",
                                    i.name_src.as_deref().unwrap_or("?"),
                                    i.level_src.as_deref().unwrap_or("?")
                                ));
                            }
                        }
                        if let Some(e) = err {
                            ui.colored_label(Color32::from_rgb(230, 100, 80), format!("Can't read game.db: {e}"));
                        }
                        ui.add_space(4.0);
                        ui.weak(p.display().to_string());
                        if ui.small_button("Open folder").clicked() {
                            open_path(p);
                        }
                    }
                }
            });

            ui.add_space(10.0);
            ui.label(RichText::new("SAVE SNAPSHOT").strong().color(SAND));
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.label("Snapshot name");
                let name_resp = ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("e.g. atreides-run").desired_width(f32::INFINITY));
                if name_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    enter_save = true;
                }
                ui.add_space(4.0);
                ui.label("Note (optional)");
                egui::ScrollArea::vertical().id_source("note_scroll").max_height(130.0).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.new_note)
                            .hint_text("Anything worth remembering about this save…")
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    );
                });
                ui.add_space(4.0);
                ui.label("Screenshot");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.shot_mode, ShotMode::Auto, "Auto");
                    ui.selectable_value(&mut self.shot_mode, ShotMode::File, "Pick file");
                    ui.selectable_value(&mut self.shot_mode, ShotMode::Off, "None");
                });
                match self.shot_mode {
                    ShotMode::Auto => {
                        ui.horizontal(|ui| {
                            ui.label("newer than");
                            ui.add(egui::DragValue::new(&mut self.max_age).clamp_range(1..=720).suffix(" h"));
                        });
                        ui.weak("Captures the game window now; otherwise uses your newest Steam (F12) screenshot.");
                    }
                    ShotMode::File => {
                        ui.add(egui::TextEdit::singleline(&mut self.shot_file).hint_text("path to image").desired_width(f32::INFINITY));
                    }
                    ShotMode::Off => {}
                }
                ui.add_space(8.0);
                let ok = self.live.is_ok() && !self.new_name.trim().is_empty() && self.job.is_none();
                let btn = egui::Button::new(RichText::new("Save snapshot").size(17.0).strong().color(Color32::WHITE))
                    .fill(Color32::from_rgb(168, 112, 36))
                    .rounding(6.0)
                    .min_size(Vec2::new(ui.available_width(), 38.0));
                let clicked = ui.add_enabled(ok, btn).clicked();
                if clicked || (enter_save && ok) {
                    want_save = true;
                }
                if self.job.is_none() && !ok {
                    ui.weak(if self.live.is_err() { "Live save not found - open Settings." } else { "Enter a name to enable saving." });
                }
                if let Some(job) = &self.job {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(&job.status);
                    });
                } else if running {
                    ui.weak("Game is running: the copy waits for a safe moment after the game's own database save (up to ~30 s).");
                }
            });
            });
        });
        if want_save {
            self.do_save(ctx);
        }

        // ── central: snapshot cards
        let mut load: Option<String> = None;
        let mut del: Option<String> = None;
        let mut attach: Option<String> = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("SNAPSHOTS").strong().color(SAND));
                ui.checkbox(&mut self.show_auto, "show automatic backups");
                if ui.small_button("Open vault folder").clicked() {
                    open_path(&self.vault);
                }
            });
            ui.separator();
            let visible: Vec<&Meta> = self.metas.iter().filter(|m| self.show_auto || !m.name.starts_with("_auto-")).collect();
            // Character tabs, only when the vault holds more than one character (most recently saved first).
            let mut names: Vec<String> = vec![];
            for m in &visible {
                let n = char_label(m);
                if !names.contains(&n) {
                    names.push(n);
                }
            }
            if names.len() > 1 {
                if self.char_tab.as_ref().map_or(false, |t| !names.contains(t)) {
                    self.char_tab = None;
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.selectable_label(self.char_tab.is_none(), format!("All ({})", visible.len())).clicked() {
                        self.char_tab = None;
                    }
                    for n in &names {
                        let count = visible.iter().filter(|m| &char_label(m) == n).count();
                        let sel = self.char_tab.as_ref() == Some(n);
                        if ui.selectable_label(sel, format!("{n} ({count})")).clicked() {
                            self.char_tab = Some(n.clone());
                        }
                    }
                });
                ui.add_space(2.0);
            } else {
                self.char_tab = None;
            }
            let metas: Vec<&Meta> = visible.into_iter().filter(|m| self.char_tab.as_ref().map_or(true, |t| &char_label(m) == t)).collect();
            if metas.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| ui.weak("No snapshots yet. Name your current character on the left and press Save."));
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                // Cards have a fixed width; work out how many fit per row and lay them out row by row
                // (egui's wrapped layout does not wrap nested frames, so everything ended up in one row).
                let frame = egui::Frame::group(ui.style());
                let card_w = THUMB.x + frame.total_margin().sum().x;
                let gap = 10.0;
                ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
                let cols = (((ui.available_width() + gap) / (card_w + gap)).floor() as usize).max(1);
                for row in metas.chunks(cols) {
                    ui.horizontal_top(|ui| {
                        for m in row {
                            frame.show(ui, |ui| {
                              ui.vertical(|ui| {
                                ui.set_width(THUMB.x);
                                ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                                match &m.screenshot {
                                    Some(s) => {
                                        let shown = self.thumbs.get(&m.name).cloned().unwrap_or_else(|| self.vault.join(&m.name).join(s));
                                        ui.add(egui::Image::new(img_uri(&shown)).fit_to_exact_size(THUMB).rounding(4.0));
                                    }
                                    None => {
                                        let (rect, _) = ui.allocate_exact_size(THUMB, Sense::hover());
                                        ui.painter().rect_filled(rect, 4.0, Color32::from_gray(24));
                                        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "no screenshot", FontId::proportional(13.0), Color32::GRAY);
                                    }
                                }
                                // line 1: snapshot name
                                ui.add(egui::Label::new(RichText::new(&m.name).strong().size(16.0)).truncate(true)).on_hover_text(&m.name);
                                // line 2: character, level, date
                                let n = m.character.name.clone().unwrap_or("Unknown".into());
                                let l = m.character.level.map(|x| x.to_string()).unwrap_or("?".into());
                                let mut job = egui::text::LayoutJob::default();
                                job.append(&format!("{n}  ·  Lv {l}"), 0.0, egui::text::TextFormat { font_id: FontId::proportional(14.0), color: SAND, ..Default::default() });
                                job.append(&format!("  ·  {}", m.created), 0.0, egui::text::TextFormat { font_id: FontId::proportional(13.0), color: Color32::GRAY, ..Default::default() });
                                ui.add(egui::Label::new(job).truncate(true)).on_hover_text(format!("{n}, level {l}\nSaved {}  ({})", m.created, mb(m.bytes)));
                                // line 3: buttons + note icon
                                ui.horizontal(|ui| {
                                    let load_btn = egui::Button::new(RichText::new("Load").strong().color(Color32::WHITE)).fill(Color32::from_rgb(40, 130, 85)).min_size(Vec2::new(54.0, 0.0));
                                    if ui.add_enabled(!running, load_btn).on_disabled_hover_text("Close the game first").clicked() {
                                        load = Some(m.name.clone());
                                    }
                                    if ui.button("Screenshot").clicked() {
                                        attach = Some(m.name.clone());
                                    }
                                    if ui.button("Delete").clicked() {
                                        del = Some(m.name.clone());
                                    }
                                    if let Some(note) = m.note.as_ref().filter(|t| !t.trim().is_empty()) {
                                        // Hovering the glyph shows the note itself (long notes are wrapped and capped).
                                        let text: String = if note.chars().count() > 1500 { note.chars().take(1500).collect::<String>() + "…" } else { note.clone() };
                                        note_icon(ui).on_hover_ui(|ui| {
                                            ui.set_max_width(360.0);
                                            ui.label(text);
                                        });
                                    }
                                });
                              });
                            });
                        }
                    });
                }
            });
        });
        if let Some(n) = load {
            self.pending = Some(Pending::Load(n));
        }
        if let Some(n) = del {
            self.pending = Some(Pending::Delete(n));
        }
        if let Some(n) = attach {
            self.attach_for = Some((n, String::new()));
        }

        // ── confirm dialog
        let mut decision: Option<bool> = None;
        if let Some(p) = &self.pending {
            let text = match p {
                Pending::Load(n) => format!("Replace your live save with '{n}'?\nYour current save is backed up automatically first."),
                Pending::Delete(n) => format!("Permanently delete snapshot '{n}'?"),
            };
            egui::Window::new("Confirm").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(text);
                ui.horizontal(|ui| {
                    if ui.button("Yes").clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
        }
        match decision {
            Some(true) => self.do_pending(ctx),
            Some(false) => self.pending = None,
            None => {}
        }

        // ── attach screenshot dialog
        let mut close_attach = false;
        let mut apply_attach: Option<(String, PathBuf)> = None;
        let mut newest: Option<String> = None;
        if let Some((name, path)) = &mut self.attach_for {
            egui::Window::new(format!("Screenshot for '{name}'")).collapsible(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label("Path to an image (png/jpg/bmp/webp):");
                ui.add(egui::TextEdit::singleline(path).desired_width(420.0));
                ui.horizontal(|ui| {
                    if ui.button("Use newest Steam screenshot (7 days)").clicked() {
                        newest = Some(name.clone());
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Attach").clicked() {
                        apply_attach = Some((name.clone(), PathBuf::from(path.trim())));
                    }
                    if ui.button("Close").clicked() {
                        close_attach = true;
                    }
                });
            });
        }
        if let Some(name) = newest {
            let found = self.live_path().and_then(|l| newest_screenshot(&l, &self.cfg, 24 * 7));
            match found {
                Some(f) => apply_attach = Some((name, f)),
                None => self.say("No screenshot from the last 7 days found.", true),
            }
        }
        if let Some((name, file)) = apply_attach {
            match set_screenshot(&self.vault, &name, &file) {
                Ok(()) => {
                    if let Ok(m) = read_meta(&self.vault, &name) {
                        if let Some(s) = m.screenshot {
                            ctx.forget_image(&img_uri(&self.vault.join(&name).join(s)));
                            ctx.forget_image(&img_uri(&self.vault.join(&name).join("thumb.jpg")));
                        }
                    }
                    self.say(format!("Screenshot attached to '{name}'."), false);
                    self.refresh();
                    close_attach = true;
                }
                Err(e) => self.say(e, true),
            }
        }
        if close_attach {
            self.attach_for = None;
        }

        // ── settings window
        if self.settings_open {
            let mut open = true;
            let mut apply = false;
            let mut probe_now = false;
            let mut pick_profile: Option<String> = None;
            let profiles = self.profiles.clone();
            let cands = self.cands.clone();
            let log_found = self.log_found.clone();
            egui::Window::new("Settings").open(&mut open).default_width(560.0).vscroll(true).show(ctx, |ui| {
                ui.label(RichText::new("Save location (default: Steam)").strong());
                ui.label("The <steamid> folder containing game.db, or its FLS_retail parent. Leave blank to auto-detect.");
                ui.add(egui::TextEdit::singleline(&mut self.saves_edit).desired_width(f32::INFINITY));
                if cands.is_empty() {
                    ui.weak("Auto-detect found nothing (Steam on Windows, Steam/Proton on Linux). Paste the path above.");
                } else {
                    ui.label("Detected:");
                    for c in &cands {
                        if ui.small_button(c.display().to_string()).clicked() {
                            self.saves_edit = c.display().to_string();
                        }
                    }
                }
                if !profiles.is_empty() {
                    ui.add_space(4.0);
                    ui.label("Several Steam accounts found, pick one:");
                    ui.horizontal_wrapped(|ui| {
                        for p in &profiles {
                            let sel = self.cfg.profile.as_deref() == Some(p.as_str());
                            if ui.selectable_label(sel, p).clicked() {
                                pick_profile = Some(p.clone());
                            }
                        }
                    });
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Vault (where snapshots are stored)").strong());
                ui.add(egui::TextEdit::singleline(&mut self.vault_edit).hint_text("blank = default").desired_width(f32::INFINITY));
                ui.add_space(8.0);
                ui.label(RichText::new("Game log (used to time saves while the game runs)").strong());
                ui.add(egui::TextEdit::singleline(&mut self.log_edit).hint_text("blank = auto-detect Saved/Logs/DuneSandbox.log").desired_width(f32::INFINITY));
                match &log_found {
                    Some(p) => ui.weak(format!("Using: {}", p.display())),
                    None => ui.weak("No log found yet (the game must have been started once)."),
                };
                ui.add_space(8.0);
                ui.label(RichText::new("Extra screenshot folders").strong());
                for d in self.cfg.screenshot_dirs.clone() {
                    ui.horizontal(|ui| {
                        ui.label(d.display().to_string());
                        if ui.small_button("remove").clicked() {
                            self.cfg.screenshot_dirs.retain(|x| x != &d);
                        }
                    });
                }
                ui.add(egui::TextEdit::singleline(&mut self.shots_edit).hint_text("add a folder, then Apply").desired_width(f32::INFINITY));
                ui.add_space(8.0);
                ui.label(RichText::new("Character name / level overrides (advanced)").strong());
                ui.weak("Only needed if the automatic lookup is wrong. Use Probe to see candidate columns.");
                ui.add(egui::TextEdit::singleline(&mut self.name_sql).hint_text("SELECT col FROM tbl WHERE ... LIMIT 1").desired_width(f32::INFINITY));
                ui.add(egui::TextEdit::singleline(&mut self.level_sql).hint_text("SELECT col FROM tbl WHERE ... LIMIT 1").desired_width(f32::INFINITY));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        apply = true;
                    }
                    if ui.button("Probe database").clicked() {
                        probe_now = true;
                    }
                });
                if let Some(out) = &self.probe_out {
                    ui.add_space(6.0);
                    let lines: Vec<&str> = out.lines().collect();
                    let h = ui.text_style_height(&egui::TextStyle::Monospace);
                    egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show_rows(ui, h, lines.len(), |ui, range| {
                        for i in range {
                            ui.add(egui::Label::new(RichText::new(lines[i]).monospace()).wrap(false));
                        }
                    });
                    if ui.small_button("Save probe to file").clicked() {
                        if let Some(d) = dirs::data_local_dir() {
                            let f = d.join("DuneSaveManager").join("probe.txt");
                            let _ = std::fs::create_dir_all(f.parent().unwrap());
                            if std::fs::write(&f, out).is_ok() {
                                open_path(&f);
                            }
                        }
                    }
                }
            });
            if let Some(p) = pick_profile {
                self.cfg.profile = Some(p);
                let _ = save_config(&self.cfg);
                self.refresh();
            }
            if apply {
                self.apply_settings();
            }
            if probe_now {
                self.probe_out = Some(match self.live_path() {
                    Some(p) => probe_text(&p).unwrap_or_else(|e| format!("error: {e}")),
                    None => "No live save resolved yet.".into(),
                });
            }
            self.settings_open = open;
        }
    }
}

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1150.0, 740.0]).with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Dune Solo Save Manager",
        opts,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let app = App::new();
            app.start_watcher(cc.egui_ctx.clone());
            Box::new(app)
        }),
    )
}