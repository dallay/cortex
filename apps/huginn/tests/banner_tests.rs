use huginn::banner::{print_banner, BANNER};

#[test]
fn banner_constant_is_valid() {
    assert!(!BANNER.is_empty(), "Huginn banner should not be empty");
    assert!(
        BANNER.contains("\x1b[38;2;228;71;55m"),
        "Huginn banner should contain red ANSI color sequence (#e44737)"
    );
    assert!(
        BANNER.lines().count() >= 6,
        "Huginn banner should have at least 6 lines"
    );
}

#[test]
fn print_banner_does_not_panic() {
    print_banner();
}
