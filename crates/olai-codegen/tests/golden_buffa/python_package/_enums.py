# @generated — do not edit by hand.
"""The enums of the API.

Each member is a `str` equal to its name, so a member and its name compare equal, and a
parameter takes either.
"""

import enum


class CatalogType(enum.StrEnum):
    MANAGED_CATALOG = "MANAGED_CATALOG"
    DELTASHARING_CATALOG = "DELTASHARING_CATALOG"


class GetSchemaRequestView(enum.StrEnum):
    """A *nested* enum (declared inside the request message). Exercises nested-enum parsing and the
    parent-qualified naming in the Python typings (`GetSchemaRequestView`), which must not collide
    with any other nested enum of the same simple name.
    """

    BASIC = "BASIC"
    FULL = "FULL"


class SchemaType(enum.StrEnum):
    MANAGED_SCHEMA = "MANAGED_SCHEMA"
    EXTERNAL_SCHEMA = "EXTERNAL_SCHEMA"
