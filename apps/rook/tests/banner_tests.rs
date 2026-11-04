use rook::banner::{print_banner, BANNER};

#[test]
fn banner_constant_is_valid() {
    assert!(!BANNER.is_empty(), "Rook banner should not be empty");
    assert!(
        BANNER.contains("\x1b[38;2;228;71;55m"),
        "Rook banner should contain red ANSI color sequence (#e44737)"
    );
    assert!(
        BANNER.lines().count() >= 6,
        "Rook banner should have at least 6 lines"
    );
}

#[test]
fn print_banner_does_not_panic() {
    print_banner();
}
