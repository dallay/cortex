/// Parses a duration such as `1h2m3s` or `250ms` into milliseconds.
pub fn parse_reset_milliseconds(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let mut rest = value;
    let mut total = 0.0_f64;
    while !rest.is_empty() {
        let number_end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(rest.len());
        if number_end == 0 {
            return None;
        }
        let amount = rest[..number_end].parse::<f64>().ok()?;
        let unit_end = rest[number_end..]
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .map(|offset| number_end + offset)
            .unwrap_or(rest.len());
        let unit = &rest[number_end..unit_end];
        let multiplier = match unit {
            "h" => 3_600_000.0,
            "m" => 60_000.0,
            "s" => 1_000.0,
            "ms" => 1.0,
            _ => return None,
        };
        total += amount * multiplier;
        if !total.is_finite() || total > u64::MAX as f64 || unit.is_empty() {
            return None;
        }
        rest = &rest[unit_end..];
    }
    Some(total.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::parse_reset_milliseconds;

    #[test]
    fn parses_composite_and_fractional_durations() {
        assert_eq!(parse_reset_milliseconds("1h2m3.5s"), Some(3_723_500));
        assert_eq!(parse_reset_milliseconds("250ms"), Some(250));
        assert_eq!(parse_reset_milliseconds(" 0.5s "), Some(500));
    }

    #[test]
    fn rejects_empty_malformed_unknown_and_overflow_values() {
        for value in [
            "",
            "   ",
            "1",
            "1x",
            "s",
            "1.2.3s",
            "1ms2",
            "999999999999999999999999h",
        ] {
            assert_eq!(parse_reset_milliseconds(value), None, "input: {value:?}");
        }
    }
}
