use rook::startup::build_bootstrap_banner;

#[test]
fn bootstrap_banner_renders_the_exact_current_layout() {
    let banner = build_bootstrap_banner("tok-1234");
    assert_eq!(
        banner,
        concat!(
            "+--------------------------------------------------------------+\n",
            "|                ROOK -- BOOTSTRAP MODE ACTIVE                 |\n",
            "+--------------------------------------------------------------+\n",
            "| Setup token: tok-1234                                        |\n",
            "|                                                              |\n",
            "| Paste it into the dashboard to complete setup.               |\n",
            "| To complete setup, open the dashboard at                     |\n",
            "| http://localhost:4747                                        |\n",
            "| along with your desired admin password.                      |\n",
            "+--------------------------------------------------------------+"
        )
    );
}

#[test]
fn bootstrap_banner_preserves_the_sanitized_token_without_masking_it() {
    let token = "A?B?C?D?E?F?G?H?I";
    let banner = build_bootstrap_banner(token);
    assert!(banner.contains("Setup token: A?B?C?D?E?F?G?H?I"));
    assert!(!banner.contains('…'));
}

#[test]
fn bootstrap_banner_widens_to_fit_a_long_token() {
    let token = "t".repeat(80);
    let banner = build_bootstrap_banner(&token);
    let border = banner.lines().next().expect("the banner has a border");
    // The widest line is "Setup token: <token>", so the box grows with it.
    let widest = "Setup token: ".len() + token.len();
    assert!(banner.lines().all(|line| line.len() == border.len()));
    assert_eq!(border.len(), widest + 3);
    assert!(banner.contains(&format!("Setup token: {token}")));
}
