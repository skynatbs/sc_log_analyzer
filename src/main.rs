use chrono::{DateTime, Utc};
use eframe::egui::{self, Color32, RichText, Sense};
use once_cell::sync::Lazy;
use regex::Regex;
use rfd::FileDialog;
use std::{
    collections::{HashMap, HashSet},
    env,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant, SystemTime},
};

mod player_info;
mod settings;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "SC Log Analyzer",
        native_options,
        Box::new(|cc| Box::new(LogApp::new(cc))),
    )
}

struct LogApp {
    file_path_input: String,
    events: Vec<PlayerEvent>,
    app_version: String,
    filter_show_kills: bool,
    filter_show_spawns: bool,
    filter_show_corpse: bool,
    filter_show_zone_moves: bool,
    filter_show_status_effects: bool,
    filter_show_hits: bool,
    filter_show_vehicle_destruction: bool,
    search_text: String,
    ignored_player: String,
    ignored_player_user_override: bool,
    load_error: Option<String>,
    auto_refresh_interval: Duration,
    last_auto_check: Instant,
    last_modified: Option<SystemTime>,
    player_info_cache: HashMap<String, PlayerInfoEntry>,
    player_info_window: Option<String>,
    player_info_tx: Sender<PlayerInfoResponse>,
    player_info_rx: Receiver<PlayerInfoResponse>,
}

