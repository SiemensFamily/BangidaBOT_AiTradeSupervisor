use egui::{Color32, RichText, Ui};

const ACCENT: Color32 = Color32::from_rgb(0x58, 0xa6, 0xff);
const GREEN: Color32 = Color32::from_rgb(0x2e, 0xa0, 0x43);

pub fn show(ui: &mut Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;

        // ── Ensemble Voting System ──
        ui.label(RichText::new("Ensemble Voting System").heading().color(ACCENT));
        ui.label("Minimum weighted strength >= 0.20");
        ui.label("Minimum 2 strategies must agree on direction");
        ui.add_space(8.0);

        // ── Regime-Adaptive Weights ──
        ui.label(RichText::new("Regime-Adaptive Weights").heading().color(ACCENT));
        egui::Grid::new("regime_weights")
            .striped(true)
            .min_col_width(100.0)
            .show(ui, |ui| {
                ui.label(RichText::new("Regime").strong());
                ui.label(RichText::new("Momentum").strong());
                ui.label(RichText::new("OB Imbalance").strong());
                ui.label(RichText::new("Liq Wick").strong());
                ui.label(RichText::new("Funding").strong());
                ui.end_row();

                ui.label("Volatile");
                ui.label("50%");
                ui.label("10%");
                ui.label("30%");
                ui.label("10%");
                ui.end_row();

                ui.label("Normal");
                ui.label("40%");
                ui.label("25%");
                ui.label("20%");
                ui.label("15%");
                ui.end_row();

                ui.label("Ranging");
                ui.label("20%");
                ui.label("40%");
                ui.label("25%");
                ui.label("15%");
                ui.end_row();

                ui.label(RichText::new("Extreme").color(Color32::from_rgb(0xf8, 0x51, 0x49)));
                for _ in 0..4 {
                    ui.label(
                        RichText::new("PAUSED")
                            .color(Color32::from_rgb(0xf8, 0x51, 0x49)),
                    );
                }
                ui.end_row();
            });
        ui.add_space(12.0);

        // ── Strategy 1: Momentum Breakout ──
        ui.label(
            RichText::new("Strategy 1: Momentum Breakout (40% weight)")
                .heading()
                .color(ACCENT),
        );
        ui.label(RichText::new("LONG conditions:").strong().color(GREEN));
        ui.label(
            "  price > 60s high AND volume >= 2.5x avg AND RSI < 80 AND OBV >= 0 \
             AND EMA9 > EMA21 AND 5m/15m not Down",
        );
        ui.label(RichText::new("SHORT conditions:").strong().color(Color32::from_rgb(0xf8, 0x51, 0x49)));
        ui.label(
            "  price < 60s low AND volume >= 2.5x AND RSI > 20 AND OBV <= 0 \
             AND EMA9 < EMA21",
        );
        ui.label("TP: 0.50%, SL: 0.25% (2:1 R:R), trailing stop: 0.20%");
        ui.add_space(12.0);

        // ── Strategy 2: Order Book Imbalance ──
        ui.label(
            RichText::new("Strategy 2: Order Book Imbalance (25% weight)")
                .heading()
                .color(ACCENT),
        );
        ui.label(RichText::new("LONG conditions:").strong().color(GREEN));
        ui.label("  imbalance > 0.30 AND CVD > 0 AND spread <= 2x tick");
        ui.label(RichText::new("SHORT conditions:").strong().color(Color32::from_rgb(0xf8, 0x51, 0x49)));
        ui.label("  imbalance < -0.30 AND CVD < 0");
        ui.label("PostOnly orders only (maker rebates)");
        ui.label("TP: 3 ticks, SL: 2 ticks");
        ui.add_space(12.0);

        // ── Strategy 3: Liquidation Wick Reversal ──
        ui.label(
            RichText::new("Strategy 3: Liquidation Wick Reversal (20% weight)")
                .heading()
                .color(ACCENT),
        );
        ui.label(RichText::new("LONG conditions:").strong().color(GREEN));
        ui.label("  liq_volume >= 3x avg AND price_velocity < -1%/30s");
        ui.label(RichText::new("SHORT conditions:").strong().color(Color32::from_rgb(0xf8, 0x51, 0x49)));
        ui.label("  liq_volume >= 3x avg AND price_velocity > +1%/30s");
        ui.label("TP: 0.80%, SL: 0.40%");
        ui.label("Fires 1-3 times/day");
        ui.add_space(12.0);

        // ── Strategy 4: Funding Rate Bias ──
        ui.label(
            RichText::new("Strategy 4: Funding Rate Bias (15% weight)")
                .heading()
                .color(ACCENT),
        );
        ui.label("Does not generate standalone signals");
        ui.label("Boosts signal +0.10 when funding > 0.05% or < -0.05%");
        ui.label("Cross-exchange divergence > 0.025% triggers directional bias");
        ui.add_space(12.0);

        // ── Circuit Breaker Rules ──
        ui.label(
            RichText::new("Circuit Breaker Rules")
                .heading()
                .color(Color32::from_rgb(0xf8, 0x51, 0x49)),
        );
        ui.label("  \u{2022} Consecutive losses >= max \u{2192} cooldown");
        ui.label("  \u{2022} Daily loss >= max% \u{2192} halt");
        ui.label("  \u{2022} Drawdown from peak >= max% \u{2192} halt");
        ui.label("  \u{2022} Equity < min_equity \u{2192} halt");
        ui.label("  \u{2022} Trades per hour >= max \u{2192} throttle");
    });
}
