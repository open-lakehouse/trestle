// @generated — do not edit by hand.
import { fromBinary, toBinary } from "@bufbuild/protobuf";
import {
  type Catalog,
  type DeleteCatalogResponse,
  type DeleteSchemaResponse,
  type DeleteTagAssignmentResponse,
  type ListByTagsResponse,
  type ListTagAssignmentsResponse,
  type Schema,
  type TagAssignment,
  CatalogSchema,
  DeleteCatalogResponseSchema,
  DeleteSchemaResponseSchema,
  DeleteTagAssignmentResponseSchema,
  ListByTagsResponseSchema,
  ListTagAssignmentsResponseSchema,
  SchemaSchema,
  TagAssignmentSchema,
} from "./models";
import {
  NapiCatalogClient as NativeCatalogClient,
  NapiExampleClient as NativeClient,
  NapiSchemaClient as NativeSchemaClient,
} from "./native";

// ── ExampleError error hierarchy ────────────────────────────────────────────────────────

/** Base class for all ExampleError errors. */
export class ExampleError extends Error {
  readonly errorCode: string;
  constructor(message: string, errorCode: string) {
    super(message);
    this.name = "ExampleError";
    this.errorCode = errorCode;
  }
}

export class NotFoundError extends ExampleError {
  constructor(message: string) {
    super(message, "RESOURCE_NOT_FOUND");
    this.name = "NotFoundError";
  }
}

export class AlreadyExistsError extends ExampleError {
  constructor(message: string) {
    super(message, "RESOURCE_ALREADY_EXISTS");
    this.name = "AlreadyExistsError";
  }
}

export class PermissionDeniedError extends ExampleError {
  constructor(message: string) {
    super(message, "PERMISSION_DENIED");
    this.name = "PermissionDeniedError";
  }
}

export class UnauthenticatedError extends ExampleError {
  constructor(message: string) {
    super(message, "UNAUTHENTICATED");
    this.name = "UnauthenticatedError";
  }
}

export class InvalidParameterError extends ExampleError {
  constructor(message: string) {
    super(message, "INVALID_PARAMETER_VALUE");
    this.name = "InvalidParameterError";
  }
}

export class RequestLimitError extends ExampleError {
  constructor(message: string) {
    super(message, "REQUEST_LIMIT_EXCEEDED");
    this.name = "RequestLimitError";
  }
}

export class InternalServerError extends ExampleError {
  constructor(message: string) {
    super(message, "INTERNAL_ERROR");
    this.name = "InternalServerError";
  }
}

export class ServiceUnavailableError extends ExampleError {
  constructor(message: string) {
    super(message, "TEMPORARILY_UNAVAILABLE");
    this.name = "ServiceUnavailableError";
  }
}

type ErrorConstructor = new (message: string) => ExampleError;

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
 * Parse a native NAPI error that may carry a `EX:<CODE>:<message>` prefix
 * and re-throw as the appropriate typed subclass of `ExampleError`.
 */
function parseNativeError(e: unknown): never {
  if (e instanceof Error) {
    const match = e.message.match(/^EX:([^:]+):([\s\S]*)$/);
    if (match) {
      const [, code, message] = match;
      const Ctor = ERROR_MAP[code] ?? ExampleError;
      throw new Ctor(message);
    }
  }
  throw e;
}

// ── end ExampleError error hierarchy ─────────────────────────────────────────────────────

export class CatalogClient {
  private readonly inner: NativeCatalogClient;

  /** @internal */
  constructor(inner: NativeCatalogClient) {
    this.inner = inner;
  }

  async get(): Promise<Catalog> {
    try {
      return fromBinary(CatalogSchema, await this.inner.get());
    } catch (e) { throw parseNativeError(e); }
  }

  async update(): Promise<Catalog> {
    try {
      return fromBinary(CatalogSchema, await this.inner.update());
    } catch (e) { throw parseNativeError(e); }
  }

  async delete(): Promise<void> {
    try {
      await this.inner.delete();
    } catch (e) { throw parseNativeError(e); }
  }

}

export class SchemaClient {
  private readonly inner: NativeSchemaClient;

  /** @internal */
  constructor(inner: NativeSchemaClient) {
    this.inner = inner;
  }

  async get(view: number): Promise<Schema> {
    try {
      return fromBinary(SchemaSchema, await this.inner.get(view));
    } catch (e) { throw parseNativeError(e); }
  }

  async update(): Promise<Schema> {
    try {
      return fromBinary(SchemaSchema, await this.inner.update());
    } catch (e) { throw parseNativeError(e); }
  }

  async delete(): Promise<void> {
    try {
      await this.inner.delete();
    } catch (e) { throw parseNativeError(e); }
  }

}

export class ExampleClient {
  private readonly inner: NativeClient;

  constructor(url: string, token?: string) {
    this.inner = NativeClient.fromUrl(url, token);
  }

