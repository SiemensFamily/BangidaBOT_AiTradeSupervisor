use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub exchanges: ExchangesConfig,
    pub trading: TradingConfig,
    pub risk: RiskConfig,
    pub strategy: StrategyConfig,
    pub database: DatabaseConfig,
    pub monitoring: MonitoringConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GeneralConfig {
    pub mode: String,
    pub log_level: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExchangesConfig {
    pub binance: ExchangeCredentials,
    pub bybit: ExchangeCredentials,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExchangeCredentials {
    pub api_key: String,
    pub api_secret: String,
    pub base_url_rest: String,
    pub base_url_ws: String,
    pub testnet: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TradingConfig {
    pub symbols: Vec<String>,
    pub default_leverage: u32,
    pub max_leverage: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RiskConfig {
    pub max_risk_per_trade_pct: f64,
    pub max_daily_loss_pct: f64,
    pub max_drawdown_pct: f64,
    pub max_consecutive_losses: u32,
    pub cooldown_minutes: u32,
    pub min_equity: f64,
    pub max_open_positions: u32,
    pub max_trades_per_hour: u32,
    #[serde(default = "default_max_leverage")]
    pub max_leverage: u32,
}

fn default_max_leverage() -> u32 {
    20
}

#[derive(Debug, Deserialize, Clone)]
pub struct StrategyConfig {
    pub ob_imbalance: ObImbalanceConfig,
    pub stat_arb: StatArbConfig,
    pub momentum: MomentumConfig,
    pub funding_bias: FundingBiasConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ObImbalanceConfig {
    pub enabled: bool,
    pub weight: f64,
    pub depth_levels: usize,
    pub imbalance_threshold: f64,
    pub hold_time_max_seconds: u64,
    pub take_profit_ticks: u32,
    pub stop_loss_ticks: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StatArbConfig {
    pub enabled: bool,
    pub weight: f64,
    pub spread_window_seconds: u64,
    pub entry_z_score: f64,
    pub min_spread_pct: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MomentumConfig {
    pub enabled: bool,
    pub weight: f64,
    pub breakout_period_seconds: u64,
    pub volume_spike_multiplier: f64,
    pub take_profit_pct: f64,
    pub trailing_stop_pct: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FundingBiasConfig {
    pub enabled: bool,
    pub weight: f64,
    pub high_funding_threshold: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MonitoringConfig {
    pub metrics_port: u16,
}

impl AppConfig {
    /// Load configuration from TOML files + environment variable overrides.
    /// Environment variables use prefix BANGIDA__ with double-underscore separators.
    /// Example: BANGIDA__EXCHANGES__BINANCE__API_KEY=xxx
    pub fn load() -> anyhow::Result<Self> {
        let config = config::Config::builder()
            .add_source(config::File::with_name("config/default"))
            .add_source(config::File::with_name("config/local").required(false))
            .add_source(
                config::Environment::with_prefix("BANGIDA")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?;

        let app_config: AppConfig = config.try_deserialize()?;
        Ok(app_config)
    }

    /// Load with a specific mode override (paper, live, backtest).
    pub fn load_with_mode(mode: &str) -> anyhow::Result<Self> {
        let config = config::Config::builder()
            .add_source(config::File::with_name("config/default"))
            .add_source(config::File::with_name(&format!("config/{}", mode)).required(false))
            .add_source(config::File::with_name("config/local").required(false))
            .add_source(
                config::Environment::with_prefix("BANGIDA")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?;

        let app_config: AppConfig = config.try_deserialize()?;
        Ok(app_config)
    }
}
