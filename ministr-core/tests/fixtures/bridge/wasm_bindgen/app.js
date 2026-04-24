// wasm-bindgen imports — JavaScript side
import { greet, fibonacci, Counter } from './pkg/my_wasm';

const msg = greet("World");
const fib = fibonacci(10);
const counter = new Counter();
