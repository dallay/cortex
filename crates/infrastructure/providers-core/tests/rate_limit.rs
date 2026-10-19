use providers_core::parse_reset_milliseconds;

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
