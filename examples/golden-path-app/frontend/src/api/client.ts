// @generated — do not edit by hand.
import { fromBinary, toBinary } from "@bufbuild/protobuf";
import {
  type Greeting,
  GreetingSchema,
} from "./models";
import {
  NapiGoldenPathAppClient as NativeClient,
  NapiGreetingClient as NativeGreetingClient,
} from "./native";

// ── ApiError error hierarchy ────────────────────────────────────────────────────────

/** Base class for all ApiError errors. */
export class ApiError extends Error {
  readonly errorCode: string;
  constructor(message: string, errorCode: string) {
    super(message);
    this.name = "ApiError";
    this.errorCode = errorCode;
  }
}

export class NotFoundError extends ApiError {
  constructor(message: string) {
    super(message, "RESOURCE_NOT_FOUND");
    this.name = "NotFoundError";
  }
}

export class AlreadyExistsError extends ApiError {
  constructor(message: string) {
    super(message, "RESOURCE_ALREADY_EXISTS");
    this.name = "AlreadyExistsError";
  }
}

export class PermissionDeniedError extends ApiError {
  constructor(message: string) {
    super(message, "PERMISSION_DENIED");
    this.name = "PermissionDeniedError";
  }
}

export class UnauthenticatedError extends ApiError {
  constructor(message: string) {
    super(message, "UNAUTHENTICATED");
    this.name = "UnauthenticatedError";
  }
}

export class InvalidParameterError extends ApiError {
  constructor(message: string) {
    super(message, "INVALID_PARAMETER_VALUE");
    this.name = "InvalidParameterError";
  }
}

export class RequestLimitError extends ApiError {
  constructor(message: string) {
    super(message, "REQUEST_LIMIT_EXCEEDED");
    this.name = "RequestLimitError";
  }
}

export class InternalServerError extends ApiError {
  constructor(message: string) {
    super(message, "INTERNAL_ERROR");
    this.name = "InternalServerError";
  }
}

export class ServiceUnavailableError extends ApiError {
  constructor(message: string) {
    super(message, "TEMPORARILY_UNAVAILABLE");
    this.name = "ServiceUnavailableError";
  }
}

type ErrorConstructor = new (message: string) => ApiError;

const ERROR_MAP: Record<string, ErrorConstructor> = {
  RESOURCE_NOT_FOUND: NotFoundError,
  RESOURCE_ALREADY_EXISTS: AlreadyExistsError,
  PERMISSION_DENIED: PermissionDeniedError,
  UNAUTHENTICATED: UnauthenticatedError,
  INVALID_PARAMETER_VALUE: InvalidParameterError,
  REQUEST_LIMIT_EXCEEDED: RequestLimitError,
  INTERNAL_ERROR: InternalServerError,
  TEMPORARILY_UNAVAILABLE: ServiceUnavailableError,
};

/**
 * Parse a native NAPI error that may carry a `GOLDEN_PATH_APP_:<CODE>:<message>` prefix
 * and re-throw as the appropriate typed subclass of `ApiError`.
 */
function parseNativeError(e: unknown): never {
  if (e instanceof Error) {
    const match = e.message.match(/^GOLDEN_PATH_APP_:([^:]+):([\s\S]*)$/);
    if (match) {
      const [, code, message] = match;
      const Ctor = ERROR_MAP[code] ?? ApiError;
      throw new Ctor(message);
    }
  }
  throw e;
}

// ── end ApiError error hierarchy ─────────────────────────────────────────────────────

export class GreetingClient {
  private readonly inner: NativeGreetingClient;

  /** @internal */
  constructor(inner: NativeGreetingClient) {
    this.inner = inner;
  }

  /**
     * Fetch a greeting by name.
     */
  async get(): Promise<Greeting> {
    try {
      return fromBinary(GreetingSchema, await this.inner.get());
    } catch (e) { throw parseNativeError(e); }
  }

}

export class GoldenPathAppClient {
  private readonly inner: NativeClient;

  constructor(url: string, token?: string) {
    this.inner = NativeClient.fromUrl(url, token);
  }

  /**
     * Create a new greeting.
     */
  async createGreeting(greeting: Greeting): Promise<Greeting> {
    try {
      return fromBinary(GreetingSchema, await this.inner.createGreeting(Buffer.from(toBinary(GreetingSchema, greeting))));
    } catch (e) { throw parseNativeError(e); }
  }

  greeting(greetingName: string): GreetingClient {
    return new GreetingClient(this.inner.greeting(greetingName));
  }

}
