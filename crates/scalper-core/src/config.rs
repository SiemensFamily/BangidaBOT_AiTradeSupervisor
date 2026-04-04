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
                    .separator("__")
                    .try_parsing(true),
            );

        let settings = builder.build()?;
        let cfg: ScalperConfig = settings.try_deserialize()?;
        Ok(cfg)
    }
}
