use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use eframe::egui;

use crate::client::state::{AppMode, ChatApp, Conversation, LoginTab};
use crate::client::ui::theme::Theme;

impl ChatApp {
    pub fn login_ui(&mut self, ui: &mut egui::Ui, theme: &Theme) {
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

        // Tab buttons
        egui::Frame::default()
            .fill(egui::Color32::TRANSPARENT)
            .show(ui, |ui| {
                let total_width = 410.0;
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
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Connect To Existing Server").size(16.0),
                                    )
                                    .min_size(egui::vec2(200.0, 40.0)),
                                )
                                .clicked()
                            {
                                self.login_tab = LoginTab::Connect;
                            }
                            ui.add_space(10.0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Create A Server").size(16.0),
                                    )
                                    .min_size(egui::vec2(200.0, 40.0)),
                                )
                                .clicked()
                            {
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
                LoginTab::Connect => self.connect_tab(ui, theme),
                LoginTab::Server => self.server_tab(ui, theme),
            });

        ui.add_space(30.0);

        if !self.error_message.is_empty() {
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(&self.error_message).color(theme.error));
            });
        }
    }

    fn connect_tab(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Connect To An Existing Server")
                    .size(22.0)
                    .color(theme.title),
            );
            ui.add_space(15.0);

            self.nick_field(ui, theme);
            ui.add_space(15.0);
            self.ip_field(ui, theme, true);
            ui.add_space(15.0);
            self.port_field(ui, theme, &mut self.server_port.clone(), true);
        });

        ui.add_space(30.0);

        ui.vertical_centered(|ui| {
            let button = ui.add(
                egui::Button::new(egui::RichText::new("Join Chat").size(20.0))
                    .min_size(egui::vec2(200.0, 45.0)),
            );
            let enter =
                ui.input(|i| i.key_pressed(egui::Key::Enter)) && self.mode == AppMode::Login;

            if button.clicked() || enter {
                self.attempt_connect();
            }
        });
    }

    fn server_tab(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        let server_running = self.server_started && self.mode == AppMode::Login;

        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Start Your Own Server")
                    .size(22.0)
                    .color(theme.title),
            );
            ui.add_space(15.0);

            self.nick_field(ui, theme);
            ui.add_space(15.0);
            self.ip_field(ui, theme, !server_running);
            ui.add_space(15.0);

            let mut port = self.local_server_port;
            self.port_field(ui, theme, &mut port, !server_running);
            self.local_server_port = port;

            if self.server_started {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Server is running").color(theme.status));
            }
        });

        ui.add_space(30.0);

        ui.vertical_centered(|ui| {
            let btn_text = if server_running {
                "Rejoin Chat"
            } else if self.server_started {
                "Stop Server"
            } else {
                "Start Server & Join Chat"
            };

            if ui
                .add(
                    egui::Button::new(egui::RichText::new(btn_text).size(20.0))
                        .min_size(egui::vec2(200.0, 45.0)),
                )
                .clicked()
            {
                self.error_message.clear();

                if server_running {
                    self.attempt_rejoin_local();
                } else if self.server_started {
                    self.server_started = false;
                } else {
                    self.attempt_start_server();
                }
            }
        });
    }

    fn nick_field(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        ui.label(egui::RichText::new("Your Handle:").color(theme.muted).size(16.0));
        ui.add(
            egui::TextEdit::singleline(&mut self.nick)
                .desired_width(280.0)
                .text_color(theme.text)
                .margin(egui::vec2(12.0, 10.0)),
        );
    }

    fn ip_field(&mut self, ui: &mut egui::Ui, theme: &Theme, editable: bool) {
        ui.label(egui::RichText::new("Server IP:").color(theme.muted).size(16.0));
        ui.add(
            egui::TextEdit::singleline(&mut self.server_ip)
                .desired_width(280.0)
                .text_color(if editable { theme.text } else { theme.muted })
                .margin(egui::vec2(12.0, 10.0)),
        );
    }

    fn port_field(&mut self, ui: &mut egui::Ui, theme: &Theme, port: &mut u16, editable: bool) {
        ui.label(egui::RichText::new("Port:").color(theme.muted).size(16.0));
        let mut port_string = port.to_string();
        ui.add(
            egui::TextEdit::singleline(&mut port_string)
                .desired_width(280.0)
                .text_color(if editable { theme.text } else { theme.muted })
                .margin(egui::vec2(12.0, 10.0)),
        );
        if let Ok(p) = port_string.parse() {
            *port = p;
        }
    }

    fn validate_nick(&mut self) -> bool {
        if self.nick.len() < 2 || self.nick.len() > 24 {
            self.error_message = "Nick must be 2-24 characters".into();
            return false;
        }
        if !self
            .nick
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            self.error_message = "Only letters, numbers, _ and - allowed".into();
            return false;
        }
        true
    }

    fn attempt_connect(&mut self) {
        self.error_message.clear();
        if !self.validate_nick() {
            return;
        }
        if self.server_ip.is_empty() {
            self.error_message = "Enter server IP".into();
            return;
        }

        self.start_client_connection(self.server_ip.clone(), self.server_port);
    }

    fn attempt_rejoin_local(&mut self) {
        if !self.validate_nick() {
            return;
        }
        self.start_client_connection(self.server_ip.clone(), self.local_server_port);
    }

    fn attempt_start_server(&mut self) {
        if !self.validate_nick() {
            return;
        }
        if self.server_ip.is_empty() {
            self.error_message = "Enter server IP (use 127.0.0.1 for local)".into();
            return;
        }

        let port = self.local_server_port;
        let _ = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                if let Err(e) = crate::server::start(port) {
                    eprintln!("Server error: {e}");
                } else {
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
                    }
                }
            });
        });
        thread::sleep(std::time::Duration::from_millis(100));
        self.server_started = true;

        self.start_client_connection(self.server_ip.clone(), self.local_server_port);
    }

    fn start_client_connection(&mut self, ip: String, port: u16) {
        let nick = self.nick.clone();

        let (tx, rx) = mpsc::channel();
        let outgoing = Arc::new(Mutex::new(VecDeque::new()));
        self.msg_receiver = Some(rx);
        self.outgoing_queue = Some(Arc::clone(&outgoing));

        thread::spawn(move || {
            if let Err(e) = crate::client::connect_threaded(&ip, port, &nick, tx, outgoing) {
                eprintln!("Connection error: {e}");
            }
        });

        self.mode = AppMode::Chat;
        self.was_logged_out = false;
        self.active_conversation = Conversation::Group;
        self.dm_conversations.clear();
        self.conversation_messages.clear();
        self.unread_dms.clear();
        self.online_users.push(self.nick.clone());
    }
}
