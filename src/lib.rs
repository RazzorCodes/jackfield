pub mod components;
pub mod integrations;

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
#[pymodule]
fn jackfield(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<integrations::python::PyMessage>()?;
    m.add_class::<integrations::python::PyMessageBus>()?;
    Ok(())
}
