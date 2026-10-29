//! Generated canonical ABI bindings shared by guests and conformance fixtures.
//! Keep generation in a dependency so exported external macros retain their
//! generator's hygiene. Every workspace crate still denies handwritten unsafe.
#[cfg(target_arch = "wasm32")]
mod bindings {
    wit_bindgen::generate!({
        path: "../plugin-api/wit/experimental-rust-outline",
        world: "rust-outline",
        pub_export_macro: true,
    });
}
#[cfg(target_arch = "wasm32")]
pub use bindings::*;