impl LogApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let initial_path = settings::load_last_log_path().unwrap_or_else(|| "Game.log".to_string());
        let (initial_ignored_player, ignored_player_user_override) =
            match settings::load_ignored_player() {
                Some(value) => (value, true),
                None => (String::new(), false),
            };
        let (player_info_tx, player_info_rx) = mpsc::channel();
        let mut app = Self {
            file_path_input: initial_path,
            events: Vec::new(),
            app_version: env!("SC_LOG_ANALYZER_VERSION").to_string(),
            filter_show_kills: true,
            filter_show_spawns: true,
            filter_show_corpse: true,
            filter_show_zone_moves: true,
            filter_show_status_effects: true,
            filter_show_hits: true,
            filter_show_vehicle_destruction: true,
            search_text: String::new(),
            ignored_player: initial_ignored_player,
            ignored_player_user_override,
            load_error: None,
            auto_refresh_interval: Duration::from_secs(2),
            last_auto_check: Instant::now(),
            last_modified: None,
            player_info_cache: HashMap::new(),
            player_info_window: None,
            player_info_tx,
            player_info_rx,
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        let path = resolve_input_path(&self.file_path_input);
        if path.as_os_str().is_empty() {
            self.events.clear();
            self.load_error = Some("No log file selected.".to_string());
            self.last_modified = None;
            return;
        }
        match parse_log(&path) {
            Ok(parsed) => {
                self.events = parsed.events;
                if !self.ignored_player_user_override {
                    if let Some(nickname) = parsed.primary_nickname {
                        let trimmed = nickname.trim();
                        if !trimmed.is_empty() {
                            self.ignored_player = trimmed.to_string();
                        }
                    }
                }
                self.load_error = None;
                if let Ok(metadata) = std::fs::metadata(&path) {
                    self.last_modified = metadata.modified().ok();
                }
                if let Err(err) = settings::save_last_log_path(&path) {
                    eprintln!("Failed to persist last log path: {}", err);
                }
            }
            Err(err) => {
                self.events.clear();
                self.load_error = Some(err);
            }
        }
        self.last_auto_check = Instant::now();
    }

    fn persist_ignored_player(&self) {
        if !self.ignored_player_user_override {
            return;
        }
        if let Err(err) = settings::save_ignored_player(&self.ignored_player) {
            eprintln!("Failed to persist ignored player: {}", err);
        }
    }

    fn filtered_events(&self) -> Vec<PlayerEvent> {
        let search_lower = self.search_text.to_lowercase();
        let ignored = self.ignored_player.trim();

        self.events
            .iter()
            .filter(|event| match event.kind {
                EventKind::Kill(_) => self.filter_show_kills,
                EventKind::SpawnReservation(_) => self.filter_show_spawns,
                EventKind::CorpseStatus(_) => self.filter_show_corpse,
                EventKind::ZoneTransfer(_) => self.filter_show_zone_moves,
                EventKind::StatusEffect(_) => self.filter_show_status_effects,
                EventKind::Hit(_) => self.filter_show_hits,
                EventKind::VehicleDestruction(_) => self.filter_show_vehicle_destruction,
            })
            .filter(|event| {
                if ignored.is_empty() {
                    true
                } else {
                    !event.should_ignore(ignored)
                }
            })
            .filter(|event| {
                if search_lower.is_empty() {
                    true
                } else {
                    event.matches_search(&search_lower)
                }
            })
            .cloned()
            .collect()
    }

    fn maybe_refresh(&mut self) {
        if self.last_auto_check.elapsed() < self.auto_refresh_interval {
            return;
        }

        self.last_auto_check = Instant::now();

        let path = resolve_input_path(&self.file_path_input);
        if path.as_os_str().is_empty() {
            return;
        }
        if !path.exists() {
            return;
        }

        if let Ok(metadata) = std::fs::metadata(&path) {
            if let Ok(modified) = metadata.modified() {
                let changed = self
                    .last_modified
                    .map_or(true, |previous| modified > previous);
                if changed {
                    self.reload();
                }
            }
        }
    }

    fn dialog_start_dir(&self) -> Option<PathBuf> {
        let trimmed = self.file_path_input.trim();
        if trimmed.is_empty() {
            return None;
        }
        let path = resolve_input_path(trimmed);
        if path.as_os_str().is_empty() {
            return None;
        }
        if path.is_dir() {
            Some(path)
        } else {
            path.parent().map(|p| p.to_path_buf())
        }
    }

    fn set_selected_file(&mut self, path: &Path) {
        match path.to_str() {
            Some(as_str) => {
                self.file_path_input = as_str.to_string();
                self.reload();
            }
            None => {
                self.load_error = Some("Selected path contains invalid UTF-8.".to_string());
            }
        }
    }

    fn poll_player_info_responses(&mut self) {
        while let Ok(message) = self.player_info_rx.try_recv() {
            let entry =
                self.player_info_cache
                    .entry(message.key.clone())
                    .or_insert(PlayerInfoEntry {
                        display_name: message.display_name.clone(),
                        state: PlayerInfoState::NotLoaded,
                    });
            entry.display_name = message.display_name;
            entry.state = match message.result {
                PlayerInfoResult::Success(info) => PlayerInfoState::Loaded(info),
                PlayerInfoResult::Error(err) => PlayerInfoState::Error(err),
            };
        }
    }

    fn open_player_info(&mut self, name: &str) {
        let display = name.trim().to_string();
        if display.is_empty() {
            return;
        }
        let key = canonical_player_key(&display);
        let mut should_request = false;
        {
            let entry = self
                .player_info_cache
                .entry(key.clone())
                .or_insert(PlayerInfoEntry {
                    display_name: display.clone(),
                    state: PlayerInfoState::NotLoaded,
                });
            entry.display_name = display.clone();
            if matches!(
                entry.state,
                PlayerInfoState::NotLoaded | PlayerInfoState::Error(_)
            ) {
                entry.state = PlayerInfoState::Loading;
                should_request = true;
            }
        }
        self.player_info_window = Some(key.clone());
        if should_request {
            self.spawn_player_info_request(key, display);
        }
    }

    fn spawn_player_info_request(&self, key: String, display: String) {
        let tx = self.player_info_tx.clone();
        std::thread::spawn(move || {
            let result = match player_info::fetch_player_info(&display) {
                Ok(info) => PlayerInfoResult::Success(info),
                Err(err) => PlayerInfoResult::Error(err.to_string()),
            };
            let _ = tx.send(PlayerInfoResponse {
                key,
                display_name: display,
                result,
            });
        });
    }

    fn render_player_info_window(&mut self, ctx: &egui::Context) {
        let Some(current_key) = self.player_info_window.clone() else {
            return;
        };

        let title = self
            .player_info_cache
            .get(&current_key)
            .map(|entry| format!("Player info — {}", entry.display_name))
            .unwrap_or_else(|| "Player info".to_string());

        let mut open = true;
        let mut request_retry = false;

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_min_width(320.0);
                match self.player_info_cache.get(&current_key) {
                    Some(entry) => match &entry.state {
                        PlayerInfoState::Loading => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Fetching profile…");
                            });
                        }
                        PlayerInfoState::Loaded(info) => {
                            self.render_player_info_details(ui, info);
                            if ui.button("Refresh").clicked() {
                                request_retry = true;
                            }
                        }
                        PlayerInfoState::Error(err) => {
                            ui.colored_label(Color32::from_rgb(240, 90, 80), err);
                            if ui.button("Retry").clicked() {
                                request_retry = true;
                            }
                        }
                        PlayerInfoState::NotLoaded => {
                            ui.label("No data fetched yet.");
                            if ui.button("Load").clicked() {
                                request_retry = true;
                            }
                        }
                    },
                    None => {
                        ui.label("No player selected.");
                    }
                }
            });

        if !open {
            self.player_info_window = None;
            return;
        }

        if request_retry {
            let request_key = current_key.clone();
            let mut display = None;
            if let Some(entry) = self.player_info_cache.get_mut(&current_key) {
                entry.state = PlayerInfoState::Loading;
                display = Some(entry.display_name.clone());
            }
            let display_to_use = display.unwrap_or_else(|| current_key.clone());
            self.spawn_player_info_request(request_key, display_to_use);
        }
    }

    fn render_player_info_details(&self, ui: &mut egui::Ui, info: &player_info::PlayerInfo) {
        ui.vertical(|ui| {
            self.render_player_info_field(ui, "Enlisted", info.enlisted.as_deref());
            self.render_player_info_field(ui, "Location", info.location.as_deref());
            self.render_player_info_field(ui, "Fluency", info.fluency.as_deref());
            self.render_player_info_field(
                ui,
                "Main Organization",
                info.main_organization.as_deref(),
            );
        });
    }

    fn render_player_info_field(&self, ui: &mut egui::Ui, label: &str, value: Option<&str>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{}:", label)).color(Color32::from_rgb(180, 180, 180)));
            let text = value.unwrap_or("Unknown");
            ui.label(RichText::new(text).color(Color32::from_rgb(220, 220, 220)));
        });
    }
}

