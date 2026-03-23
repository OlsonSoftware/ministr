// Tauri command imports — client-side TypeScript
import { invoke } from "@tauri-apps/api/core";

const greeting = await invoke("greet", { name: "World" });
const settings = await invoke("get_settings");
const result = await invoke("save_file", { path: "/tmp/test", content: "hello" });
