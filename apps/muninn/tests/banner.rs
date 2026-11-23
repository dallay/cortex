//! Banner module integration tests.

use muninn::banner::{print_banner, BANNER};

#[test]
fn banner_not_empty() {
    assert!(!BANNER.is_empty());
    assert!(BANNER.contains("\x1b[38;2;228;71;55m"));
    assert!(BANNER.lines().count() >= 6);
}

#[test]
fn print_banner_runs() {
    print_banner();
}
