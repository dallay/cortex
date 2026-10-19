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