impl eframe::App for LogApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Keep the app ticking so background refresh and worker updates still run when unfocused.
        let wake_interval = self.auto_refresh_interval.min(Duration::from_millis(250));
        ctx.request_repaint_after(wake_interval);

        self.poll_player_info_responses();
        self.maybe_refresh();

        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            egui::Frame::none()
                .fill(Color32::from_rgb(28, 32, 40))
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .rounding(egui::Rounding::same(6.0))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("Version: {}", self.app_version))
                                    .color(Color32::from_rgb(160, 160, 160)),
                            );
                        });
                        ui.add_space(4.0);

                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new("Log file:").color(Color32::from_rgb(210, 210, 210)),
                            );
                            let response =
                                ui.add(egui::TextEdit::singleline(&mut self.file_path_input));
                            if response.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                            {
                                self.reload();
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Browse…").color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_rgb(0, 95, 145)),
                                )
                                .clicked()
                            {
                                let mut dialog = FileDialog::new();
                                if let Some(dir) = self.dialog_start_dir() {
                                    dialog = dialog.set_directory(dir);
                                }
                                if let Some(path) = dialog.pick_file() {
                                    self.set_selected_file(&path);
                                }
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Reload").color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_rgb(70, 70, 70)),
                                )
                                .clicked()
                            {
                                self.reload();
                            }
                        });

                        if let Some(error) = &self.load_error {
                            ui.colored_label(Color32::from_rgb(240, 90, 80), error);
                        }

                        ui.label(
                            RichText::new(
                                "The view refreshes automatically when the selected file changes.",
                            )
                            .color(Color32::from_rgb(160, 160, 160)),
                        );

                        ui.add_space(6.0);

                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(
                                &mut self.filter_show_kills,
                                RichText::new("Show kills").color(Color32::from_rgb(210, 210, 210)),
                            );
                            ui.checkbox(
                                &mut self.filter_show_spawns,
                                RichText::new("Show spawns")
                                    .color(Color32::from_rgb(210, 210, 210)),
                            );
                            ui.checkbox(
                                &mut self.filter_show_corpse,
                                RichText::new("Show corpse toggles")
                                    .color(Color32::from_rgb(210, 210, 210)),
                            );
                            ui.checkbox(
                                &mut self.filter_show_zone_moves,
                                RichText::new("Show zone moves")
                                    .color(Color32::from_rgb(210, 210, 210)),
                            );
                            ui.checkbox(
                                &mut self.filter_show_status_effects,
                                RichText::new("Show status effects")
                                    .color(Color32::from_rgb(210, 210, 210)),
                            );
                            ui.checkbox(
                                &mut self.filter_show_hits,
                                RichText::new("Show hits").color(Color32::from_rgb(210, 210, 210)),
                            );
                            ui.checkbox(
                                &mut self.filter_show_vehicle_destruction,
                                RichText::new("Show vehicle destruction")
                                    .color(Color32::from_rgb(210, 210, 210)),
                            );
                        });

                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new("Ignore player:")
                                    .color(Color32::from_rgb(210, 210, 210)),
                            );
                            let response =
                                ui.add(egui::TextEdit::singleline(&mut self.ignored_player));
                            if response.changed() {
                                self.ignored_player_user_override = true;
                                self.persist_ignored_player();
                            }
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("Clear").color(Color32::WHITE))
                                        .fill(Color32::from_rgb(80, 80, 80)),
                                )
                                .clicked()
                            {
                                self.ignored_player.clear();
                                self.ignored_player_user_override = true;
                                self.persist_ignored_player();
                            }
                        });

                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new("Search:").color(Color32::from_rgb(210, 210, 210)),
                            );
                            ui.add(egui::TextEdit::singleline(&mut self.search_text));
                        });
                    });
                });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let filtered = self.filtered_events();

            let header_text = format!(
                "Showing {} events ({} total parsed)",
                filtered.len(),
                self.events.len()
            );
            ui.label(RichText::new(header_text).color(Color32::from_rgb(200, 200, 200)));

            if filtered.is_empty() {
                ui.label(
                    RichText::new("No events match the current filters.")
                        .color(Color32::from_rgb(200, 200, 200)),
                );
                return;
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                for event in &filtered {
                    let summary = event.summary_line();
                    let (fill, text_color, border) = match &event.kind {
                        EventKind::Kill(_) => (
                            Color32::from_rgb(50, 25, 30),
                            Color32::from_rgb(235, 130, 130),
                            Color32::from_rgb(120, 45, 55),
                        ),
                        EventKind::SpawnReservation(_) => (
                            Color32::from_rgb(24, 36, 52),
                            Color32::from_rgb(130, 185, 245),
                            Color32::from_rgb(55, 95, 150),
                        ),
                        EventKind::CorpseStatus(_) => (
                            Color32::from_rgb(32, 38, 24),
                            Color32::from_rgb(200, 220, 150),
                            Color32::from_rgb(80, 110, 40),
                        ),
                        EventKind::ZoneTransfer(_) => (
                            Color32::from_rgb(36, 30, 48),
                            Color32::from_rgb(190, 160, 235),
                            Color32::from_rgb(90, 70, 150),
                        ),
                        EventKind::StatusEffect(_) => (
                            Color32::from_rgb(44, 28, 24),
                            Color32::from_rgb(245, 180, 140),
                            Color32::from_rgb(130, 70, 40),
                        ),
                        EventKind::Hit(_) => (
                            Color32::from_rgb(25, 45, 30),
                            Color32::from_rgb(160, 240, 160),
                            Color32::from_rgb(60, 120, 70),
                        ),
                        EventKind::VehicleDestruction(_) => (
                            Color32::from_rgb(48, 30, 30),
                            Color32::from_rgb(245, 150, 150),
                            Color32::from_rgb(120, 60, 60),
                        ),
                    };
                    egui::Frame::none()
                        .fill(fill)
                        .stroke(egui::Stroke::new(1.0, border))
                        .rounding(egui::Rounding::same(6.0))
                        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new(summary).color(text_color));
                            let detail_color = Color32::from_rgb(220, 220, 220);
                            for detail in event.detail_lines() {
                                ui.label(RichText::new(detail).color(detail_color));
                            }
                            let players = event.involved_players();
                            if !players.is_empty() {
                                ui.add_space(6.0);
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new("Players:")
                                            .color(Color32::from_rgb(190, 190, 190)),
                                    );
                                    for (index, player) in players.iter().enumerate() {
                                        let response = ui.add(
                                            egui::Label::new(
                                                RichText::new(player.clone())
                                                    .underline()
                                                    .color(Color32::from_rgb(140, 200, 255)),
                                            )
                                            .sense(Sense::click()),
                                        );
                                        if response.clicked() {
                                            self.open_player_info(player);
                                        }
                                        if index + 1 < players.len() {
                                            ui.label(
                                                RichText::new("•")
                                                    .color(Color32::from_rgb(120, 120, 120)),
                                            );
                                        }
                                    }
                                });
                            }
                        });
                    ui.add_space(8.0);
                }
            });
        });
        self.render_player_info_window(ctx);
    }
}

