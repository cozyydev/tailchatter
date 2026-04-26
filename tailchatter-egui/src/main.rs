#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use eframe::egui;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const DEFAULT_PORT: u16 = 42069;

fn state_file_path() -> PathBuf {
    let mut path = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("tailchatter");
    path.push("state.json");
    path
}

fn now_hms() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secs_in_day = secs % 86_400;
    let hour = secs_in_day / 3_600;
    let minute = (secs_in_day % 3_600) / 60;
    let second = secs_in_day % 60;
    format!("{hour:02}:{minute:02}:{second:02}")
}

#[derive(PartialEq, Clone, Copy)]
pub enum ChatMode {
    Login,
    Chat,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct PersistedState {
    nick: String,
    server_ip: String,
    server_port: u16,
    was_logged_out: bool,
    messages: Vec<(String, String, String)>,
}

pub struct ChatApp {
    mode: ChatMode,
    nick: String,
    server_ip: String,
    server_port: u16,
    start_server: bool,
    messages: Vec<(String, String, String)>,
    online_users: Vec<String>,
    room_name: String,
    input_message: String,
    error_message: String,
    msg_receiver: Option<Receiver<String>>,
    outgoing_queue: Option<Arc<Mutex<VecDeque<String>>>>,
    was_logged_out: bool,
    needs_reconnect: bool,
    save_counter: u32,
}

impl Default for ChatApp {
    fn default() -> Self {
        Self {
            mode: ChatMode::Login,
            nick: String::new(),
            server_ip: String::new(),
            server_port: DEFAULT_PORT,
            start_server: false,
            messages: Vec::new(),
            online_users: Vec::new(),
            room_name: String::from("Chat Room"),
            input_message: String::new(),
            error_message: String::new(),
            msg_receiver: None,
            outgoing_queue: None,
            was_logged_out: false,
            needs_reconnect: false,
            save_counter: 0,
        }
    }
}

impl eframe::App for ChatApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let theme = Theme::default();

        ctx.style_mut(|style| {
            style.visuals = theme.dark_mode();
        });

        // Request repaint every 100ms to keep messages flowing
        ctx.request_repaint_after(std::time::Duration::from_millis(100));

        // Poll for incoming messages
        if let Some(ref rx) = self.msg_receiver {
            while let Ok(msg) = rx.try_recv() {
                // Parse message: either "user: message" or system messages
                // Check for "Online (X):" format BEFORE splitting
                let is_online_list = msg.starts_with("Online");
                let is_join_leave = msg.contains(" has joined") || msg.contains(" has left");
                
                let (from, body) = if msg.contains(": ") && !is_online_list {
                    let parts: Vec<&str> = msg.splitn(2, ": ").collect();
                    if parts.len() == 2 {
                        (parts[0].to_string(), parts[1].to_string())
                    } else {
                        ("System".to_string(), msg.clone())
                    }
                } else {
                    ("System".to_string(), msg.clone())
                };

                // Handle system messages
                if is_online_list {
                    // Parse online users list (show in sidebar, don't display in chat)
                    if let Some(users_part) = msg.splitn(2, ':').nth(1) {
                        self.online_users.clear();
                        for name in users_part.split(',') {
                            let name = name.trim();
                            if !name.is_empty() {
                                self.online_users.push(name.to_string());
                            }
                        }
                    }
                } else if from == self.nick && (body.contains("has joined") || body.contains("has left")) {
                    // Don't show self joining/leaving
                } else if is_join_leave {
                    // Skip join/leave messages
                } else {
                    self.messages.push((from, body, now_hms()));
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| match self.mode {
            ChatMode::Login => self.login_ui(ui, &theme),
            ChatMode::Chat => self.chat_ui(ui, &theme, ctx),
        });
    }
}

impl ChatApp {
    fn login_ui(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        let input_bg = egui::Color32::from_rgb(40, 42, 54);

        // Center the content vertically
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.heading(
                egui::RichText::new("TailChatter")
                    .size(36.0)
                    .color(theme.title),
            );
            ui.add_space(15.0);
            ui.label(egui::RichText::new("Enter your details to join").color(theme.muted));
        });

        ui.add_space(40.0);

