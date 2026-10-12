use rook_usecases::oauth_tokens::token_requires_refresh;

#[test]
fn refresh_is_due_inside_the_safety_window() {
    assert!(token_requires_refresh(1_000, 900, 100));
    assert!(token_requires_refresh(800, 900, 100));
    assert!(!token_requires_refresh(1_001, 900, 100));
}
