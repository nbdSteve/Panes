import { spawn, execSync, type ChildProcess } from "child_process";
import { mkdtempSync, rmSync, writeFileSync, readFileSync, existsSync } from "fs";
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
    timeout: 600_000,
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

/// Well-known path Playwright workers read to discover the fullstack
/// backend's data dir — globalSetup runs in a different Node process so
/// module-level state can't be shared directly.
const DATA_DIR_MARKER = resolve(tmpdir(), "panes-e2e-datadir");

export async function startBackend(): Promise<void> {
  if (backendProcess) return;
  ensureBinary();

  dataDir = mkdtempSync(resolve(tmpdir(), "panes-e2e-data-"));
  // Publish the dir to the marker file so test workers can find it.
  writeFileSync(DATA_DIR_MARKER, dataDir);

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
  if (dataDir) return dataDir;
  // Test workers read the marker file that globalSetup wrote.
  if (existsSync(DATA_DIR_MARKER)) {
    return readFileSync(DATA_DIR_MARKER, "utf8").trim();
  }
  throw new Error("panes-e2e data dir not yet published — startBackend must run before test workers");
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
  if (existsSync(DATA_DIR_MARKER)) {
    rmSync(DATA_DIR_MARKER, { force: true });
  }
}