        // Form in a container
        egui::Frame::default()
            .fill(input_bg)
            .inner_margin(30.0)
            .rounding(10.0)
            .show(ui, |ui| {
                // Stack inputs vertically, centered
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Your Handle:")
                            .color(theme.muted)
                            .size(14.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.nick)
                            .desired_width(280.0)
                            .text_color(theme.text)
                            .desired_rows(1)
                            .margin(egui::vec2(12.0, 10.0)),
                    );

                    ui.add_space(15.0);

                    ui.label(
                        egui::RichText::new("Server IP:")
                            .color(theme.muted)
                            .size(14.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.server_ip)
                            .desired_width(280.0)
                            .text_color(theme.text)
                            .desired_rows(1)
                            .margin(egui::vec2(12.0, 10.0)),
                    );

                    ui.add_space(15.0);

                    ui.label(egui::RichText::new("Port:").color(theme.muted).size(14.0));
                    ui.add(egui::DragValue::new(&mut self.server_port).range(1..=65535));

                    ui.add_space(15.0);

                    ui.checkbox(
                        &mut self.start_server,
                        egui::RichText::new("Start local server").color(theme.muted),
                    );
                });
            });

        ui.add_space(30.0);

        if !self.error_message.is_empty() {
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(&self.error_message).color(theme.error));
            });
            ui.add_space(15.0);
        }

        let button_id = egui::Id::new("join_button");

        ui.vertical_centered(|ui| {
            let button = egui::Button::new(egui::RichText::new("Join Chat").size(20.0))
                .min_size(egui::vec2(200.0, 45.0));
            let button = ui.add(button);
            if button.clicked()
                || (ui.input(|i| i.key_pressed(egui::Key::Enter)) && self.mode == ChatMode::Login)
            {
                self.error_message.clear();

                if self.nick.len() < 2 || self.nick.len() > 24 {
                    self.error_message = "Nick must be 2-24 characters".to_string();
                } else if !self
                    .nick
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    self.error_message = "Only letters, numbers, _ and - allowed".to_string();
                } else if self.server_ip.is_empty() && !self.start_server {
                    self.error_message = "Enter server IP or start local server".to_string();
                } else {
                    // Start connection in background thread
                    let ip = self.server_ip.clone();
                    let port = self.server_port;
                    let nick = self.nick.clone();
                    let conn_msg = format!("Connecting to {}:{}...", ip, port);

                    // Create channels for communication
                    let (tx, rx) = mpsc::channel();
                    let outgoing = Arc::new(Mutex::new(VecDeque::new()));
                    self.msg_receiver = Some(rx);
                    self.outgoing_queue = Some(Arc::clone(&outgoing));

                    // Spawn background thread for TCP
                    thread::spawn(move || {
                        if let Err(e) = run_tcp_client_threaded(&ip, port, &nick, tx, outgoing) {
                            eprintln!("Connection error: {}", e);
                        }
                    });

                    self.mode = ChatMode::Chat;
                    self.was_logged_out = false;
                    save_state(&self.nick, &self.server_ip, self.server_port, false, &self.messages);
                    self.online_users.push(self.nick.clone());
                    self.messages
                        .push(("System".to_string(), conn_msg, now_hms()));
                }
            }
        });
    }

    fn chat_ui(&mut self, ui: &mut egui::Ui, theme: &Theme, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("TailChatter").color(theme.title));
                ui.separator();
                ui.label(egui::RichText::new(&self.room_name).color(theme.muted));
                ui.separator();
                let count = self.online_users.len();
                ui.label(egui::RichText::new(format!("{count} online")).color(theme.status));
                ui.separator();
                ui.label(egui::RichText::new(&format!("Logged in as: {}", self.nick)).color(theme.self_name));
                if ui.small_button("Logout").clicked() {
                    self.logout();
                }
            });
        });

        egui::SidePanel::left("sidebar")
            .default_width(150.0)
            .show(ctx, |ui| {
                ui.heading(egui::RichText::new("Users").size(14.0).color(theme.muted));
                ui.separator();
                for user in &self.online_users {
                    let color = if user == &self.nick {
                        theme.self_name
                    } else {
                        theme.color_for_name(user)
                    };
                    ui.label(egui::RichText::new(user).color(color));
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (from, body, _time) in &self.messages {
                        let color = if from == "System" {
                            theme.system
                        } else if from == "Error" {
                            theme.error
                        } else if from == &self.nick {
                            theme.self_name
                        } else {
                            theme.color_for_name(from)
                        };
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(from).color(color).strong());
                            ui.label(egui::RichText::new(body).color(theme.text));
                        });
                        ui.add_space(3.0);
                    }
                });
        });

        let input_id = egui::Id::new("chat_input");
        let send_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

        egui::TopBottomPanel::bottom("input").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let available_width = ui.available_width();
                ui.add(
                    egui::TextEdit::singleline(&mut self.input_message)
                        .id(input_id)
                        .desired_width(available_width - 80.0)
                        .margin(egui::vec2(12.0, 10.0)),
                );
                if ui.button("Send").clicked() {
                    if !self.input_message.is_empty() {
                        if let Some(ref queue) = self.outgoing_queue {
                            queue.lock().unwrap().push_back(self.input_message.clone());
                        }
                        self.input_message.clear();
                    }
                }
            });
        });

        // Handle Enter key outside the panel to keep focus
        if send_pressed && !self.input_message.is_empty() {
            if let Some(ref queue) = self.outgoing_queue {
                queue.lock().unwrap().push_back(self.input_message.clone());
            }
            self.input_message.clear();
            ctx.memory_mut(|mem| {
                mem.request_focus(input_id);
            });
        }
    }

    fn logout(&mut self) {
        if let Some(ref queue) = self.outgoing_queue {
            queue.lock().unwrap().push_back("/quit".to_string());
        }
        self.messages.clear();
        self.online_users.clear();
        self.input_message.clear();
        self.mode = ChatMode::Login;
    }
}

