pub const fn default_telnet_keep_alive_interval() -> u32 {
    60
}

/// Start after a full interval and skip missed ticks instead of sending a burst.
pub fn telnet_keep_alive_interval(config: &TelnetSessionConfig) -> Option<tokio::time::Interval> {
    if config.raw_tcp_cli || config.keep_alive_interval == 0 {
        return None;
    }
    let period = std::time::Duration::from_secs(u64::from(config.keep_alive_interval));
    let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    Some(timer)
}

/// Disabled timers stay pending, including when polled in a session select loop.
pub async fn tick_telnet_keep_alive(timer: &mut Option<tokio::time::Interval>) {
    match timer {
        Some(timer) => {
            timer.tick().await;
        }
        None => std::future::pending::<()>().await,
    }
}
