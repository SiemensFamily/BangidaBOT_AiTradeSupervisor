use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalperConfig {
    pub general: GeneralConfig,
    pub exchanges: ExchangesConfig,
    pub trading: TradingConfig,
    pub risk: RiskConfig,
    pub strategy: StrategyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangesConfig {
    pub binance: Option<ExchangeConfig>,
    pub bybit: Option<ExchangeConfig>,
    pub okx: Option<OkxExchangeConfig>,
    pub kraken: Option<ExchangeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeConfig {
    pub api_key: String,
    pub api_secret: String,
    pub base_url_rest: String,
    pub base_url_ws: String,
    pub testnet: bool,
    /// Per-exchange symbol mapping, e.g. { "BTCUSDT" = "PI_XBTUSD" } for Kraken.
    #[serde(default)]
    pub symbol_map: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OkxExchangeConfig {
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: String,
    pub base_url_rest: String,
    pub base_url_ws: String,
    pub testnet: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingConfig {
    pub symbols: Vec<String>,
    pub default_leverage: u32,
    pub max_leverage: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    pub max_risk_per_trade_pct: f64,
    pub max_daily_loss_pct: f64,
    pub max_drawdown_pct: f64,
    pub max_consecutive_losses: u32,
    pub cooldown_minutes: u32,
    pub min_equity: f64,
    pub max_open_positions: u32,
    pub max_trades_per_hour: u32,
    pub max_leverage: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    pub ensemble_threshold: f64,
    pub momentum: MomentumConfig,
    pub ob_imbalance: ObImbalanceConfig,
    pub liquidation_wick: LiquidationWickConfig,
    pub funding_bias: FundingBiasConfig,
    #[serde(default = "default_mean_reversion_config")]
    pub mean_reversion: MeanReversionConfig,
}

fn default_mean_reversion_config() -> MeanReversionConfig {
    MeanReversionConfig {
        enabled: false,
        weight: 0.20,
        rsi_oversold: 30.0,
        rsi_overbought: 70.0,
        bb_penetration: 0.05,
        atr_tp_multiplier: 1.5,
        atr_sl_multiplier: 1.0,
        max_adx: 25.0,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MomentumConfig {
    pub enabled: bool,
    pub weight: f64,
    pub volume_spike_multiplier: f64,
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub trailing_stop_pct: f64,
    pub rsi_overbought: f64,
    pub rsi_oversold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObImbalanceConfig {
    pub enabled: bool,
    pub weight: f64,
    pub imbalance_threshold: f64,
    pub take_profit_ticks: u32,
    pub stop_loss_ticks: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidationWickConfig {
    pub enabled: bool,
    pub weight: f64,
    pub price_velocity_threshold: f64,
    pub volume_spike_multiplier: f64,
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingBiasConfig {
    pub enabled: bool,
    pub weight: f64,
    pub funding_threshold: f64,
    pub strength_boost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeanReversionConfig {
    pub enabled: bool,
    pub weight: f64,
    /// RSI threshold below which we consider the market oversold (long entries).
    pub rsi_oversold: f64,
    /// RSI threshold above which we consider the market overbought (short entries).
    pub rsi_overbought: f64,
    /// How far outside the Bollinger band the price must poke, measured
    /// as a fraction of the band-width. 0.05 = 5% of the band-width.
    pub bb_penetration: f64,
    /// Take-profit distance in ATR multiples from entry.
    pub atr_tp_multiplier: f64,
    /// Stop-loss distance in ATR multiples from entry.
    pub atr_sl_multiplier: f64,
    /// Skip entries when ADX is above this value (i.e., market is trending).
    /// Mean reversion works best in ranging markets.
    pub max_adx: f64,
}

impl ScalperConfig {
    /// Load configuration by merging:
    /// 1. config/default.toml (base defaults)
    /// 2. config/{mode}.toml (mode-specific overrides, e.g. paper.toml, live.toml)
    /// 3. Environment variables with prefix "SCALPER" (e.g. SCALPER_GENERAL__MODE)
    pub fn load(mode: &str) -> anyhow::Result<Self> {
        let builder = config::Config::builder()
            .add_source(config::File::with_name("config/default").required(true))
            .add_source(
                config::File::with_name(&format!("config/{}", mode)).required(false),
            )
            .add_source(
                config::Environment::with_prefix("SCALPER")
                    .prefix_separator("__")
                    .separator("__")
                    .try_parsing(true),
            );

        let settings = builder.build()?;
        let mut cfg: ScalperConfig = settings.try_deserialize()?;

        // The config crate's Environment source can wipe nested HashMaps like
        // symbol_map because env vars can only express leaf values. It also
        // lowercases all keys from merged TOML tables, so symbol_map keys
        // arrive as "btcusdt" instead of "BTCUSDT". Re-uppercase the keys and
        // backfill known defaults when the map is empty.
        let normalize_map = |map: &mut std::collections::HashMap<String, String>| {
            let entries: Vec<(String, String)> = map
                .drain()
                .map(|(k, v)| (k.to_uppercase(), v))
                .collect();
            for (k, v) in entries {
                map.insert(k, v);
            }
        };
        if let Some(ref mut kraken) = cfg.exchanges.kraken {
            normalize_map(&mut kraken.symbol_map);
            if kraken.symbol_map.is_empty() {
                kraken.symbol_map.insert("BTCUSDT".into(), "PI_XBTUSD".into());
                kraken.symbol_map.insert("ETHUSDT".into(), "PI_ETHUSD".into());
            }
        }
        if let Some(ref mut binance) = cfg.exchanges.binance {
            normalize_map(&mut binance.symbol_map);
            if binance.symbol_map.is_empty() {
                binance.symbol_map.insert("BTCUSDT".into(), "BTCUSDT".into());
                binance.symbol_map.insert("ETHUSDT".into(), "ETHUSDT".into());
            }
        }
        if let Some(ref mut bybit) = cfg.exchanges.bybit {
            normalize_map(&mut bybit.symbol_map);
            if bybit.symbol_map.is_empty() {
                bybit.symbol_map.insert("BTCUSDT".into(), "BTCUSDT".into());
                bybit.symbol_map.insert("ETHUSDT".into(), "ETHUSDT".into());
            }
        }

        Ok(cfg)
    }
}
