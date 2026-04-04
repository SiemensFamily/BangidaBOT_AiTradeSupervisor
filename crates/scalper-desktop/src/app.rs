use eframe::egui;
use scalper_core::ScalperConfig;

use crate::tabs;
use crate::tabs::settings::SettingsIO;
use crate::ws_client::{Snapshot, WsClient};

const MAX_EQUITY_HISTORY: usize = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Settings,
    TradeLogic,
}

pub struct ScalperApp {
    current_tab: Tab,
    ws: WsClient,
    snapshot: Snapshot,
    equity_history: Vec<f64>,
    config_cache: Option<ScalperConfig>,
    config_dirty: bool,
    toast: Option<(String, f64)>,
    settings_loading: bool,
    settings_io: SettingsIO,
    first_frame: bool,
}

impl ScalperApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            current_tab: Tab::Dashboard,
            ws: WsClient::new(),
            snapshot: Snapshot::default(),
            equity_history: Vec::with_capacity(MAX_EQUITY_HISTORY),
            config_cache: None,
            config_dirty: false,
            toast: None,
            settings_loading: false,
            settings_io: SettingsIO::new(),
            first_frame: true,
        }
    }
}

impl eframe::App for ScalperApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Connect on first frame
        if self.first_frame {
            self.ws.connect(ctx);
            self.first_frame = false;
        }

        // Poll WebSocket
        if let Some(snap) = self.ws.poll() {
            // Push equity for chart
            self.equity_history.push(snap.equity);
            if self.equity_history.len() > MAX_EQUITY_HISTORY {
                self.equity_history.remove(0);
            }
            self.snapshot = snap;
        }

        // Expire toast
        if let Some((_msg, expiry)) = &self.toast {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            if now > *expiry {
                self.toast = None;
            }
        }

        // Top panel with tabs
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.current_tab, Tab::Dashboard, "Dashboard");
                ui.selectable_value(&mut self.current_tab, Tab::Settings, "Settings");
                ui.selectable_value(&mut self.current_tab, Tab::TradeLogic, "Trade Logic");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let status = if self.ws.is_connected() {
                        egui::RichText::new("WS Connected")
                            .color(egui::Color32::from_rgb(0x2e, 0xa0, 0x43))
                    } else {
                        egui::RichText::new("WS Disconnected")
                            .color(egui::Color32::from_rgb(0xf8, 0x51, 0x49))
                    };
                    ui.label(status);
                });
            });

            // Toast message
            if let Some((msg, _)) = &self.toast {
                ui.label(
                    egui::RichText::new(msg).color(egui::Color32::from_rgb(0x58, 0xa6, 0xff)),
                );
            }
        });

        // Central panel delegates to active tab
        egui::CentralPanel::default().show(ctx, |ui| match self.current_tab {
            Tab::Dashboard => {
                tabs::dashboard::show(ui, &self.snapshot, &self.equity_history);
            }
            Tab::Settings => {
                tabs::settings::show(
                    ui,
                    &mut self.config_cache,
                    &mut self.config_dirty,
                    &mut self.settings_loading,
                    &mut self.toast,
                    &mut self.settings_io,
                );
            }
            Tab::TradeLogic => {
                tabs::trade_logic::show(ui);
            }
        });
    }
}