struct ParsedLog {
    events: Vec<PlayerEvent>,
    primary_nickname: Option<String>,
}

fn parse_log(path: &Path) -> Result<ParsedLog, String> {
    let file =
        File::open(path).map_err(|err| format!("Failed to open {}: {}", path.display(), err))?;
    let reader = BufReader::new(file);
    let mut events: Vec<PlayerEvent> = Vec::new();
    let mut primary_nickname = None;
    let mut reader = reader;
    let mut buffer = Vec::new();

    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => break,
            Ok(_) => {
                if buffer.ends_with(&[b'\n']) {
                    buffer.pop();
                    if buffer.ends_with(&[b'\r']) {
                        buffer.pop();
                    }
                }
                let line = String::from_utf8_lossy(&buffer).to_string();
                if primary_nickname.is_none() {
                    if let Some(name) = extract_nickname(&line) {
                        primary_nickname = Some(name);
                    }
                }
                if let Some(event) = parse_line(&line) {
                    let is_duplicate = events
                        .last()
                        .map(|previous| previous.raw == event.raw)
                        .unwrap_or(false);
                    if !is_duplicate {
                        events.push(event);
                    }
                }
            }
            Err(err) => {
                return Err(format!(
                    "Failed to read line from {}: {}",
                    path.display(),
                    err
                ));
            }
        }
    }

    events.sort_by_key(|event| event.timestamp);
    events.reverse();

    Ok(ParsedLog {
        events,
        primary_nickname,
    })
}

fn extract_nickname(line: &str) -> Option<String> {
    let marker = "nickname=\"";
    let start = line.find(marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn parse_line(line: &str) -> Option<PlayerEvent> {
    parse_actor_death(line)
        .or_else(|| parse_spawn_reservation(line))
        .or_else(|| parse_corpse_status(line))
        .or_else(|| parse_zone_transfer(line))
        .or_else(|| parse_status_effect(line))
        .or_else(|| parse_hit_event(line))
        .or_else(|| parse_vehicle_destruction(line))
}

fn parse_actor_death(line: &str) -> Option<PlayerEvent> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"^<(?P<timestamp>[^>]+)>.*?<Actor Death> CActor::Kill: ["'](?P<victim>[^"']+)["']\s\[(?P<victim_id>[^\]]+)\](?: in zone ["'](?P<zone>[^"']+)["'])? killed by ["'](?P<killer>[^"']+)["']\s\[(?P<killer_id>[^\]]+)\](?: using ["'](?P<weapon>[^"']*)["'](?: \[(?P<weapon_class>[^\]]+)\])?)?\s+with damage type ["'](?P<damage>[^"']+)["'].*"#)
            .unwrap()
    });

    let caps = RE.captures(line)?;
    let timestamp = parse_timestamp(caps.name("timestamp")?.as_str())?;
    let victim_name = caps.name("victim")?.as_str().to_string();
    let victim_id = caps.name("victim_id")?.as_str().to_string();
    let zone = caps
        .name("zone")
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();
    let killer_name = caps.name("killer")?.as_str().to_string();
    let killer_id = caps.name("killer_id")?.as_str().to_string();
    let weapon = caps
        .name("weapon")
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();
    let weapon_class = caps
        .name("weapon_class")
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();
    let damage_type = caps
        .name("damage")
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();

    Some(PlayerEvent {
        timestamp,
        kind: EventKind::Kill(KillEvent {
            victim_name,
            victim_id,
            killer_name,
            killer_id,
            weapon,
            weapon_class,
            damage_type,
            zone,
        }),
        raw: line.to_string(),
    })
}

