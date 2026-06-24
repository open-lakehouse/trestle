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

/** Browser entry point. Construct from a base URL; the browser manages the session. */
export class GoldenPathAppClient {
  /** @param baseUrl Absolute base URL of the API (same-origin for cookie auth). */
  constructor(baseUrl: string);
  /** Access the `greeting` service. */
  greeting(): GreetingServiceClient;
}
