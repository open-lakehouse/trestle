import { useEffect, useState } from "react";

// The browser client is generated from your proto and compiled to WASM by
// `just build-wasm` (output in `src/wasm/`). It speaks to the same origin the
// app is served from, so this works locally (via Vite's /v1 proxy) and on
// Databricks (via the platform edge) with zero env-conditional code.
import init, { GoldenPathAppClient } from "@/wasm/client";

interface Greeting {
  name: string;
  recipient: string;
  message: string;
}

const PAGE_STYLE = { fontFamily: "system-ui", padding: 24 } as const;
const INPUT_STYLE = { padding: 8, marginRight: 8 } as const;
const RESULT_STYLE = { marginTop: 16 } as const;
const ERROR_STYLE = { color: "crimson" } as const;

export default function App() {
  const [client, setClient] = useState<GoldenPathAppClient | null>(null);
  const [recipient, setRecipient] = useState("world");
  const [greeting, setGreeting] = useState<Greeting | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Initialize the WASM module once, then construct the generated client.
  useEffect(() => {
    init()
      .then(() => setClient(new GoldenPathAppClient(window.location.origin)))
      .catch((e) => setError(String(e)));
  }, []);


  return (
    <div style={PAGE_STYLE}>
      <h1>golden-path-app</h1>
      <p>
        Local Databricks-style Rust app, scaffolded by trestle. The greeting
        below is created through the generated WASM client.
      </p>

      <input
        value={recipient}
        onChange={(e) => setRecipient(e.target.value)}
        style={INPUT_STYLE}
      />
      <button
        disabled={!client}
        onClick={async () => {
          try {
            // `greeting()` is the per-service accessor; `createGreeting` takes the
            // RPC request ({ greeting: { recipient } }) and returns the created resource.
            const created = await client!
              .greeting()
              .createGreeting({ greeting: { recipient } });
            setGreeting(created as Greeting);
            setError(null);
          } catch (e) {
            setError(String(e));
          }
        }}
      >
        Greet
      </button>

      {greeting && (
        <pre style={RESULT_STYLE}>{JSON.stringify(greeting, null, 2)}</pre>
      )}
      {error && <p style={ERROR_STYLE}>{error}</p>}
    </div>
  );
}