fn parse_spawn_reservation(line: &str) -> Option<PlayerEvent> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"^<(?P<timestamp>[^>]+)>.*?<Spawn Flow>.*?Player ["'](?P<player>[^"']+)["']\s\[(?P<player_id>[^\]]+)\] lost reservation for spawnpoint (?P<spawnpoint>[^\[]+)\s\[(?P<spawn_id>[^\]]+)\] at location (?P<location>[-\d]+)"#)
            .unwrap()
    });

    let caps = RE.captures(line)?;
    let timestamp = parse_timestamp(caps.name("timestamp")?.as_str())?;
    let player_name = caps.name("player")?.as_str().trim().to_string();
    let player_id = caps.name("player_id")?.as_str().to_string();
    let spawn_point = caps
        .name("spawnpoint")
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_default();
    let spawn_id = caps
        .name("spawn_id")
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();
    let location = caps
        .name("location")
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();

    Some(PlayerEvent {
        timestamp,
        kind: EventKind::SpawnReservation(SpawnReservationEvent {
            player_name,
            player_id,
            spawn_point,
            spawn_id,
            location,
        }),
        raw: line.to_string(),
    })
}

fn parse_corpse_status(line: &str) -> Option<PlayerEvent> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"^<(?P<timestamp>[^>]+)>.*?\bPlayer ["'](?P<player>[^"'<>]+)["']\s*(?P<context><[^>]+>)?:\s*IsCorpseEnabled:\s*(?P<enabled>Yes|No)\.?"#)
            .unwrap()
    });

    let caps = RE.captures(line)?;
    let timestamp = parse_timestamp(caps.name("timestamp")?.as_str())?;
    let player_name = caps.name("player")?.as_str().trim().to_string();
    let enabled_raw = caps.name("enabled")?.as_str();
    let corpse_enabled = matches_ignore_case(enabled_raw, "Yes");
    if corpse_enabled {
        return None;
    }
    let context = caps.name("context").and_then(|m| {
        let trimmed = m
            .as_str()
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    Some(PlayerEvent {
        timestamp,
        kind: EventKind::CorpseStatus(CorpseStatusEvent {
            player_name,
            context,
            corpse_enabled,
        }),
        raw: line.to_string(),
    })
}

fn parse_zone_transfer(line: &str) -> Option<PlayerEvent> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"^<(?P<timestamp>[^>]+)>.*?moving zone hosted child id = (?P<child_id>\d+)\s+name\s*=\s*"(?P<player>[^"]+)"\s+to unblock removal of parent id = (?P<parent_id>\d+)\s+name\s*=\s*"(?P<parent_name>[^"]+)"\s+into zone host id = (?P<host_id>\d+)\s+name\s*=\s*"(?P<host_name>[^"]+)""#)
            .unwrap()
    });

    let caps = RE.captures(line)?;
    let timestamp = parse_timestamp(caps.name("timestamp")?.as_str())?;
    let player_name = caps.name("player")?.as_str().to_string();

    if player_name.is_empty() {
        return None;
    }

    Some(PlayerEvent {
        timestamp,
        kind: EventKind::ZoneTransfer(ZoneTransferEvent {
            player_name,
            child_id: caps.name("child_id").map(|m| m.as_str().to_string()),
            parent_id: caps.name("parent_id").map(|m| m.as_str().to_string()),
            parent_name: caps.name("parent_name").map(|m| m.as_str().to_string()),
            host_id: caps.name("host_id").map(|m| m.as_str().to_string()),
            host_name: caps.name("host_name").map(|m| m.as_str().to_string()),
        }),
        raw: line.to_string(),
    })
}

fn parse_status_effect(line: &str) -> Option<PlayerEvent> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"^<(?P<timestamp>[^>]+)>.*?Logged (?P<article>a|an) (?P<stage>start|end) of a status effect!\s*nickname:\s*(?P<nickname>[^,]+),\s*status effect:\s*(?P<effect>.+)"#)
            .unwrap()
    });

    let caps = RE.captures(line)?;
    let timestamp = parse_timestamp(caps.name("timestamp")?.as_str())?;
    let nickname = caps.name("nickname")?.as_str().trim().to_string();

    Some(PlayerEvent {
        timestamp,
        kind: EventKind::StatusEffect(StatusEffectEvent {
            player_name: nickname,
            effect: caps
                .name("effect")
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default(),
            stage: caps
                .name("stage")
                .map(|m| m.as_str().trim().to_ascii_lowercase())
                .unwrap_or_else(|| "start".to_string()),
        }),
        raw: line.to_string(),
    })
}

