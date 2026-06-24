// @generated — do not edit by hand.
// TypeScript declarations for the wasm-bindgen browser client.
// Request/response values are plain objects (the JSON form of the proto messages).

import type {
  CreateGreetingRequest,
  GetGreetingRequest,
  Greeting,
} from "./models";

/** WASM/browser binding for the `greeting` service. */
export class GreetingServiceClient {
  /** Create a new greeting. */
  createGreeting(request: CreateGreetingRequest): Promise<Greeting>;
  /** Fetch a greeting by name. */
  getGreeting(request: GetGreetingRequest): Promise<Greeting>;
}

/** Browser entry point. Construct from a base URL; the browser manages the session
   by default, or pass options for bearer-token auth. */
export class GoldenPathAppClient {
  /**
   * @param baseUrl Absolute base URL of the API (same-origin for cookie auth).
   * @param options Optional auth/session settings. Omit to keep the default
   *   browser-session (cookie) behavior, where `fetch` sends credentials with
   *   `credentials: "include"`.
   *
   *   - `authToken` — when set, every request carries
   *     `Authorization: Bearer <authToken>`. Unless `credentials` is given too,
   *     this also switches the `fetch` credentials mode to `"omit"` so a stale
   *     cookie can't shadow the header.
   *   - `credentials` — the `fetch` credentials mode: `"include"` (default),
   *     `"same-origin"`, or `"omit"`.
   */
  constructor(baseUrl: string, options?: { authToken?: string; credentials?: "include" | "same-origin" | "omit" });
  /** Access the `greeting` service. */
  greeting(): GreetingServiceClient;
}
