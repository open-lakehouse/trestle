@id("admin-permit")
permit (principal in Docs::Role::"admin", action, resource);

@id("banned-forbid")
forbid (principal is Docs::User, action, resource)
when { principal.banned };

@id("public-read")
permit (principal, action == Docs::Action::"read", resource is Docs::Doc)
when { resource.public };

@id("owner-edit")
permit (
  principal is Docs::User,
  action in [Docs::Action::"read", Docs::Action::"write"],
  resource is Docs::Doc
)
when { resource.owner == principal };

@id("editors-write")
permit (
  principal in Docs::Group::"editors",
  action == Docs::Action::"write",
  resource in Docs::Folder::"shared"
);

@id("ingest-scope")
permit (principal is Docs::ServiceAccount, action == Docs::Action::"ingest", resource)
when { context.scopes.contains("ingest") };

@id("writes-from-office")
forbid (principal, action == Docs::Action::"write", resource)
when { context has source_ip && !context.source_ip.isInRange(ip("10.0.0.0/8")) };

@id("no-writes-after-2030")
forbid (principal, action == Docs::Action::"write", resource)
when { context has request_time && context.request_time >= datetime("2030-01-01") };
