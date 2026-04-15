use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalperConfig {
    pub general: GeneralConfig,
    pub exchanges: ExchangesConfig,
    #[serde(default)]
    pub account: AccountConfig,
    pub risk: RiskConfig,
    #[serde(default)]
    pub execution: ExecutionConfig,
    pub strategy: StrategyConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub trading: TradingConfig,
}

impl ScalperConfig {
    /// Load configuration with layered merging:
    /// config/default.toml → config/{mode}.toml → env vars (SCALPER__*).
    pub fn load(mode: &str) -> anyhow::Result<Self> {
        let builder = config::Config::builder()
            .add_source(config::File::with_name("config/default").required(true))
            .add_source(config::File::with_name(&format!("config/{mode}")).required(false))
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExchangesConfig {
    #[serde(default)]
    pub binance: Option<ExchangeConfig>,
    #[serde(default)]
    pub bybit: Option<ExchangeConfig>,
    #[serde(default)]
    pub okx: Option<ExchangeConfig>,
    #[serde(default)]
    pub kraken: Option<ExchangeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeConfig {
    #[serde(default)]
    pub base_url_rest: String,
    #[serde(default)]
    pub base_url_ws: String,
    #[serde(default)]
    pub testnet: bool,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub api_secret: String,
    #[serde(default)]
    pub passphrase: Option<String>,
    #[serde(default)]
    pub symbol_map: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingConfig {
    #[serde(default = "default_symbols")]
    pub symbols: Vec<String>,
    #[serde(default = "default_leverage")]
    pub default_leverage: u32,
    #[serde(default = "default_leverage")]
    pub max_leverage: u32,
}

fn default_symbols() -> Vec<String> {
    vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()]
}

fn default_leverage() -> u32 {
    10
}

impl Default for TradingConfig {
    fn default() -> Self {
        Self {
            symbols: default_symbols(),
            default_leverage: default_leverage(),
            max_leverage: default_leverage(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountConfig {
    #[serde(default = "default_initial_capital")]
    pub initial_capital: f64,
}

fn default_initial_capital() -> f64 {
    200.0
}

impl Default for AccountConfig {
    fn default() -> Self {
        Self { initial_capital: default_initial_capital() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    #[serde(default)]
    pub max_risk_per_trade: f64,
    #[serde(default)]
    pub daily_drawdown_limit: f64,
    pub max_consecutive_losses: u32,
    pub max_daily_loss_pct: f64,
    pub max_drawdown_pct: f64,
    pub min_equity: f64,
    pub max_trades_per_hour: u32,
    pub cooldown_minutes: u32,
    pub max_risk_per_trade_pct: f64,
    pub max_leverage: u32,
    #[serde(default = "default_max_open_positions")]
    pub max_open_positions: u32,
}

fn default_max_open_positions() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    #[serde(default)]
    pub post_only: bool,
    #[serde(default)]
    pub max_slippage_bps: u32,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            post_only: true,
            max_slippage_bps: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub console: bool,
    #[serde(default)]
    pub file_enabled: bool,
    #[serde(default = "default_log_path")]
    pub file_path: String,
    #[serde(default = "default_perf_log")]
    pub performance_log: String,
    #[serde(default = "default_trade_log")]
    pub trade_log: String,
    #[serde(default = "default_supervisor_log")]
    pub supervisor_log: String,
    #[serde(default = "default_perf_interval")]
    pub performance_summary_interval: u64,
    #[serde(default)]
    pub rotate_daily: bool,
    #[serde(default = "default_max_log_size")]
    pub max_file_size_mb: u64,
}

fn default_log_level() -> String { "info".to_string() }
fn default_log_path() -> String { "logs/bangida_bot.log".to_string() }
fn default_perf_log() -> String { "logs/performance_tracker.log".to_string() }
fn default_trade_log() -> String { "logs/trades.log".to_string() }
fn default_supervisor_log() -> String { "logs/ai_supervisor.log".to_string() }
fn default_perf_interval() -> u64 { 60 }
fn default_max_log_size() -> u64 { 10 }

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            console: true,
            file_enabled: false,
            file_path: default_log_path(),
            performance_log: default_perf_log(),
            trade_log: default_trade_log(),
            supervisor_log: default_supervisor_log(),
            performance_summary_interval: default_perf_interval(),
            rotate_daily: false,
            max_file_size_mb: default_max_log_size(),
        }
    }
}

// ==================== STRATEGY CONFIG (FULL VERSION) ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    #[serde(default)]
    pub ensemble: EnsembleConfig,
    #[serde(default)]
    pub momentum: MomentumConfig,
    #[serde(default)]
    pub ob_imbalance: ObImbalanceConfig,
    #[serde(default)]
    pub liquidation_wick: LiquidationWickConfig,
    #[serde(default)]
    pub funding_bias: FundingBiasConfig,
    #[serde(default)]
    pub supertrend: SupertrendConfig,
    #[serde(default)]
    pub ema_pullback: EmaPullbackConfig,

    #[serde(default)]
    pub mean_reversion: MeanReversionConfig,
    #[serde(default)]
    pub donchian: DonchianConfig,
    #[serde(default)]
    pub ma_cross: MaCrossConfig,

    // New strategies
    #[serde(default)]
    pub cvd_divergence: CvdDivergenceConfig,
    #[serde(default)]
    pub volume_profile: VolumeProfileConfig,
    #[serde(default)]
    pub rsi_fvg: RsiFvgConfig,
    #[serde(default)]
    pub session_retrace: SessionRetraceConfig,

    // Flat fields from TOML (not part of sub-structs)
    #[serde(default)]
    pub ensemble_threshold: f64,
    #[serde(default)]
    pub min_confluence_score: f64,
    #[serde(default)]
    pub min_agreeing_strategies: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnsembleConfig {
    #[serde(default)]
    pub min_strength_threshold: f64,
    /// Minimum ATR-to-price ratio required to approve a signal.
    /// Filters out low-volatility chop where TP/SL targets won't be reached.
    /// E.g. 0.0015 = 0.15% minimum ATR. Set to 0.0 to disable.
    #[serde(default)]
    pub min_atr_ratio: f64,
    /// Minimum number of agreeing strategies (0 or unset = 2 default).
    #[serde(default)]
    pub min_consensus: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MomentumConfig {
    pub enabled: bool,
    #[serde(default)]
    pub volume_spike_multiplier: f64,
    #[serde(default)]
    pub rsi_overbought: f64,
    #[serde(default)]
    pub rsi_oversold: f64,
    #[serde(default)]
    pub take_profit_pct: f64,
    #[serde(default)]
    pub stop_loss_pct: f64,
    #[serde(default)]
    pub weight: f64,
    #[serde(default)]
    pub trailing_stop_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObImbalanceConfig {
    pub enabled: bool,
    #[serde(default)]
    pub min_imbalance_ratio: f64,
    #[serde(default)]
    pub imbalance_threshold: f64,
    #[serde(default)]
    pub take_profit_ticks: u32,
    #[serde(default)]
    pub stop_loss_ticks: u32,
    #[serde(default)]
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LiquidationWickConfig {
    pub enabled: bool,
    #[serde(default)]
    pub volume_spike_multiplier: f64,
    #[serde(default)]
    pub price_velocity_threshold: f64,
    #[serde(default)]
    pub take_profit_pct: f64,
    #[serde(default)]
    pub stop_loss_pct: f64,
    #[serde(default)]
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FundingBiasConfig {
    pub enabled: bool,
    #[serde(default)]
    pub funding_threshold: f64,
    #[serde(default)]
    pub strength_boost: f64,
    #[serde(default)]
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SupertrendConfig {
    pub enabled: bool,
    #[serde(default)]
    pub period: u32,
    #[serde(default)]
    pub multiplier: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmaPullbackConfig {
    pub enabled: bool,
    #[serde(default)]
    pub fast_period: u32,
    #[serde(default)]
    pub slow_period: u32,
    #[serde(default)]
    pub min_pullback_strength: f64,
}

// Legacy configs (with all fields the files expect)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MeanReversionConfig {
    pub enabled: bool,
    #[serde(default)]
    pub bb_penetration: f64,
    #[serde(default)]
    pub rsi_oversold: f64,
    #[serde(default)]
    pub rsi_overbought: f64,
    #[serde(default)]
    pub max_adx: f64,
    #[serde(default)]
    pub atr_tp_multiplier: f64,
    #[serde(default)]
    pub atr_sl_multiplier: f64,
    #[serde(default)]
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DonchianConfig {
    pub enabled: bool,
    #[serde(default)]
    pub entry_period: u32,
    #[serde(default)]
    pub use_trend_filter: bool,
    #[serde(default)]
    pub atr_tp_multiplier: f64,
    #[serde(default)]
    pub atr_stop_multiplier: f64,
    #[serde(default)]
    pub weight: f64,
    #[serde(default)]
    pub exit_period: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MaCrossConfig {
    pub enabled: bool,
    #[serde(default)]
    pub fast_period: u32,
    #[serde(default)]
    pub slow_period: u32,
    #[serde(default)]
    pub min_spread_pct: f64,
    #[serde(default)]
    pub atr_tp_multiplier: f64,
    #[serde(default)]
    pub atr_stop_multiplier: f64,
    #[serde(default)]
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CvdDivergenceConfig {
    pub enabled: bool,
    #[serde(default)]
    pub min_divergence_strength: f64,
    #[serde(default)]
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VolumeProfileConfig {
    pub enabled: bool,
    #[serde(default)]
    pub poc_lookback_bars: u32,
    #[serde(default)]
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RsiFvgConfig {
    pub enabled: bool,
    #[serde(default)]
    pub rsi_overbought: f64,
    #[serde(default)]
    pub rsi_oversold: f64,
    #[serde(default)]
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionRetraceConfig {
    pub enabled: bool,
    #[serde(default)]
    pub weight: f64,
}
