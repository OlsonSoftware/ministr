#[pyo3::pyfunction]
pub fn normalize_record(value: String) -> String {
    value.trim().to_owned()
}
