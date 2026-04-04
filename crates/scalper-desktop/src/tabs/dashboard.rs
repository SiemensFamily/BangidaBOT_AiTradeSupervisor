use egui::{Color32, RichText, Ui};
use egui_plot::{Line, Plot, PlotPoints};

use crate::ws_client::Snapshot;

const ACCENT: Color32 = Color32::from_rgb(0x58, 0xa6, 0xff);
const GREEN: Color32 = Color32::from_rgb(0x2e, 0xa0, 0x43);
const RED: Color32 = Color32::from_rgb(0xf8, 0x51, 0x49);
const YELLOW: Color32 = Color32::from_rgb(0xe3, 0xb3, 0x41);

fn pnl_color(value: f64) -> Color32 {
    if value >= 0.0 { GREEN } else { RED }
}

pub fn show(ui: &mut Ui, snap: &Snapshot, equity_history: &[f64]) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;

        // ── Warmup Banner ──
        if !snap.warmup_ready {
            ui.group(|ui| {
                ui.label(RichText::new("System Warming Up...").color(YELLOW).heading());
                ui.horizontal(|ui| {
                    ui.label("Indicators:");
                    let ind_frac = if snap.indicators_total > 0 {
                        snap.indicators_ready as f32 / snap.indicators_total as f32
                    } else {
                        0.0
                    };
                    ui.add(
                        egui::ProgressBar::new(ind_frac)
                            .text(format!("{}/{}", snap.indicators_ready, snap.indicators_total)),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Regime ATR:");
                    let regime_frac = if snap.regime_atr_needed > 0 {
                        snap.regime_atr_count as f32 / snap.regime_atr_needed as f32
                    } else {
                        0.0
                    };
                    ui.add(
                        egui::ProgressBar::new(regime_frac)
                            .text(format!("{}/{}", snap.regime_atr_count, snap.regime_atr_needed)),
                    );
                });
                let remaining_indicators =
                    snap.indicators_total.saturating_sub(snap.indicators_ready);
                let remaining_regime =
                    snap.regime_atr_needed.saturating_sub(snap.regime_atr_count);
                let est_secs = remaining_indicators.max(remaining_regime as u32) as u64;
                if est_secs > 0 {
                    ui.label(format!("Estimated time: ~{}s", est_secs));
                }
            });
        } else {
            ui.label(RichText::new("System Ready").color(GREEN).heading());
        }

        ui.add_space(4.0);

        // ── Stats Grid ──
        ui.label(RichText::new("Performance").strong().color(ACCENT));
        egui::Grid::new("stats_grid")
            .num_columns(4)
            .striped(true)
            .spacing([20.0, 4.0])
            .show(ui, |ui| {
                ui.label("Equity:");
                ui.label(RichText::new(format!("${:.2}", snap.equity)).strong());
                ui.label("Starting:");
                ui.label(format!("${:.2}", snap.starting_equity));
                ui.end_row();

                ui.label("Daily PnL:");
                ui.label(
                    RichText::new(format!("${:.2}", snap.daily_pnl)).color(pnl_color(snap.daily_pnl)),
                );
                ui.label("Total PnL:");
                ui.label(
                    RichText::new(format!("${:.2}", snap.total_pnl)).color(pnl_color(snap.total_pnl)),
                );
                ui.end_row();

                ui.label("Drawdown:");
                ui.label(
                    RichText::new(format!("{:.2}%", snap.drawdown_pct)).color(RED),
                );
                ui.label("Fees:");
                ui.label(format!("${:.2}", snap.total_fees));
                ui.end_row();

                ui.label("Win Rate:");
                ui.label(format!("{:.1}%", snap.win_rate * 100.0));
                ui.label("Profit Factor:");
                ui.label(format!("{:.2}", snap.profit_factor));
                ui.end_row();

                ui.label("Expectancy:");
                ui.label(format!("${:.4}", snap.expectancy));
                ui.label("Total Trades:");
                ui.label(format!("{}", snap.total_trades));
                ui.end_row();
            });

        ui.add_space(8.0);

        // ── Risk Section ──
        ui.label(RichText::new("Risk Status").strong().color(ACCENT));
        ui.horizontal(|ui| {
            let (badge_text, badge_color) = if snap.can_trade {
                ("CAN TRADE", GREEN)
            } else {
                ("HALTED", RED)
            };
            ui.label(RichText::new(badge_text).strong().color(badge_color));
            ui.separator();
            ui.label(format!("Consecutive losses: {}", snap.consecutive_losses));
            ui.separator();
            ui.label(format!("Trades/hour: {}", snap.trades_this_hour));
            ui.separator();
            ui.label(
                RichText::new(format!("Daily loss: ${:.2}", snap.daily_loss))
                    .color(pnl_color(-snap.daily_loss.abs())),
            );
            ui.separator();
            ui.label(format!("Regime: {}", snap.regime));
            ui.separator();
            ui.label(format!("Mode: {}", snap.mode));
            ui.separator();
            ui.label(format!("Uptime: {}s", snap.uptime_secs));
        });

        ui.add_space(8.0);

        // ── Equity Chart ──
        ui.label(RichText::new("Equity Chart").strong().color(ACCENT));
        let points: PlotPoints = PlotPoints::from_iter(
            equity_history
                .iter()
                .enumerate()
                .map(|(i, &v)| [i as f64, v]),
        );
        let line = Line::new(points).color(ACCENT).name("Equity");
        Plot::new("equity_plot")
            .height(200.0)
            .allow_drag(false)
            .allow_zoom(false)
            .show(ui, |plot_ui| {
                plot_ui.line(line);
            });

        ui.add_space(8.0);

        // ── Open Orders ──
        ui.label(RichText::new("Open Orders").strong().color(ACCENT));
        if snap.open_orders.is_empty() {
            ui.label("No open orders");
        } else {
            egui::Grid::new("orders_grid")
                .striped(true)
                .min_col_width(80.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("Order ID").strong());
                    ui.label(RichText::new("Symbol").strong());
                    ui.label(RichText::new("Side").strong());
                    ui.label(RichText::new("Price").strong());
                    ui.label(RichText::new("Qty").strong());
                    ui.label(RichText::new("Filled").strong());
                    ui.label(RichText::new("Status").strong());
                    ui.end_row();

                    for order in &snap.open_orders {
                        ui.label(&order.order_id);
                        ui.label(&order.symbol);
                        let side_color = if order.side == "Buy" { GREEN } else { RED };
                        ui.label(RichText::new(&order.side).color(side_color));
                        ui.label(&order.price);
                        ui.label(&order.quantity);
                        ui.label(&order.filled_qty);
                        ui.label(&order.status);
                        ui.end_row();
                    }
                });
        }

        ui.add_space(8.0);

        // ── Markets ──
        ui.label(RichText::new("Markets").strong().color(ACCENT));
        if snap.markets.is_empty() {
            ui.label("No market data");
        } else {
            egui::Grid::new("markets_grid")
                .striped(true)
                .min_col_width(100.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("Symbol").strong());
                    ui.label(RichText::new("Best Bid").strong());
                    ui.label(RichText::new("Best Ask").strong());
                    ui.label(RichText::new("Spread").strong());
                    ui.end_row();

                    for m in &snap.markets {
                        ui.label(&m.symbol);
                        ui.label(&m.best_bid);
                        ui.label(&m.best_ask);
                        ui.label(&m.spread);
                        ui.end_row();
                    }
                });
        }

        ui.add_space(8.0);

        // ── CSV Export ──
        if ui.button("Export Trades CSV").clicked() {
            let url = "http://localhost:3000/api/trades.csv";
            if open::that(url).is_err() {
                ui.label(format!("Open in browser: {url}"));
            }
        }
    });
}
