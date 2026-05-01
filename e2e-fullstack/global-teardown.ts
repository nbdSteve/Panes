import { cleanupAll } from "./fixtures/tauri-app";

export default async function globalTeardown() {
  await cleanupAll();
}
