use std::time::Duration;

#[derive(Debug, Clone)]
pub struct QueueConfig {
    pub poll_interval: Duration,
    pub batch_size: usize,
    pub default_timeout: Duration,
    pub default_max_retries: u32,
    pub cleanup_interval: Duration,
    pub vacuum_interval: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(100),
            batch_size: 100,
            default_timeout: Duration::from_secs(60),
            default_max_retries: 3,
            cleanup_interval: Duration::from_secs(60),
            vacuum_interval: Duration::from_secs(300),
        }
    }
}

impl QueueConfig {
    pub fn from_env() -> Self {
        Self {
            poll_interval: parse_duration_env("KRUXIAFLOW_QUEUE_POLL_INTERVAL")
                .unwrap_or_else(|| Duration::from_millis(100)),
            batch_size: parse_env("KRUXIAFLOW_QUEUE_BATCH_SIZE").unwrap_or(100),
            default_timeout: parse_duration_env("KRUXIAFLOW_QUEUE_DEFAULT_TIMEOUT")
                .unwrap_or_else(|| Duration::from_secs(60)),
            default_max_retries: parse_env("KRUXIAFLOW_QUEUE_DEFAULT_MAX_RETRIES").unwrap_or(3),
            cleanup_interval: parse_duration_env("KRUXIAFLOW_QUEUE_CLEANUP_INTERVAL")
                .unwrap_or_else(|| Duration::from_secs(60)),
            vacuum_interval: parse_duration_env("KRUXIAFLOW_QUEUE_VACUUM_INTERVAL")
                .unwrap_or_else(|| Duration::from_secs(300)),
        }
    }
}

fn parse_env<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok()?.parse().ok()
}

fn parse_duration_env(key: &str) -> Option<Duration> {
    let s = std::env::var(key).ok()?;
    parse_duration(&s)
}

fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if let Some(ms) = s.strip_suffix("ms") {
        ms.parse::<u64>().ok().map(Duration::from_millis)
    } else if let Some(secs) = s.strip_suffix('s') {
        secs.parse::<u64>().ok().map(Duration::from_secs)
    } else if let Some(mins) = s.strip_suffix('m') {
        mins.parse::<u64>()
            .ok()
            .map(|m| Duration::from_secs(m * 60))
    } else {
        s.parse::<u64>().ok().map(Duration::from_secs)
    }
}
