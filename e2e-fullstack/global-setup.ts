import { startBackend, startVite } from "./fixtures/tauri-app";

export default async function globalSetup() {
  await startBackend();
  await startVite();
}
