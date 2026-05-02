use eframe::egui;

use crate::client::state::{ChatApp, Conversation};
use crate::client::ui::theme::Theme;

impl ChatApp {
    pub fn chat_ui(&mut self, ui: &mut egui::Ui, theme: &Theme, ctx: &egui::Context) {
        self.chat_header(theme, ctx);
        self.chat_sidebar(theme, ctx);
        self.chat_messages(theme, ctx);
        self.chat_input(ui, ctx);
    }

    fn chat_header(&mut self, theme: &Theme, ctx: &egui::Context) {
        let mut logout_clicked = false;
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(
                    egui::RichText::new("TailChatter")
                        .size(20.0)
                        .color(theme.title),
                );
                ui.separator();

                let conv_name = match &self.active_conversation {
                    Conversation::Group => self.room_name.clone(),
                    Conversation::Dm(partner) => format!("DM with {partner}"),
                };
                ui.label(
                    egui::RichText::new(&conv_name)
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
                    egui::RichText::new(format!("Logged in as: {}", self.nick))
                        .size(16.0)
                        .color(theme.self_name),
                );
                ui.separator();

                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Logout").size(16.0).color(theme.error),
                        )
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::new(1.5, theme.error))
                        .rounding(egui::Rounding::same(4.0)),
                    )
                    .clicked()
                {
                    logout_clicked = true;
                }
            });
        });
        if logout_clicked {
            self.logout();
        }
    }

    fn chat_sidebar(&mut self, theme: &Theme, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .default_width(180.0)
            .show(ctx, |ui| {
                // Online users
                ui.heading(egui::RichText::new("Users").size(16.0).color(theme.muted));
                ui.separator();

                let users = self.online_users.clone();
                for user in &users {
                    let color = if user == &self.nick {
                        theme.self_name
                    } else {
                        theme.color_for_name(user)
                    };

                    if ui
                        .selectable_label(false, egui::RichText::new(user).size(16.0).color(color))
                        .clicked()
                        && user != &self.nick
                    {
                        if !self.dm_conversations.contains(user) {
                            self.dm_conversations.push(user.clone());
                        }
                        self.active_conversation = Conversation::Dm(user.clone());
                        self.unread_dms.remove(user);
                    }
                }

                ui.add_space(10.0);
                ui.separator();

                // Conversations list
                ui.heading(
                    egui::RichText::new("Conversations")
                        .size(16.0)
                        .color(theme.muted),
                );
                ui.separator();

                if ui
                    .selectable_label(
                        self.active_conversation == Conversation::Group,
                        egui::RichText::new("Group Chat")
                            .size(16.0)
                            .color(theme.text),
                    )
                    .clicked()
                {
                    self.active_conversation = Conversation::Group;
                }

                let dm_convos = self.dm_conversations.clone();
                for user in &dm_convos {
                    let label = if let Some(&count) = self.unread_dms.get(user) {
                        format!("{user} ({count})")
                    } else {
                        user.clone()
                    };
                    let color = if self.unread_dms.contains_key(user) {
                        theme.error
                    } else {
                        theme.text
                    };

                    if ui
                        .selectable_label(
                            self.active_conversation == Conversation::Dm(user.clone()),
                            egui::RichText::new(label).size(16.0).color(color),
                        )
                        .clicked()
                    {
                        self.active_conversation = Conversation::Dm(user.clone());
                        self.unread_dms.remove(user);
                    }
                }
            });
    }

    fn chat_messages(&self, theme: &Theme, ctx: &egui::Context) {
        let messages = self
            .conversation_messages
            .get(&self.active_conversation)
            .cloned()
            .unwrap_or_default();

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (from, body, _) in &messages {
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
    }

    fn chat_input(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let input_id = egui::Id::new("chat_input");
        let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

        egui::TopBottomPanel::bottom("input").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let width = ui.available_width() - 80.0;
                ui.add(
                    egui::TextEdit::singleline(&mut self.input_message)
                        .id(input_id)
                        .desired_width(width)
                        .margin(egui::vec2(12.0, 10.0)),
                );
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("Send").size(16.0))
                            .min_size(egui::vec2(50.0, 30.0)), // width 70, height 36
                    )
                    .clicked()
                {
                    self.send_message();
                }
            });
            ui.add_space(8.0);
        });

        if enter_pressed && !self.input_message.is_empty() {
            self.send_message();
            ctx.memory_mut(|mem| mem.request_focus(input_id));
        }
    }
}
