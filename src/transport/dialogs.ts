import { message } from "@tauri-apps/plugin-dialog";

/** Native error dialog. File pickers stay on the Rust side so paths never become a renderer capability. */
export async function showErrorDialog(title: string, body: string): Promise<void> {
  await message(body, { title, kind: "error" });
}
