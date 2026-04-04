use std::sync::mpsc;

use egui::{Color32, RichText, Ui};
use scalper_core::ScalperConfig;

const ACCENT: Color32 = Color32::from_rgb(0x58, 0xa6, 0xff);
const API_BASE: &str = "http://localhost:3000/api/config";

/// Channel types used for async HTTP results.
pub type ConfigResult = Result<ScalperConfig, String>;

/// Holds in-flight HTTP receivers so the UI can poll them without static mut.
pub struct SettingsIO {
    load_rx: Option<mpsc::Receiver<ConfigResult>>,
    save_rx: Option<mpsc::Receiver<Result<(), String>>>,
}

impl SettingsIO {
    pub fn new() -> Self {
        Self {
            load_rx: None,
            save_rx: None,
        }
    }

    fn spawn_load(&mut self) {
        let (tx, rx) = mpsc::channel::<ConfigResult>();
        self.load_rx = Some(rx);
        std::thread::spawn(move || {
            let result = (|| {
                let body: String = ureq::get(API_BASE)
                    .call()
                    .map_err(|e| e.to_string())?
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| e.to_string())?;
                let cfg: ScalperConfig =
                    serde_json::from_str(&body).map_err(|e| e.to_string())?;
                Ok(cfg)
            })();
            let _ = tx.send(result);
        });
    }

    fn spawn_save(&mut self, cfg: ScalperConfig) {
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        self.save_rx = Some(rx);
        std::thread::spawn(move || {
            let result = (|| {
                let json = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
                ureq::put(API_BASE)
                    .content_type("application/json")
                    .send(json.as_bytes())
                    .map_err(|e| e.to_string())?;
                Ok(())
            })();
            let _ = tx.send(result);
        });
    }

    fn poll_load(
        &mut self,
        config: &mut Option<ScalperConfig>,
        loading: &mut bool,
        toast: &mut Option<(String, f64)>,
    ) {
        if let Some(rx) = &self.load_rx {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(cfg) => {
                        *config = Some(cfg);
                    }
                    Err(e) => {
                        *toast = Some((format!("Load failed: {e}"), current_time() + 5.0));
                    }
                }
                *loading = false;
                self.load_rx = None;
            }
        }
    }

    fn poll_save(&mut self, toast: &mut Option<(String, f64)>) {
        if let Some(rx) = &self.save_rx {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(()) => {
                        *toast =
                            Some(("Config saved successfully".into(), current_time() + 4.0));
                    }
                    Err(e) => {
                        *toast = Some((format!("Save failed: {e}"), current_time() + 5.0));
                    }
                }
                self.save_rx = None;
            }
        }
    }
}

