// Compile-time proof (typechecked by `tsc --noEmit`, never built or shipped)
// that the structural `ConnectTransport` and `@connectrpc/connect`'s
// `Transport` are assignable to each other, so hosts can bind real transports
// and clients can `registerTransport(hostTransport(id))`.
import type { Transport } from "@connectrpc/connect";
import { hostTransport } from "./registry.js";
import type { ConnectTransport } from "./types.js";

declare const real: Transport;
export const intoHost: ConnectTransport = real;
export const intoClient: Transport = hostTransport("headwaters");