fn parse_hit_event(line: &str) -> Option<PlayerEvent> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"^<(?P<timestamp>[^>]+)>.*?<Debug Hostility Events>.*?Fake hit FROM (?P<attacker>[^\s]+) TO (?P<target>[^\.]+)\.\s*Being sent to child (?P<child>[\w_-]+)"#)
            .unwrap()
    });

    let caps = RE.captures(line)?;
    let timestamp = parse_timestamp(caps.name("timestamp")?.as_str())?;
    let attacker = caps.name("attacker")?.as_str().to_string();
    let target = caps.name("target")?.as_str().trim().to_string();
    let child = caps
        .name("child")
        .map(|m| m.as_str().trim().to_string())
        .filter(|s| !s.is_empty());

    Some(PlayerEvent {
        timestamp,
        kind: EventKind::Hit(HitEvent {
            attacker,
            target,
            child,
        }),
        raw: line.to_string(),
    })
}

fn parse_vehicle_destruction(line: &str) -> Option<PlayerEvent> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"^<(?P<timestamp>[^>]+)>.*?<Vehicle Destruction>.*?Vehicle '(?P<vehicle>[^']+)'\s*\[(?P<vehicle_id>[^\]]+)\].*?zone '(?P<zone>[^']+)'.*?driven by '(?P<driver>[^']+)'\s*\[(?P<driver_id>[^\]]+)\].*?advanced from destroy level (?P<from>\d+) to (?P<to>\d+) caused by '(?P<attacker>[^']+)'\s*\[(?P<attacker_id>[^\]]+)\]\s*with '(?P<cause>[^']+)'"#)
            .unwrap()
    });

    let caps = RE.captures(line)?;
    let timestamp = parse_timestamp(caps.name("timestamp")?.as_str())?;
    Some(PlayerEvent {
        timestamp,
        kind: EventKind::VehicleDestruction(VehicleDestructionEvent {
            vehicle_name: caps
                .name("vehicle")
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            vehicle_id: caps
                .name("vehicle_id")
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            zone: caps
                .name("zone")
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            driver_name: caps
                .name("driver")
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            driver_id: caps
                .name("driver_id")
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            from_level: caps
                .name("from")
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .unwrap_or_default(),
            to_level: caps
                .name("to")
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .unwrap_or_default(),
            attacker_name: caps
                .name("attacker")
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            attacker_id: caps
                .name("attacker_id")
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
            cause: caps
                .name("cause")
                .map(|m| m.as_str().to_string())
                .unwrap_or_default(),
        }),
        raw: line.to_string(),
    })
}

fn matches_ignore_case(value: &str, expected: &str) -> bool {
    value.eq_ignore_ascii_case(expected)
}

fn format_status_stage(stage: &str, effect: &str) -> String {
    if matches_ignore_case(stage, "start") {
        format!("started {}", effect)
    } else if matches_ignore_case(stage, "end") {
        format!("ended {}", effect)
    } else {
        format!("{} {}", stage, effect)
    }
}

fn describe_destroy_levels(from: u32, to: u32) -> &'static str {
    match (from, to) {
        (0, 1) => "soft kill",
        (1, 2) => "hard kill",
        (start, end) if end > start => "destroyed",
        _ => "changed",
    }
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}

#[derive(Clone)]
struct PlayerEvent {
    timestamp: DateTime<Utc>,
    kind: EventKind,
    raw: String,
}

impl PlayerEvent {
    fn summary_line(&self) -> String {
        let ts = self.timestamp.format("%Y-%m-%d %H:%M:%S");
        match &self.kind {
            EventKind::Kill(event) => {
                let weapon_display = if event.weapon.is_empty() {
                    "unknown weapon".to_string()
                } else if event.weapon_class.is_empty() {
                    event.weapon.clone()
                } else {
                    format!("{} ({})", event.weapon, event.weapon_class)
                };
                format!(
                    "{} | Kill | {} → {} with {}",
                    ts, event.killer_name, event.victim_name, weapon_display
                )
            }
            EventKind::SpawnReservation(event) => format!(
                "{} | Spawn | {} lost {}",
                ts, event.player_name, event.spawn_point
            ),
            EventKind::CorpseStatus(event) => format!(
                "{} | Corpse | {} corpse {}",
                ts,
                event.player_name,
                if event.corpse_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            ),
            EventKind::ZoneTransfer(event) => format!(
                "{} | Zone | {} → {}",
                ts,
                event.player_name,
                event
                    .host_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("unknown destination")
            ),
            EventKind::StatusEffect(event) => format!(
                "{} | Status | {} {}",
                ts,
                event.player_name,
                format_status_stage(&event.stage, &event.effect)
            ),
            EventKind::Hit(event) => {
                format!("{} | Hit | {} → {}", ts, event.attacker, event.target)
            }
            EventKind::VehicleDestruction(event) => format!(
                "{} | Vehicle | {} {} ({})",
                ts,
                event.attacker_name,
                describe_destroy_levels(event.from_level, event.to_level),
                event.vehicle_name
            ),
        }
    }

