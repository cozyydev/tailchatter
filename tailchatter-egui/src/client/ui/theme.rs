use eframe::egui;

pub struct Theme {
    pub bg: egui::Color32,
    pub title: egui::Color32,
    pub text: egui::Color32,
    pub muted: egui::Color32,
    pub status: egui::Color32,
    pub error: egui::Color32,
    pub self_name: egui::Color32,
    pub system: egui::Color32,
    pub name_palette: [egui::Color32; 6],
}

impl Default for Theme {
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
}

impl Theme {
    pub fn visuals(&self) -> egui::Visuals {
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

    pub fn color_for_name(&self, name: &str) -> egui::Color32 {
        let hash: u64 = name
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_add(b as u64).wrapping_mul(31));
        self.name_palette[(hash as usize) % self.name_palette.len()]
    }
}
