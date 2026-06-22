use std::time::Duration;

pub(super) fn format_duration_ns(duration_ns: u64) -> String {
    let duration = Duration::from_nanos(duration_ns);
    let elapsed = duration.as_secs_f64();
    if elapsed < 0.001 {
        let micros = elapsed * 1e6;
        let decimals = if micros >= 100.0 {
            0
        } else if micros >= 10.0 {
            1
        } else {
            2
        };
        format!("{micros:.decimals$}µs")
    } else if elapsed < 1.0 {
        let millis = elapsed * 1e3;
        let decimals = if millis >= 100.0 {
            0
        } else if millis >= 10.0 {
            1
        } else {
            2
        };
        format!("{millis:.decimals$}ms")
    } else if elapsed < 60.0 {
        let decimals = if elapsed >= 10.0 { 1 } else { 2 };
        format!("{elapsed:.decimals$}s")
    } else {
        let total_seconds = duration.as_secs();
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        format!("{minutes}m{seconds:02}s")
    }
}

#[cfg(test)]
mod tests {
    use super::format_duration_ns;

    #[test]
    fn formats_sub_millisecond_runtime_in_microseconds() {
        assert_eq!(format_duration_ns(999), "1.00µs");
        assert_eq!(format_duration_ns(12_300), "12.3µs");
        assert_eq!(format_duration_ns(123_000), "123µs");
    }

    #[test]
    fn formats_sub_second_runtime_in_milliseconds() {
        assert_eq!(format_duration_ns(1_230_000), "1.23ms");
        assert_eq!(format_duration_ns(12_300_000), "12.3ms");
        assert_eq!(format_duration_ns(123_000_000), "123ms");
    }

    #[test]
    fn formats_seconds_and_minutes() {
        assert_eq!(format_duration_ns(1_230_000_000), "1.23s");
        assert_eq!(format_duration_ns(12_300_000_000), "12.3s");
        assert_eq!(format_duration_ns(61_000_000_000), "1m01s");
    }
}