  async createCatalog(): Promise<Catalog> {
    try {
      return fromBinary(CatalogSchema, await this.inner.createCatalog());
    } catch (e) { throw parseNativeError(e); }
  }

  async listCatalogs(maxResults: number, pageToken: string): Promise<Catalog[]> {
    try {
      return (await this.inner.listCatalogs(maxResults, pageToken)).map((data) =>
        fromBinary(CatalogSchema, data),
      );
    } catch (e) { throw parseNativeError(e); }
  }

  async *listCatalogsStream(maxResults: number, pageToken: string): AsyncIterable<Catalog> {
    try {
      for await (const data of this.inner.listCatalogsStream(maxResults, pageToken)) {
        yield fromBinary(CatalogSchema, data);
      }
    } catch (e) { throw parseNativeError(e); }
  }

  catalog(catalogName: string): CatalogClient {
    return new CatalogClient(this.inner.catalog(catalogName));
  }

  /**
     * Repeated string query param
     */
  async listByTags(tags: string[], maxResults: number): Promise<ListByTagsResponse> {
    try {
      return fromBinary(ListByTagsResponseSchema, await this.inner.listByTags(tags, maxResults));
    } catch (e) { throw parseNativeError(e); }
  }

  /**
     * Enum query param
     */
  async listByCatalogType(catalogType: number): Promise<ListByTagsResponse> {
    try {
      return fromBinary(ListByTagsResponseSchema, await this.inner.listByCatalogType(catalogType));
    } catch (e) { throw parseNativeError(e); }
  }

  async createSchema(name: string, catalogName: string, schemaType: number): Promise<Schema> {
    try {
      return fromBinary(SchemaSchema, await this.inner.createSchema(name, catalogName, schemaType));
    } catch (e) { throw parseNativeError(e); }
  }

  async listSchemas(catalogName: string, maxResults: number, pageToken: string): Promise<Schema[]> {
    try {
      return (await this.inner.listSchemas(catalogName, maxResults, pageToken)).map((data) =>
        fromBinary(SchemaSchema, data),
      );
    } catch (e) { throw parseNativeError(e); }
  }

  async *listSchemasStream(catalogName: string, maxResults: number, pageToken: string): AsyncIterable<Schema> {
    try {
      for await (const data of this.inner.listSchemasStream(catalogName, maxResults, pageToken)) {
        yield fromBinary(SchemaSchema, data);
      }
    } catch (e) { throw parseNativeError(e); }
  }

  schema(catalogName: string, schemaName: string): SchemaClient {
    return new SchemaClient(this.inner.schema(catalogName, schemaName));
  }

  /**
     * List assignments for an entity. Path params: entity_type, entity_name.
     */
  async listTagAssignments(entityType: string, entityName: string, maxResults: number, pageToken: string): Promise<ListTagAssignmentsResponse> {
    try {
      return fromBinary(ListTagAssignmentsResponseSchema, await this.inner.listTagAssignments(entityType, entityName, maxResults, pageToken));
    } catch (e) { throw parseNativeError(e); }
  }

  /**
     * Create/assign a tag. Path params: entity_type, entity_name; body: tag.
     */
  async createTagAssignment(entityType: string, entityName: string): Promise<TagAssignment> {
    try {
      return fromBinary(TagAssignmentSchema, await this.inner.createTagAssignment(entityType, entityName));
    } catch (e) { throw parseNativeError(e); }
  }

  /**
     * Get a single assignment. Composite key: entity_type, entity_name, tag_key.
     * Carries a gnostic `operation_id` to exercise annotation-driven binding method naming
     * (the binding method should be named `fetch_tag_assignment`, not `get_tag_assignment`).
     */
  async fetchTagAssignment(entityType: string, entityName: string, tagKey: string): Promise<TagAssignment> {
    try {
      return fromBinary(TagAssignmentSchema, await this.inner.fetchTagAssignment(entityType, entityName, tagKey));
    } catch (e) { throw parseNativeError(e); }
  }

  /**
     * Delete a single assignment. Composite key path params.
     */
  async deleteTagAssignment(entityType: string, entityName: string, tagKey: string): Promise<DeleteTagAssignmentResponse> {
    try {
      return fromBinary(DeleteTagAssignmentResponseSchema, await this.inner.deleteTagAssignment(entityType, entityName, tagKey));
    } catch (e) { throw parseNativeError(e); }
  }

  /**
     * Custom POST RPC targeting a composite key that returns `Empty` — exercises
     * the `<()>` / void-return path for a resource-less, path-param'd method.
     */
  async touchTagAssignment(entityType: string, entityName: string, tagKey: string): Promise<void> {
    try {
      await this.inner.touchTagAssignment(entityType, entityName, tagKey);
    } catch (e) { throw parseNativeError(e); }
  }

}
