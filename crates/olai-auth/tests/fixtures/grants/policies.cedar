// Reading a table takes SELECT on it or above, and the use of the schema and catalog it
// is in, each on that container itself.
@id("get-table")
permit (principal, action == Cat::Action::"GetTable", resource is Cat::Table)
when {
  resource.inherited_privileges.contains("SELECT") &&
  resource.schema.privileges.contains("USE_SCHEMA") &&
  resource.catalog.privileges.contains("USE_CATALOG")
};

// The owner of a table may read it.
@id("table-owner")
permit (principal, action == Cat::Action::"GetTable", resource is Cat::Table)
when { resource.privileges.contains("OWNER") };
