// PyO3 exports — Rust side
#[pyfunction]
fn hello(name: &str) -> String {
    format!("Hello, {name}!")
}

#[pyclass]
struct Config {
    debug: bool,
}

#[pymethods]
impl Config {
    fn is_debug(&self) -> bool {
        self.debug
    }
}