    fn detail_lines(&self) -> Vec<String> {
        match &self.kind {
            EventKind::Kill(event) => {
                let mut lines = Vec::new();
                lines.push(format!(
                    "Victim: {} [{}] in {}",
                    event.victim_name, event.victim_id, event.zone
                ));
                lines.push(format!(
                    "Killer: {} [{}]",
                    event.killer_name, event.killer_id
                ));
                if !event.damage_type.is_empty() {
                    lines.push(format!("Damage type: {}", event.damage_type));
                }
                lines
            }
            EventKind::SpawnReservation(event) => vec![
                format!("Player: {} [{}]", event.player_name, event.player_id),
                format!("Spawn point: {}", event.spawn_point),
                format!("Spawn ID: {}", event.spawn_id),
                format!("Location: {}", event.location),
            ],
            EventKind::CorpseStatus(event) => {
                let mut lines = vec![format!(
                    "Corpse state: {}",
                    if event.corpse_enabled {
                        "Enabled"
                    } else {
                        "Disabled"
                    }
                )];
                if let Some(context) = event.context.as_ref() {
                    if !context.is_empty() {
                        lines.push(format!("Context: {}", context));
                    }
                }
                lines
            }
            EventKind::ZoneTransfer(event) => {
                let mut lines = vec![format!("Player: {}", event.player_name)];
                if let Some(child_id) = event.child_id.as_ref() {
                    lines.push(format!("Child ID: {}", child_id));
                }
                if let Some(parent_name) = event.parent_name.as_ref() {
                    if !parent_name.is_empty() {
                        lines.push(format!("Parent: {}", parent_name));
                    }
                }
                if let Some(parent_id) = event.parent_id.as_ref() {
                    lines.push(format!("Parent ID: {}", parent_id));
                }
                if let Some(host_name) = event.host_name.as_ref() {
                    if !host_name.is_empty() {
                        lines.push(format!("Zone host: {}", host_name));
                    }
                }
                if let Some(host_id) = event.host_id.as_ref() {
                    lines.push(format!("Zone host ID: {}", host_id));
                }
                lines
            }
            EventKind::StatusEffect(event) => vec![
                format!("Player: {}", event.player_name),
                format!("Status effect: {}", event.effect),
                format!(
                    "Stage: {}",
                    if matches_ignore_case(&event.stage, "start") {
                        "Start".to_string()
                    } else if matches_ignore_case(&event.stage, "end") {
                        "End".to_string()
                    } else {
                        event.stage.clone()
                    }
                ),
            ],
            EventKind::Hit(event) => {
                let mut lines = vec![format!("Attacker: {}", event.attacker)];
                lines.push(format!("Target: {}", event.target));
                if let Some(child) = event.child.as_ref() {
                    lines.push(format!("Child channel: {}", child));
                }
                lines
            }
            EventKind::VehicleDestruction(event) => {
                let mut lines = Vec::new();
                lines.push(format!(
                    "Vehicle: {} [{}]",
                    event.vehicle_name, event.vehicle_id
                ));
                lines.push(format!(
                    "Destroy level: {} → {} ({})",
                    event.from_level,
                    event.to_level,
                    describe_destroy_levels(event.from_level, event.to_level)
                ));
                lines.push(format!(
                    "Attacker: {} [{}] via {}",
                    event.attacker_name, event.attacker_id, event.cause
                ));
                if !event.zone.is_empty() {
                    lines.push(format!("Zone: {}", event.zone));
                }
                if !event.driver_name.is_empty() {
                    lines.push(format!(
                        "Driver: {} [{}]",
                        event.driver_name, event.driver_id
                    ));
                }
                lines
            }
        }
    }

    fn matches_search(&self, needle: &str) -> bool {
        let needle = needle.trim();
        if needle.is_empty() {
            return true;
        }
        self.search_blob().contains(needle)
    }

    fn should_ignore(&self, ignored: &str) -> bool {
        let trimmed = ignored.trim();
        if trimmed.is_empty() {
            return false;
        }
        match &self.kind {
            EventKind::Kill(event) => {
                event.killer_name.eq_ignore_ascii_case(trimmed)
                    && !event.victim_name.eq_ignore_ascii_case(trimmed)
            }
            EventKind::SpawnReservation(event) => event.player_name.eq_ignore_ascii_case(trimmed),
            EventKind::CorpseStatus(event) => event.player_name.eq_ignore_ascii_case(trimmed),
            EventKind::ZoneTransfer(event) => event.player_name.eq_ignore_ascii_case(trimmed),
            EventKind::StatusEffect(event) => event.player_name.eq_ignore_ascii_case(trimmed),
            EventKind::Hit(event) => event.attacker.eq_ignore_ascii_case(trimmed),
            EventKind::VehicleDestruction(event) => {
                event.attacker_name.eq_ignore_ascii_case(trimmed)
                    || (!event.driver_name.is_empty()
                        && event.driver_name.eq_ignore_ascii_case(trimmed))
            }
        }
    }

    fn search_blob(&self) -> String {
        let mut blob = self.summary_line().to_lowercase();
        for line in self.detail_lines() {
            blob.push('\n');
            blob.push_str(&line.to_lowercase());
        }
        blob.push('\n');
        blob.push_str(&self.raw.to_lowercase());
        blob
    }

    fn participants(&self) -> Vec<String> {
        match &self.kind {
            EventKind::Kill(event) => vec![
                event.killer_name.to_lowercase(),
                event.victim_name.to_lowercase(),
            ],
            EventKind::SpawnReservation(event) => vec![event.player_name.to_lowercase()],
            EventKind::CorpseStatus(event) => vec![event.player_name.to_lowercase()],
            EventKind::ZoneTransfer(event) => vec![event.player_name.to_lowercase()],
            EventKind::StatusEffect(event) => vec![event.player_name.to_lowercase()],
            EventKind::Hit(event) => vec![event.attacker.to_lowercase()],
            EventKind::VehicleDestruction(event) => {
                let mut names = vec![event.attacker_name.to_lowercase()];
                if !event.driver_name.is_empty() {
                    names.push(event.driver_name.to_lowercase());
                }
                names
            }
        }
    }

