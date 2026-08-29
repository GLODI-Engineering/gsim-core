//! PyO3 bindings for `gsim-core`. Empty smoke-test surface until the contract schema/expression
//! interpreter/document-builder design lands — see `../gsim-core/src/lib.rs` and
//! `docs/decisions/`.

use pyo3::prelude::*;

#[pyfunction]
fn version() -> &'static str {
    gsim_core::version()
}

#[pymodule]
fn gsim_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(version, m)?)?;
    Ok(())
}