fn current_time() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub fn show(
    ui: &mut Ui,
    config: &mut Option<ScalperConfig>,
    dirty: &mut bool,
    loading: &mut bool,
    toast: &mut Option<(String, f64)>,
    io: &mut SettingsIO,
) {
    // Poll async results
    io.poll_load(config, loading, toast);
    io.poll_save(toast);

    // Auto-load on first render
    if config.is_none() && !*loading {
        *loading = true;
        io.spawn_load();
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Settings").heading().color(ACCENT));
            if ui.button("Reload").clicked() && !*loading {
                *loading = true;
                io.spawn_load();
            }
            if *loading {
                ui.spinner();
            }
        });

        ui.label(
            RichText::new("Warning: Restart bot for changes to take full effect")
                .color(Color32::from_rgb(0xe3, 0xb3, 0x41))
                .italics(),
        );

        ui.add_space(8.0);

        let Some(cfg) = config.as_mut() else {
            ui.label("Loading configuration...");
            return;
        };

        // ── General ──
        egui::CollapsingHeader::new(RichText::new("General").strong())
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Mode:");
                    if ui.text_edit_singleline(&mut cfg.general.mode).changed() {
                        *dirty = true;
                    }
                });
            });

        // ── Exchanges ──
        egui::CollapsingHeader::new(RichText::new("Exchanges").strong())
            .default_open(false)
            .show(ui, |ui| {
                // Binance
                egui::CollapsingHeader::new("Binance")
                    .id_salt("binance")
                    .show(ui, |ui| {
                        let ex = cfg.exchanges.binance.get_or_insert_with(|| {
                            scalper_core::ExchangeConfig {
                                api_key: String::new(),
                                api_secret: String::new(),
                                base_url_rest: String::new(),
                                base_url_ws: String::new(),
                                testnet: false,
                                symbol_map: Default::default(),
                            }
                        });
                        *dirty |= exchange_fields(ui, ex);
                    });

                // Bybit
                egui::CollapsingHeader::new("Bybit")
                    .id_salt("bybit")
                    .show(ui, |ui| {
                        let ex = cfg.exchanges.bybit.get_or_insert_with(|| {
                            scalper_core::ExchangeConfig {
                                api_key: String::new(),
                                api_secret: String::new(),
                                base_url_rest: String::new(),
                                base_url_ws: String::new(),
                                testnet: false,
                                symbol_map: Default::default(),
                            }
                        });
                        *dirty |= exchange_fields(ui, ex);
                    });

                // OKX
                egui::CollapsingHeader::new("OKX")
                    .id_salt("okx")
                    .show(ui, |ui| {
                        let ex = cfg.exchanges.okx.get_or_insert_with(|| {
                            scalper_core::OkxExchangeConfig {
                                api_key: String::new(),
                                api_secret: String::new(),
                                passphrase: String::new(),
                                base_url_rest: String::new(),
                                base_url_ws: String::new(),
                                testnet: false,
                            }
                        });
                        let mut changed = false;
                        ui.horizontal(|ui| {
                            ui.label("API Key:");
                            changed |= ui.text_edit_singleline(&mut ex.api_key).changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("API Secret:");
                            changed |= ui.text_edit_singleline(&mut ex.api_secret).changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("Passphrase:");
                            changed |= ui.text_edit_singleline(&mut ex.passphrase).changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("REST URL:");
                            changed |= ui.text_edit_singleline(&mut ex.base_url_rest).changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("WS URL:");
                            changed |= ui.text_edit_singleline(&mut ex.base_url_ws).changed();
                        });
                        changed |= ui.checkbox(&mut ex.testnet, "Testnet").changed();
                        *dirty |= changed;
                    });

                // Kraken
                egui::CollapsingHeader::new("Kraken")
                    .id_salt("kraken")
                    .show(ui, |ui| {
                        let ex = cfg.exchanges.kraken.get_or_insert_with(|| {
                            scalper_core::ExchangeConfig {
                                api_key: String::new(),
                                api_secret: String::new(),
                                base_url_rest: String::new(),
                                base_url_ws: String::new(),
                                testnet: false,
                                symbol_map: Default::default(),
                            }
                        });
                        *dirty |= exchange_fields(ui, ex);
                    });
            });

        // ── Trading ──
        egui::CollapsingHeader::new(RichText::new("Trading").strong())
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Symbols (comma-separated):");
                    let mut symbols_str = cfg.trading.symbols.join(", ");
                    if ui.text_edit_singleline(&mut symbols_str).changed() {
                        cfg.trading.symbols = symbols_str
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        *dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Default leverage:");
                    let mut v = cfg.trading.default_leverage as i32;
                    if ui.add(egui::Slider::new(&mut v, 1..=50)).changed() {
                        cfg.trading.default_leverage = v as u32;
                        *dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Max leverage:");
                    let mut v = cfg.trading.max_leverage as i32;
                    if ui.add(egui::Slider::new(&mut v, 1..=50)).changed() {
                        cfg.trading.max_leverage = v as u32;
                        *dirty = true;
                    }
                });
            });

        // ── Risk ──
        egui::CollapsingHeader::new(RichText::new("Risk").strong())
            .default_open(false)
            .show(ui, |ui| {
                let r = &mut cfg.risk;
                *dirty |= drag_f64(ui, "Max risk/trade %:", &mut r.max_risk_per_trade_pct, 0.01, 10.0);
                *dirty |= drag_f64(ui, "Max daily loss %:", &mut r.max_daily_loss_pct, 0.1, 50.0);
                *dirty |= drag_f64(ui, "Max drawdown %:", &mut r.max_drawdown_pct, 0.1, 50.0);
                *dirty |= drag_u32(ui, "Max consecutive losses:", &mut r.max_consecutive_losses, 1, 50);
                *dirty |= drag_u32(ui, "Cooldown minutes:", &mut r.cooldown_minutes, 1, 120);
                *dirty |= drag_f64(ui, "Min equity:", &mut r.min_equity, 0.0, 1_000_000.0);
                *dirty |= drag_u32(ui, "Max open positions:", &mut r.max_open_positions, 1, 20);
                *dirty |= drag_u32(ui, "Max trades/hour:", &mut r.max_trades_per_hour, 1, 200);
                *dirty |= drag_u32(ui, "Max leverage:", &mut r.max_leverage, 1, 50);
            });

        // ── Strategy ──
        egui::CollapsingHeader::new(RichText::new("Strategy").strong())
            .default_open(false)
            .show(ui, |ui| {
                *dirty |= drag_f64(
                    ui,
                    "Ensemble threshold:",
                    &mut cfg.strategy.ensemble_threshold,
                    0.0,
                    1.0,
                );

                egui::CollapsingHeader::new("Momentum")
                    .id_salt("strat_mom")
                    .show(ui, |ui| {
                        let m = &mut cfg.strategy.momentum;
                        *dirty |= ui.checkbox(&mut m.enabled, "Enabled").changed();
                        *dirty |= drag_f64(ui, "Weight:", &mut m.weight, 0.0, 1.0);
                        *dirty |= drag_f64(ui, "Volume spike mult:", &mut m.volume_spike_multiplier, 1.0, 10.0);
                        *dirty |= drag_f64(ui, "Take profit %:", &mut m.take_profit_pct, 0.01, 5.0);
                        *dirty |= drag_f64(ui, "Stop loss %:", &mut m.stop_loss_pct, 0.01, 5.0);
                        *dirty |= drag_f64(ui, "Trailing stop %:", &mut m.trailing_stop_pct, 0.01, 5.0);
                        *dirty |= drag_f64(ui, "RSI overbought:", &mut m.rsi_overbought, 50.0, 100.0);
                        *dirty |= drag_f64(ui, "RSI oversold:", &mut m.rsi_oversold, 0.0, 50.0);
                    });

                egui::CollapsingHeader::new("Order Book Imbalance")
                    .id_salt("strat_ob")
                    .show(ui, |ui| {
                        let o = &mut cfg.strategy.ob_imbalance;
                        *dirty |= ui.checkbox(&mut o.enabled, "Enabled").changed();
                        *dirty |= drag_f64(ui, "Weight:", &mut o.weight, 0.0, 1.0);
                        *dirty |= drag_f64(ui, "Imbalance threshold:", &mut o.imbalance_threshold, 0.0, 1.0);
                        *dirty |= drag_u32(ui, "TP ticks:", &mut o.take_profit_ticks, 1, 20);
                        *dirty |= drag_u32(ui, "SL ticks:", &mut o.stop_loss_ticks, 1, 20);
                    });

                egui::CollapsingHeader::new("Liquidation Wick")
                    .id_salt("strat_liq")
                    .show(ui, |ui| {
                        let l = &mut cfg.strategy.liquidation_wick;
                        *dirty |= ui.checkbox(&mut l.enabled, "Enabled").changed();
                        *dirty |= drag_f64(ui, "Weight:", &mut l.weight, 0.0, 1.0);
                        *dirty |= drag_f64(ui, "Price velocity threshold:", &mut l.price_velocity_threshold, 0.0, 10.0);
                        *dirty |= drag_f64(ui, "Volume spike mult:", &mut l.volume_spike_multiplier, 1.0, 10.0);
                        *dirty |= drag_f64(ui, "Take profit %:", &mut l.take_profit_pct, 0.01, 5.0);
                        *dirty |= drag_f64(ui, "Stop loss %:", &mut l.stop_loss_pct, 0.01, 5.0);
                    });

                egui::CollapsingHeader::new("Funding Bias")
                    .id_salt("strat_fund")
                    .show(ui, |ui| {
                        let f = &mut cfg.strategy.funding_bias;
                        *dirty |= ui.checkbox(&mut f.enabled, "Enabled").changed();
                        *dirty |= drag_f64(ui, "Weight:", &mut f.weight, 0.0, 1.0);
                        *dirty |= drag_f64(ui, "Funding threshold:", &mut f.funding_threshold, 0.0, 1.0);
                        *dirty |= drag_f64(ui, "Strength boost:", &mut f.strength_boost, 0.0, 1.0);
                    });
            });

        ui.add_space(12.0);

        // ── Save Button ──
        ui.horizontal(|ui| {
            let save_enabled = *dirty && config.is_some();
            if ui
                .add_enabled(save_enabled, egui::Button::new("Save Config"))
                .clicked()
            {
                if let Some(cfg) = config.as_ref() {
                    io.spawn_save(cfg.clone());
                    *dirty = false;
                }
            }
            if *dirty {
                ui.label(RichText::new("(unsaved changes)").color(Color32::from_rgb(0xe3, 0xb3, 0x41)));
            }
        });
    });
}

