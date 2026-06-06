# @generated — do not edit by hand.
from __future__ import annotations
from typing import Optional, List, Dict
import enum

class AzureConfig():
    container: str

    def __init__(self, container: str) -> None:
        ...

class Catalog():
    catalog_type: CatalogType
    comment: str
    created_at: int
    name: str
    properties: Dict[str, str]
    storage_config: StorageConfig

    def __init__(

            self,
            catalog_type: CatalogType,
            comment: str,
            created_at: int,
            name: str,
            properties: Dict[str, str],
            storage_config: StorageConfig
        ) -> None:
        ...

class CatalogStatus():
    state: str

    def __init__(self, state: str) -> None:
        ...

class CatalogToken():
    token: str

    def __init__(self, token: str) -> None:
        ...

class DeleteTagAssignmentResponse():
    ...

class ListByTagsResponse():
    results: List[str]

    def __init__(self, results: Optional[List[str]] = None) -> None:
        ...

class ListTagAssignmentsResponse():
    next_page_token: str
    tag_assignments: List[TagAssignment]

    def __init__(self, next_page_token: str, tag_assignments: Optional[List[TagAssignment]] = None) -> None:
        ...

class S3Config():
    bucket: str

    def __init__(self, bucket: str) -> None:
        ...

class Schema():
    """A Schema is a child resource of a Catalog. This fixture exercises the generated nested resource-
scoped client: `catalog.schema(name)` navigation, `catalog.create_schema(...)`, and the cross-module
model import for the create method's enum parameter (`SchemaType`)."""
    comment: str
    created_at: int
    full_name: str
    """Dot-joined `catalog.schema` full name."""
    schema_type: SchemaType

    def __init__(

            self,
            comment: str,
            created_at: int,
            full_name: str,
            schema_type: SchemaType
        ) -> None:
        ...

class StorageConfig():
    azure: Optional[AzureConfig]
    s3: Optional[S3Config]

    def __init__(self, azure: Optional[AzureConfig] = None, s3: Optional[S3Config] = None) -> None:
        ...

class TagAssignment():
    entity_name: str
    entity_type: str
    tag_key: str
    tag_value: str

    def __init__(

            self,
            entity_name: str,
            entity_type: str,
            tag_key: str,
            tag_value: str
        ) -> None:
        ...

class CatalogType(enum.Enum):
    CATALOG_TYPE_UNSPECIFIED = "CATALOG_TYPE_UNSPECIFIED"
    DELTASHARING_CATALOG = "DELTASHARING_CATALOG"
    MANAGED_CATALOG = "MANAGED_CATALOG"

class GetSchemaRequestView(enum.Enum):
    """A *nested* enum (declared inside the request message). Exercises nested-enum parsing and the parent-
qualified naming in the Python typings (`GetSchemaRequestView`), which must not collide with any
other nested enum of the same simple name."""
    BASIC = "BASIC"
    FULL = "FULL"
    VIEW_UNSPECIFIED = "VIEW_UNSPECIFIED"

class SchemaType(enum.Enum):
    EXTERNAL_SCHEMA = "EXTERNAL_SCHEMA"
    MANAGED_SCHEMA = "MANAGED_SCHEMA"
    SCHEMA_TYPE_UNSPECIFIED = "SCHEMA_TYPE_UNSPECIFIED"

class CatalogClient():
    def delete(self) -> None:
        """
        Returns:
            None
        """
        ...
    def get(self) -> Catalog:
        """
        Returns:
            The requested resource
        """
        ...
    def update(self, catalog: Optional[Catalog] = None) -> Catalog:
        """
        Returns:
            The requested resource
        """
        ...
    def schema(self, catalog_name: str, schema_name: str) -> SchemaClient:
        ...

class SchemaClient():
    def delete(self) -> None:
        """
        Returns:
            None
        """
        ...
    def get(self, view: GetSchemaRequestView) -> Schema:
        """
        Returns:
            A Schema is a child resource of a Catalog. This fixture exercises the generated
            nested resource-
            scoped client: `catalog.schema(name)` navigation, `catalog.create_schema(...)`, and
            the cross-
            module model import for the create method's enum parameter (`SchemaType`).
        """
        ...
    def update(self, schema: Optional[Schema] = None) -> Schema:
        """
        Returns:
            A Schema is a child resource of a Catalog. This fixture exercises the generated
            nested resource-
            scoped client: `catalog.schema(name)` navigation, `catalog.create_schema(...)`, and
            the cross-
            module model import for the create method's enum parameter (`SchemaType`).
        """
        ...

