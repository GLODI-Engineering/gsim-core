//! wasm-bindgen bindings for `gsim-core`. Empty smoke-test surface until the contract
//! schema/expression interpreter/document-builder design lands — see
//! `../gsim-core/src/lib.rs` and `docs/decisions/`.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn version() -> String {
    gsim_core::version().to_string()
}
