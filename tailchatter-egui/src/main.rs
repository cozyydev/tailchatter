#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod server;

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

#[derive(PartialEq, Clone, Copy, Default)]
pub enum LoginTab {
    #[default]
    Connect,
    Server,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct PersistedState {
    nick: String,
    server_ip: String,
    server_port: u16,
    was_logged_out: bool,
    messages: Vec<(String, String, String)>,
    server_started: bool,
}

pub struct ChatApp {
    mode: ChatMode,
    login_tab: LoginTab,
    nick: String,
    server_ip: String,
    server_port: u16,
    local_server_port: u16,
    server_started: bool,
    reconnect_to_local: bool,
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
            login_tab: LoginTab::Connect,
            nick: String::new(),
            server_ip: String::new(),
            server_port: DEFAULT_PORT,
            local_server_port: DEFAULT_PORT,
            server_started: false,
            reconnect_to_local: false,
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
                } else if body.contains("Enter your handle") {
                    // Skip - login screen handles this
                } else if from == self.nick
                    && (body.contains("has joined") || body.contains("has left"))
                {
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

        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.heading(
                egui::RichText::new("TailChatter")
                    .size(48.0)
                    .strong()
                    .color(theme.title),
            );
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Connect To Or Start A Server").color(theme.muted));
        });

        ui.add_space(30.0);

        egui::Frame::default()
            .fill(egui::Color32::TRANSPARENT)
            .inner_margin(0.0)
            .show(ui, |ui| {
                let total_width = 200.0 * 2.0 + 10.0;
                egui::Frame::default()
                    .fill(egui::Color32::TRANSPARENT)
                    .outer_margin(egui::Margin {
                        left: (ui.available_width() - total_width) / 2.0,
                        right: 0.0,
                        top: 0.0,
                        bottom: 0.0,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let btn = egui::Button::new(
                                egui::RichText::new("Connect To Existing Server").size(16.0),
                            )
                            .min_size(egui::vec2(200.0, 40.0));
                            if ui.add(btn).clicked() {
                                self.login_tab = LoginTab::Connect;
                            }

                            ui.add_space(10.0);

                            let btn = egui::Button::new(
                                egui::RichText::new("Create A Server").size(16.0),
                            )
                            .min_size(egui::vec2(200.0, 40.0));
                            if ui.add(btn).clicked() {
                                self.login_tab = LoginTab::Server;
                            }
                        });
                    });
            });

        ui.add_space(20.0);

        egui::Frame::default()
            .fill(input_bg)
            .inner_margin(30.0)
            .rounding(10.0)
            .show(ui, |ui| match self.login_tab {
                LoginTab::Connect => self.connect_tab_ui(ui, theme),
                LoginTab::Server => self.server_tab_ui(ui, theme),
            });

        ui.add_space(30.0);

        if !self.error_message.is_empty() {
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(&self.error_message).color(theme.error));
            });
            ui.add_space(15.0);
        }
    }

    fn connect_tab_ui(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Connect To An Existing Server")
                    .size(22.0)
                    .color(theme.title),
            );
            ui.add_space(15.0);

            ui.label(
                egui::RichText::new("Your Handle:")
                    .color(theme.muted)
                    .size(16.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.nick)
                    .desired_width(280.0)
                    .text_color(theme.text)
                    .desired_rows(1)
                    .margin(egui::vec2(12.0, 10.0)),
            );

            ui.add_space(15.0);

            if self.server_started {
                ui.label(
                    egui::RichText::new("Server IP:")
                        .color(theme.muted)
                        .size(16.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.server_ip)
                        .desired_width(280.0)
                        .text_color(theme.muted)
                        .desired_rows(1)
                        .margin(egui::vec2(12.0, 10.0)),
                );

                ui.add_space(15.0);

                ui.label(egui::RichText::new("Port:").color(theme.muted).size(16.0));
                let port_str = self.local_server_port.to_string();
                let mut port_string = port_str;
                ui.add(
                    egui::TextEdit::singleline(&mut port_string)
                        .desired_width(280.0)
                        .text_color(theme.muted)
                        .desired_rows(1)
                        .margin(egui::vec2(12.0, 10.0)),
                );
                if let Ok(port) = port_string.parse() {
                    self.local_server_port = port;
                }
            } else {
                ui.label(
                    egui::RichText::new("Server IP:")
                        .color(theme.muted)
                        .size(16.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.server_ip)
                        .desired_width(280.0)
                        .text_color(theme.text)
                        .desired_rows(1)
                        .margin(egui::vec2(12.0, 10.0)),
                );

                ui.add_space(15.0);

                ui.label(egui::RichText::new("Port:").color(theme.muted).size(16.0));
                let port_str = self.server_port.to_string();
                let mut port_string = port_str;
                ui.add(
                    egui::TextEdit::singleline(&mut port_string)
                        .desired_width(280.0)
                        .text_color(theme.text)
                        .desired_rows(1)
                        .margin(egui::vec2(12.0, 10.0)),
                );
                if let Ok(port) = port_string.parse() {
                    self.server_port = port;
                }
            }
        });

        ui.add_space(30.0);

        ui.add_space(30.0);

        let _button_id = egui::Id::new("join_button");

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
                } else if self.server_ip.is_empty() {
                    self.error_message = "Enter server IP".to_string();
                } else {
                    let ip = self.server_ip.clone();
                    let port = self.server_port;
                    let nick = self.nick.clone();
                    let conn_msg = format!("Connecting to {}:{}...", ip, port);

                    let (tx, rx) = mpsc::channel();
                    let outgoing = Arc::new(Mutex::new(VecDeque::new()));
                    self.msg_receiver = Some(rx);
                    self.outgoing_queue = Some(Arc::clone(&outgoing));

                    thread::spawn(move || {
                        if let Err(e) = run_tcp_client_threaded(&ip, port, &nick, tx, outgoing) {
                            eprintln!("Connection error: {}", e);
                        }
                    });

                    self.mode = ChatMode::Chat;
                    self.was_logged_out = false;
                    save_state(
                        &self.nick,
                        &self.server_ip,
                        self.server_port,
                        false,
                        &self.messages,
                        false,
                    );
                    self.online_users.push(self.nick.clone());
                    self.messages
                        .push(("System".to_string(), conn_msg, now_hms()));
                }
            }
        });
    }

    fn server_tab_ui(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        let server_running = self.server_started && self.mode == ChatMode::Login;
        
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Start Your Own Server")
                    .size(22.0)
                    .color(theme.title),
            );
            ui.add_space(15.0);

            ui.label(
                egui::RichText::new("Your Handle:")
                    .color(theme.muted)
                    .size(16.0),
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
                    .size(16.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.server_ip)
                    .desired_width(280.0)
                    .text_color(if server_running { theme.muted } else { theme.text })
                    .desired_rows(1)
                    .margin(egui::vec2(12.0, 10.0)),
            );

            ui.add_space(15.0);

            ui.label(
                egui::RichText::new("Server Port:")
                    .color(theme.muted)
                    .size(16.0),
            );
            let port_str = self.local_server_port.to_string();
            let mut port_string = port_str;
            ui.add(
                egui::TextEdit::singleline(&mut port_string)
                    .desired_width(280.0)
                    .text_color(if server_running { theme.muted } else { theme.text })
                    .desired_rows(1)
                    .margin(egui::vec2(12.0, 10.0)),
            );
            if let Ok(port) = port_string.parse() {
                self.local_server_port = port;
            }

            if self.server_started {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Server is running").color(theme.status));
            }
        });

        ui.add_space(30.0);

        ui.vertical_centered(|ui| {
            let btn_text = if self.server_started && self.mode == ChatMode::Login {
                "Rejoin Chat"
            } else if self.server_started {
                "Stop Server"
            } else {
                "Start Server & Join Chat"
            };
            let button = egui::Button::new(egui::RichText::new(btn_text).size(20.0))
                .min_size(egui::vec2(200.0, 45.0));
            let button = ui.add(button);
            if button.clicked() {
                self.error_message.clear();

                if self.server_started && self.mode == ChatMode::Login {
                    if self.nick.len() < 2 || self.nick.len() > 24 {
                        self.error_message = "Nick must be 2-24 characters".to_string();
                    } else if !self.nick.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                        self.error_message = "Only letters, numbers, _ and - allowed".to_string();
                    } else {
                        let ip = self.server_ip.clone();
                        let port = self.local_server_port;
                        let nick = self.nick.clone();
                        let conn_msg = format!("Reconnecting to local server on port {}...", port);

                        let (tx, rx) = mpsc::channel();
                        let outgoing = Arc::new(Mutex::new(VecDeque::new()));
                        self.msg_receiver = Some(rx);
                        self.outgoing_queue = Some(Arc::clone(&outgoing));

                        thread::spawn(move || {
                            if let Err(e) = run_tcp_client_threaded(&ip, port, &nick, tx, outgoing) {
                                eprintln!("Connection error: {}", e);
                            }
                        });

                        self.mode = ChatMode::Chat;
                        self.was_logged_out = false;
                        save_state(&self.nick, &self.server_ip, self.local_server_port, false, &self.messages, true);
                        self.online_users.push(self.nick.clone());
                        self.messages
                            .push(("System".to_string(), conn_msg, now_hms()));
                    }
                } else if self.server_started {
                    self.server_started = false;
                    self.server_started = false;
                } else {
                    if self.nick.len() < 2 || self.nick.len() > 24 {
                        self.error_message = "Nick must be 2-24 characters".to_string();
                    } else if !self
                        .nick
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    {
                        self.error_message = "Only letters, numbers, _ and - allowed".to_string();
                    } else if self.server_ip.is_empty() {
                        self.error_message =
                            "Enter server IP (use 127.0.0.1 for local)".to_string();
                    } else {
                        let port = self.local_server_port;
                        if let Err(_e) = thread::spawn(move || {
                            if let Err(e) = run_server_threaded(port) {
                                eprintln!("Server error: {}", e);
                            }
                        })
                        .join()
                        {
                            self.error_message = "Failed to start server".to_string();
                        } else {
                            self.server_started = true;

                            let ip = self.server_ip.clone();
                            let port = self.local_server_port;
                            let nick = self.nick.clone();
                            let conn_msg = format!("Starting local server on port {}...", port);

                            let (tx, rx) = mpsc::channel();
                            let outgoing = Arc::new(Mutex::new(VecDeque::new()));
                            self.msg_receiver = Some(rx);
                            self.outgoing_queue = Some(Arc::clone(&outgoing));

                            thread::spawn(move || {
                                if let Err(e) =
                                    run_tcp_client_threaded(&ip, port, &nick, tx, outgoing)
                                {
                                    eprintln!("Connection error: {}", e);
                                }
                            });

                            self.mode = ChatMode::Chat;
                            self.was_logged_out = false;
                            save_state(
                                &self.nick,
                                &self.server_ip,
                                self.local_server_port,
                                false,
                                &self.messages,
                                true,
                            );
                            self.online_users.push(self.nick.clone());
                            self.messages
                                .push(("System".to_string(), conn_msg, now_hms()));
                        }
                    }
                }
            }
        });
    }

    fn chat_ui(&mut self, ui: &mut egui::Ui, theme: &Theme, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(
                    egui::RichText::new("TailChatter")
                        .size(20.0)
                        .color(theme.title),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(&self.room_name)
                        .size(16.0)
                        .color(theme.muted),
                );
                ui.separator();
                let count = self.online_users.len();
                ui.label(
                    egui::RichText::new(format!("{count} online"))
                        .size(16.0)
                        .color(theme.status),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(&format!("Logged in as: {}", self.nick))
                        .size(16.0)
                        .color(theme.self_name),
                );
                if ui.button("Logout").clicked() {
                    self.logout();
                }
            });
        });

        egui::SidePanel::left("sidebar")
            .default_width(150.0)
            .show(ctx, |ui| {
                ui.heading(egui::RichText::new("Users").size(16.0).color(theme.muted));
                ui.separator();
                for user in &self.online_users {
                    let color = if user == &self.nick {
                        theme.self_name
                    } else {
                        theme.color_for_name(user)
                    };
                    ui.label(egui::RichText::new(user).size(16.0).color(color));
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
                            ui.label(egui::RichText::new(from).size(16.0).color(color).strong());
                            ui.label(egui::RichText::new(body).size(16.0).color(theme.text));
                        });
                        ui.add_space(5.0);
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

fn load_state() -> (
    ChatMode,
    String,
    String,
    u16,
    bool,
    Vec<(String, String, String)>,
    bool,
) {
    let path = state_file_path();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(state) = serde_json::from_str::<PersistedState>(&content) {
                if !state.was_logged_out && !state.nick.is_empty() {
                    return (
                        ChatMode::Chat,
                        state.nick,
                        state.server_ip,
                        state.server_port,
                        false,
                        state.messages,
                        state.server_started,
                    );
                }
                if state.server_started && !state.nick.is_empty() {
                    return (
                        ChatMode::Login,
                        state.nick,
                        state.server_ip,
                        state.server_port,
                        false,
                        Vec::new(),
                        true,
                    );
                }
            }
        }
    }
    (
        ChatMode::Login,
        String::new(),
        String::new(),
        DEFAULT_PORT,
        false,
        Vec::new(),
        false,
    )
}

fn save_state(
    nick: &str,
    server_ip: &str,
    server_port: u16,
    was_logged_out: bool,
    messages: &[(String, String, String)],
    server_started: bool,
) {
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
        server_started,
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

fn run_server_threaded(port: u16) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        if let Err(e) = server::start_server(port).await {
            eprintln!("Failed to start server: {}", e);
        } else {
            println!("Server started on port {}", port);
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
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
