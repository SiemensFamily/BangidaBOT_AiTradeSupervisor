// OKX module stub - not used in paper mode (Kraken is the active exchange)
// This prevents compilation errors while keeping the crate structure intact.

pub struct OkxClient {}
impl OkxClient {
    pub fn new() -> Self {
        Self {}
    }
}

pub struct OkxWsFeed {}
impl OkxWsFeed {
    pub fn new() -> Self {
        Self {}
    }
}

// Re-exports to satisfy the parent crate
pub use self::OkxClient as rest;
pub use self::OkxWsFeed as ws;