class ExampleClient():
    def __init__(self, base_url: str, token: Optional[str] = None) -> None:
        ...
    def create_catalog(self, catalog: Optional[Catalog] = None) -> Catalog:
        """
        Returns:
            The requested resource
        """
        ...
    def create_schema(

            self,
            name: str,
            catalog_name: str,
            schema_type: SchemaType
        ) -> Schema:
        """
        Args:
            name: Schema's own name (the new component supplied by the caller).
            catalog_name: Parent catalog name — filled from the parent `CatalogClient`'s captured component.
            schema_type: Required enum parameter whose type lives in this (schemas) models module —
                         exercises the child-model import on the parent's generated `create_schema` method.


        Returns:
            A Schema is a child resource of a Catalog. This fixture exercises the generated
            nested resource-
            scoped client: `catalog.schema(name)` navigation, `catalog.create_schema(...)`, and
            the cross-
            module model import for the create method's enum parameter (`SchemaType`).
        """
        ...
    def create_tag_assignment(

            self,
            entity_type: str,
            entity_name: str,
            tag: Optional[TagAssignment] = None
        ) -> TagAssignment:
        """
        Create/assign a tag. Path params: entity_type, entity_name; body: tag.


        Returns:
            The requested resource
        """
        ...
    def delete_tag_assignment(

            self,
            entity_type: str,
            entity_name: str,
            tag_key: str
        ) -> DeleteTagAssignmentResponse:
        """
        Delete a single assignment. Composite key path params.


        Returns:
            The requested resource
        """
        ...
    def fetch_tag_assignment(

            self,
            entity_type: str,
            entity_name: str,
            tag_key: str
        ) -> TagAssignment:
        """
        Get a single assignment. Composite key: entity_type, entity_name, tag_key. Carries a gnostic
        `operation_id` to exercise annotation-driven binding method naming (the binding method should be
        named `fetch_tag_assignment`, not `get_tag_assignment`).


        Returns:
            The requested resource
        """
        ...
    def generate_catalog_token(self, catalog_id: str) -> CatalogToken:
        """
        Custom POST RPC without path params — covers `RequestType::Custom(Post)` dispatched as a collection
        method (the shape used by factory-style RPCs like `GenerateTemporary*Credentials`).


        Returns:
            The requested resource
        """
        ...
    def list_by_catalog_type(self, catalog_type: CatalogType) -> ListByTagsResponse:
        """
        Enum query param


        Args:
            catalog_type: enum as query param


        Returns:
            The requested resource
        """
        ...
    def list_by_tags(self, tags: List[str], max_results: int) -> ListByTagsResponse:
        """
        Repeated string query param


        Args:
            tags: becomes repeated query param


        Returns:
            The requested resource
        """
        ...
    def list_catalogs(self, max_results: int, page_token: str) -> List[Catalog]:
        """
        Returns:
            List of items
        """
        ...
    def list_schemas(

            self,
            catalog_name: str,
            max_results: int,
            page_token: str
        ) -> List[Schema]:
        """
        Args:
            catalog_name: Parent scoping field carrying the child-type reference that makes Schema a child
                          of Catalog.


        Returns:
            List of items
        """
        ...
    def list_tag_assignments(

            self,
            entity_type: str,
            entity_name: str,
            max_results: int,
            page_token: str
        ) -> ListTagAssignmentsResponse:
        """
        List assignments for an entity. Path params: entity_type, entity_name.


        Returns:
            The requested resource
        """
        ...
    def touch_tag_assignment(

            self,
            entity_type: str,
            entity_name: str,
            tag_key: str
        ) -> None:
        """
        Custom POST RPC targeting a composite key that returns `Empty` — exercises the `<()>` / void-return
        path for a resource-less, path-param'd method.


        Returns:
            The requested resource
        """
        ...
    def catalog(self, catalog_name: str) -> CatalogClient:
        ...
    def schema(self, catalog_name: str, schema_name: str) -> SchemaClient:
        ...