/// Helper: editable fields for a standard ExchangeConfig.
fn exchange_fields(ui: &mut Ui, ex: &mut scalper_core::ExchangeConfig) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("API Key:");
        changed |= ui.text_edit_singleline(&mut ex.api_key).changed();
    });
    ui.horizontal(|ui| {
        ui.label("API Secret:");
        changed |= ui.text_edit_singleline(&mut ex.api_secret).changed();
    });
    ui.horizontal(|ui| {
        ui.label("REST URL:");
        changed |= ui.text_edit_singleline(&mut ex.base_url_rest).changed();
    });
    ui.horizontal(|ui| {
        ui.label("WS URL:");
        changed |= ui.text_edit_singleline(&mut ex.base_url_ws).changed();
    });
    changed |= ui.checkbox(&mut ex.testnet, "Testnet").changed();
    changed
}

fn drag_f64(ui: &mut Ui, label: &str, val: &mut f64, min: f64, max: f64) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        changed = ui
            .add(egui::DragValue::new(val).range(min..=max).speed(0.01))
            .changed();
    });
    changed
}

fn drag_u32(ui: &mut Ui, label: &str, val: &mut u32, min: u32, max: u32) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let mut v = *val as i32;
        if ui
            .add(egui::DragValue::new(&mut v).range(min as i32..=max as i32))
            .changed()
        {
            *val = v as u32;
            changed = true;
        }
    });
    changed
}
