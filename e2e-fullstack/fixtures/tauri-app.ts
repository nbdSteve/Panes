import { spawn, execSync, type ChildProcess } from "child_process";
import { mkdtempSync, rmSync } from "fs";
import { createConnection } from "net";
import { tmpdir } from "os";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const PROJECT_ROOT = resolve(__dirname, "../..");
const BINARY = resolve(PROJECT_ROOT, "target/debug/Panes");

let built = false;
let backendProcess: ChildProcess | null = null;
let viteProcess: ChildProcess | null = null;
let dataDir: string | null = null;

function ensureBinary() {
  if (built) return;
  execSync("cargo build -p panes-app", {
    cwd: PROJECT_ROOT,
    stdio: "inherit",
    timeout: 120_000,
  });
  built = true;
}

async function waitForPort(port: number, timeout = 15_000): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const ok = await new Promise<boolean>((res) => {
      const sock = createConnection({ port, host: "127.0.0.1" }, () => {
        sock.destroy();
        res(true);
      });
      sock.on("error", () => res(false));
      sock.setTimeout(1000, () => { sock.destroy(); res(false); });
    });
    if (ok) return;
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`Port ${port} not ready after ${timeout}ms`);
}

async function waitForWs(port: number, timeout = 15_000): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    try {
      const ws = new (await import("ws")).default(`ws://127.0.0.1:${port}/ws`);
      await new Promise<void>((resolve, reject) => {
        ws.on("open", () => { ws.close(); resolve(); });
        ws.on("error", () => reject());
        setTimeout(() => reject(), 2000);
      });
      return;
    } catch {}
    await new Promise((r) => setTimeout(r, 300));
  }
  throw new Error(`WS port ${port} not ready after ${timeout}ms`);
}

export async function startBackend(): Promise<void> {
  if (backendProcess) return;
  ensureBinary();

  dataDir = mkdtempSync(resolve(tmpdir(), "panes-e2e-data-"));

  backendProcess = spawn(BINARY, [], {
    env: {
      ...process.env,
      PANES_TEST_MODE: "1",
      PANES_DATA_DIR: dataDir,
    },
    stdio: ["pipe", "pipe", "pipe"],
  });

  backendProcess.stderr?.on("data", (d: Buffer) => {
    const msg = d.toString().trim();
    if (msg) console.error("[backend]", msg);
  });

  await waitForWs(3001);
}

export async function startVite(): Promise<void> {
  if (viteProcess) return;

  viteProcess = spawn("npx", ["vite", "--port", "5174", "--strictPort", "--host", "127.0.0.1"], {
    cwd: PROJECT_ROOT,
    env: {
      ...process.env,
      VITE_FULLSTACK_TEST: "1",
    },
    stdio: ["pipe", "pipe", "pipe"],
  });

  viteProcess.stdout?.on("data", (d: Buffer) => {
    const msg = d.toString().trim();
    if (msg) console.error("[vite]", msg);
  });

  viteProcess.stderr?.on("data", (d: Buffer) => {
    const msg = d.toString().trim();
    if (msg) console.error("[vite:err]", msg);
  });

  await waitForPort(5174, 30_000);
}

export function getDataDir(): string {
  return dataDir!;
}

export async function cleanupAll(): Promise<void> {
  if (viteProcess) {
    viteProcess.kill("SIGTERM");
    viteProcess = null;
  }
  if (backendProcess) {
    backendProcess.kill("SIGTERM");
    backendProcess = null;
  }
  if (dataDir) {
    rmSync(dataDir, { recursive: true, force: true });
    dataDir = null;
  }
}
