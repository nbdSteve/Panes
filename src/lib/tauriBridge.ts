type Callback = (payload: unknown) => void;

const callbacks: Map<number, Callback> = new Map();
let nextCallbackId = 1;

interface EventListener {
  id: number;
  event: string;
  handlerCallbackId: number;
}

const eventListeners: EventListener[] = [];
let nextEventId = 1;

let ws: WebSocket | null = null;
let pendingRequests: Map<string, { resolve: (v: unknown) => void; reject: (e: Error) => void }> = new Map();
let requestId = 0;

function emitEvent(event: string, payload: unknown) {
  for (const listener of eventListeners) {
    if (listener.event === event) {
      const cb = callbacks.get(listener.handlerCallbackId);
      if (cb) {
        cb({ id: listener.id, event, payload });
      }
    }
  }
}

let connecting: Promise<void> | null = null;

function setupWsHandlers(socket: WebSocket) {
  socket.onmessage = (msg) => {
    try {
      const data = JSON.parse(msg.data);
      if (data.type === "event") {
        emitEvent("panes://thread-event", data.payload);
      } else if (data.type === "response") {
        const pending = pendingRequests.get(data.id);
        if (pending) {
          pendingRequests.delete(data.id);
          if (data.error) {
            pending.reject(new Error(data.error));
          } else {
            pending.resolve(data.ok);
          }
        }
      }
    } catch {}
  };

  socket.onclose = () => {
    ws = null;
    connecting = null;
    for (const [, p] of pendingRequests) {
      p.reject(new Error("WebSocket closed"));
    }
    pendingRequests.clear();
  };
}

function connectOnce(): Promise<void> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket("ws://127.0.0.1:3001/ws");

    socket.onopen = () => {
      ws = socket;
      setupWsHandlers(socket);
      resolve();
    };

    socket.onerror = () => {
      reject(new Error("WebSocket connection failed"));
    };
  });
}

async function connect(): Promise<void> {
  if (ws && ws.readyState === WebSocket.OPEN) return;
  if (connecting) return connecting;

  connecting = (async () => {
    for (let attempt = 0; attempt < 10; attempt++) {
      try {
        await connectOnce();
        connecting = null;
        return;
      } catch {
        if (attempt < 9) await new Promise((r) => setTimeout(r, 300));
      }
    }
    connecting = null;
    throw new Error("WebSocket connection failed after retries");
  })();

  return connecting;
}

async function bridgeInvoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  if (cmd === "plugin:event|listen") {
    const eventName = args?.event as string;
    const handlerCbId = args?.handler as number;
    const id = nextEventId++;
    eventListeners.push({ id, event: eventName, handlerCallbackId: handlerCbId });
    return id;
  }

  if (cmd === "plugin:event|unlisten") {
    const eventId = args?.eventId as number;
    const idx = eventListeners.findIndex((l) => l.id === eventId);
    if (idx >= 0) eventListeners.splice(idx, 1);
    return null;
  }

  await connect();

  const id = String(++requestId);
  return new Promise((resolve, reject) => {
    pendingRequests.set(id, { resolve, reject });
    ws!.send(JSON.stringify({ id, cmd, args: args || {} }));
  });
}

export async function installTauriBridge(): Promise<void> {
  await connect();

  (window as any).__TAURI_INTERNALS__ = {
    invoke: bridgeInvoke,
    transformCallback(callback: Callback, _once?: boolean): number {
      const id = nextCallbackId++;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback(id: number) {
      callbacks.delete(id);
    },
    convertFileSrc(path: string) {
      return path;
    },
  };

  (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(_event: string, _eventId: number) {},
  };
}