    fn involved_players(&self) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut names = Vec::new();
        let mut push_name = |name: &str| {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return;
            }
            if trimmed.eq_ignore_ascii_case("unknown") {
                return;
            }
            let key = trimmed.to_ascii_lowercase();
            if seen.insert(key) {
                names.push(trimmed.to_string());
            }
        };

        match &self.kind {
            EventKind::Kill(event) => {
                push_name(&event.killer_name);
                push_name(&event.victim_name);
            }
            EventKind::SpawnReservation(event) => {
                push_name(&event.player_name);
            }
            EventKind::CorpseStatus(event) => {
                push_name(&event.player_name);
            }
            EventKind::ZoneTransfer(event) => {
                push_name(&event.player_name);
            }
            EventKind::StatusEffect(event) => {
                push_name(&event.player_name);
            }
            EventKind::Hit(event) => {
                push_name(&event.attacker);
            }
            EventKind::VehicleDestruction(event) => {
                push_name(&event.attacker_name);
                push_name(&event.driver_name);
            }
        }

        names
    }
}

#[derive(Clone)]
enum EventKind {
    Kill(KillEvent),
    SpawnReservation(SpawnReservationEvent),
    CorpseStatus(CorpseStatusEvent),
    ZoneTransfer(ZoneTransferEvent),
    StatusEffect(StatusEffectEvent),
    Hit(HitEvent),
    VehicleDestruction(VehicleDestructionEvent),
}

#[derive(Clone)]
struct KillEvent {
    victim_name: String,
    victim_id: String,
    killer_name: String,
    killer_id: String,
    weapon: String,
    weapon_class: String,
    damage_type: String,
    zone: String,
}

#[derive(Clone)]
struct SpawnReservationEvent {
    player_name: String,
    player_id: String,
    spawn_point: String,
    spawn_id: String,
    location: String,
}

#[derive(Clone)]
struct CorpseStatusEvent {
    player_name: String,
    context: Option<String>,
    corpse_enabled: bool,
}

#[derive(Clone)]
struct ZoneTransferEvent {
    player_name: String,
    child_id: Option<String>,
    parent_id: Option<String>,
    parent_name: Option<String>,
    host_id: Option<String>,
    host_name: Option<String>,
}

#[derive(Clone)]
struct StatusEffectEvent {
    player_name: String,
    effect: String,
    stage: String,
}

#[derive(Clone)]
struct HitEvent {
    attacker: String,
    target: String,
    child: Option<String>,
}

#[derive(Clone)]
struct VehicleDestructionEvent {
    vehicle_name: String,
    vehicle_id: String,
    zone: String,
    driver_name: String,
    driver_id: String,
    from_level: u32,
    to_level: u32,
    attacker_name: String,
    attacker_id: String,
    cause: String,
}

struct PlayerInfoEntry {
    display_name: String,
    state: PlayerInfoState,
}

enum PlayerInfoState {
    NotLoaded,
    Loading,
    Loaded(player_info::PlayerInfo),
    Error(String),
}

struct PlayerInfoResponse {
    key: String,
    display_name: String,
    result: PlayerInfoResult,
}

enum PlayerInfoResult {
    Success(player_info::PlayerInfo),
    Error(String),
}

fn canonical_player_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn resolve_input_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    let normalized = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(trimmed);

    let trimmed = normalized.trim();

    if trimmed.is_empty() {
        return PathBuf::new();
    }

    if trimmed == "~" {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }

    if trimmed.starts_with("~/") || trimmed.starts_with("~\\") {
        if let Some(mut home) = env::var_os("HOME").map(PathBuf::from) {
            let remainder = trimmed[2..].replace('\\', "/");
            push_path_components(&mut home, &remainder);
            return home;
        }
    }

    if trimmed.len() >= 3
        && trimmed.as_bytes()[1] == b':'
        && (trimmed.as_bytes()[2] == b'\\' || trimmed.as_bytes()[2] == b'/')
    {
        let drive = trimmed.chars().next().unwrap();
        let remainder = trimmed[2..].replace('\\', "/");
        let remainder = remainder.trim_start_matches('/').to_string();
        if drive.eq_ignore_ascii_case(&'z') {
            let mut path = PathBuf::from("/");
            push_path_components(&mut path, &remainder);
            return path;
        }
        if let Some(mut base) = wine_drive_base(drive) {
            push_path_components(&mut base, &remainder);
            return base;
        }
    }

    PathBuf::from(trimmed)
}

fn wine_drive_base(drive: char) -> Option<PathBuf> {
    let lower = drive.to_ascii_lowercase();
    let prefix = env::var_os("WINEPREFIX").map(PathBuf::from).or_else(|| {
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|p| p.join(".wine"))
    });
    prefix.map(|mut base| {
        base.push(format!("drive_{}", lower));
        base
    })
}

fn push_path_components(base: &mut PathBuf, components: &str) {
    for part in components.split('/') {
        if part.is_empty() {
            continue;
        }
        base.push(part);
    }
}
