use huginn_core::{ImageAttachment, Message, Session, MAX_IMAGES_PER_MESSAGE};
use huginn_presentation::{
    clipboard::{parse_pasted_image_path, try_load_pasted_image_path},
    state::UiState,
};
use std::path::PathBuf;

#[test]
fn parses_quoted_and_escaped_image_paths_only() {
    assert_eq!(
        parse_pasted_image_path("'/tmp/my shot.png'"),
        Some(PathBuf::from("/tmp/my shot.png"))
    );
    assert_eq!(
        parse_pasted_image_path(r"/tmp/my\ shot.webp"),
        Some(PathBuf::from("/tmp/my shot.webp"))
    );
    assert_eq!(parse_pasted_image_path("hello world"), None);
    assert_eq!(parse_pasted_image_path("/tmp/notes.txt"), None);
}

#[test]
fn loads_valid_png_path_and_ui_state_manages_preview_and_session_budget() {
    let dir = tempfile::tempdir().unwrap();
    let temp = dir.path().join("sample.png");
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&32u32.to_be_bytes());
    png.extend_from_slice(&16u32.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0]);
    std::fs::write(&temp, &png).unwrap();

    let loaded = try_load_pasted_image_path(&format!("'{}'", temp.display()))
        .unwrap()
        .unwrap();

    let mut state = UiState::default();
    for _ in 0..MAX_IMAGES_PER_MESSAGE {
        state.attach_image(loaded.clone()).unwrap();
    }
    assert!(state.attach_image(loaded.clone()).is_err());
    assert_eq!(state.attachment_chips()[0], "[Image 1: PNG 32x16, 29 B]");
    assert!(state.remove_last_attachment());
    assert_eq!(state.attachments.len(), MAX_IMAGES_PER_MESSAGE - 1);

    // Synchronizing a session with existing in-memory images updates the session byte budget.
    let mut session = Session::new(dir.path().to_path_buf());
    let mut big_png = png.clone();
    big_png.resize(huginn_core::MAX_IMAGE_BYTES, 0);
    let big = ImageAttachment::from_bytes(big_png, Some("big.png".into())).unwrap();
    session.messages.push(Message::user_with_images(
        "prior",
        vec![big.clone(), big.clone(), big.clone()],
    ));
    let mut budgeted = UiState::default();
    budgeted.synchronize(&session);
    // 3 * 5 MiB = 15 MiB already in session; attaching another 5 MiB exceeds 16 MiB.
    assert!(budgeted.attach_image(big).is_err());
}
