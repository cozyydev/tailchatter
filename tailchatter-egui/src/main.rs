#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod client;
mod protocol;
mod server;

use eframe::egui;

use client::state::{AppMode, ChatApp};
use client::ui::theme::Theme;
use protocol::ServerMsg;

impl eframe::App for ChatApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let theme = Theme::default();
        ctx.style_mut(|style| style.visuals = theme.visuals());
        ctx.request_repaint_after(std::time::Duration::from_millis(100));

        // Poll incoming messages
        let mut received = Vec::new();
        if let Some(ref rx) = self.msg_receiver {
            while let Ok(msg) = rx.try_recv() {
                received.push(msg);
            }
        }

        for msg in received {
            match serde_json::from_str::<ServerMsg>(&msg) {
                Ok(server_msg) => self.handle_server_msg(server_msg),
                Err(_) => self.handle_plain_text(&msg),
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| match self.mode {
            AppMode::Login => self.login_ui(ui, &theme),
            AppMode::Chat => self.chat_ui(ui, &theme, ctx),
        });
    }
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
