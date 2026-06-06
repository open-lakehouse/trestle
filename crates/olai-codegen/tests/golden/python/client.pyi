# @generated — do not edit by hand.
from __future__ import annotations
from typing import Optional, List, Dict, Any, Literal
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

class CatalogToken():
    token: str

    def __init__(self, token: str) -> None:
        ...

class S3Config():
    bucket: str

    def __init__(self, bucket: str) -> None:
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

class ExampleClient():
    def __init__(self, base_url: str, token: Optional[str] = None) -> None:
        ...
    def create_catalog(self, catalog: Optional[Catalog] = None) -> Catalog:
        """
        Returns:
            The requested resource
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