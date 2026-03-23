// NAPI-RS exports — Rust side
#[napi]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[napi]
fn get_version() -> String {
    "1.0.0".into()
}

#[napi(constructor)]
pub struct Calculator {
    pub value: f64,
}
