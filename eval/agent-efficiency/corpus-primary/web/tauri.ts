import { invoke } from "@tauri-apps/api/core";

export const refreshProject = () => invoke<boolean>("refresh_project");