struct Theme {
    bg: egui::Color32,
    title: egui::Color32,
    text: egui::Color32,
    muted: egui::Color32,
    status: egui::Color32,
    error: egui::Color32,
    self_name: egui::Color32,
    system: egui::Color32,
    name_palette: [egui::Color32; 6],
}

impl Theme {
    fn default() -> Self {
        Self {
            bg: egui::Color32::from_rgb(40, 42, 54),
            title: egui::Color32::from_rgb(189, 147, 249),
            text: egui::Color32::from_rgb(248, 248, 242),
            muted: egui::Color32::from_rgb(98, 114, 164),
            status: egui::Color32::from_rgb(80, 250, 123),
            error: egui::Color32::from_rgb(255, 85, 85),
            self_name: egui::Color32::from_rgb(0, 209, 171),
            system: egui::Color32::from_rgb(139, 233, 253),
            name_palette: [
                egui::Color32::from_rgb(255, 184, 108),
                egui::Color32::from_rgb(255, 121, 198),
                egui::Color32::from_rgb(139, 233, 253),
                egui::Color32::from_rgb(189, 147, 249),
                egui::Color32::from_rgb(241, 250, 140),
                egui::Color32::from_rgb(80, 250, 123),
            ],
        }
    }

    fn dark_mode(&self) -> egui::Visuals {
        let mut visuals = egui::Visuals::dark();
        visuals.widgets.noninteractive.fg_stroke.color = self.text;
        visuals.widgets.inactive.fg_stroke.color = self.text;
        visuals.widgets.hovered.fg_stroke.color = self.text;
        visuals.widgets.active.fg_stroke.color = self.text;
        visuals.widgets.open.fg_stroke.color = self.text;
        visuals.panel_fill = self.bg;
        visuals.window_fill = self.bg;
        visuals
    }

    fn color_for_name(&self, name: &str) -> egui::Color32 {
        let mut hash = 0u64;
        for c in name.bytes() {
            hash = hash.wrapping_add(c as u64).wrapping_mul(31);
        }
        self.name_palette[(hash as usize) % self.name_palette.len()]
    }
}

fn load_state() -> (ChatMode, String, String, u16, bool, Vec<(String, String, String)>) {
    let path = state_file_path();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(state) = serde_json::from_str::<PersistedState>(&content) {
                if !state.was_logged_out && !state.nick.is_empty() {
                    return (ChatMode::Chat, state.nick, state.server_ip, state.server_port, false, state.messages);
                }
            }
        }
    }
    (ChatMode::Login, String::new(), String::new(), DEFAULT_PORT, false, Vec::new())
}

fn save_state(nick: &str, server_ip: &str, server_port: u16, was_logged_out: bool, messages: &[(String, String, String)]) {
    let path = state_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let state = PersistedState {
        nick: nick.to_string(),
        server_ip: server_ip.to_string(),
        server_port,
        was_logged_out,
        messages: messages.to_vec(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&state) {
        let _ = fs::write(&path, json);
    }
}

fn run_tcp_client_threaded(
    ip: &str,
    port: u16,
    nick: &str,
    tx: Sender<String>,
    outgoing: Arc<Mutex<VecDeque<String>>>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        let addr = format!("{}:{}", ip, port);
        println!("Connecting to {}...", addr);

        let stream = TcpStream::connect(&addr).await?;
        println!("Connected!");

        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        writer.write_all(format!("{}\n", nick).as_bytes()).await?;
        writer.flush().await?;

        loop {
            tokio::select! {
                result = lines.next_line() => {
                    match result {
                        Ok(Some(line)) => {
                            let _ = tx.send(line);
                        }
                        Ok(None) | Err(_) => {
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(50)) => {
                    let mut queue = outgoing.lock().unwrap();
                    while let Some(msg) = queue.pop_front() {
                        if let Err(e) = writer.write_all(format!("{}\n", msg).as_bytes()).await {
                            eprintln!("Send error: {}", e);
                            break;
                        }
                        let _ = writer.flush().await;
                    }
                }
            }
        }

        Ok(())
    })
}

fn run_tcp_client(ip: &str, port: u16, nick: &str) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        let addr = format!("{}:{}", ip, port);
        println!("Connecting to {}...", addr);

        let stream = TcpStream::connect(&addr).await?;
        println!("Connected!");

        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        // Send nickname
        writer.write_all(format!("{}\n", nick).as_bytes()).await?;
        writer.flush().await?;

        // Read and print each line - this acts as our "loop" to keep connection alive
        while let Ok(Some(line)) = lines.next_line().await {
            println!("Received: {}", line);
        }

        Ok(())
    })
}

fn main() -> Result<(), eframe::Error> {
    let icon = eframe::icon_data::from_png_bytes(&include_bytes!("../icon.png")[..])
        .expect("Failed to load icon");

    let options = eframe::NativeOptions {
        default_theme: eframe::Theme::Dark,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 650.0])
            .with_min_inner_size([700.0, 500.0])
            .with_title("TailChatter")
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "TailChatter",
        options,
        Box::new(|_cc| Ok(Box::new(ChatApp::default()))),
    )
}
