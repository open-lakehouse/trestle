# Tests for the gateway authorization policy: `opa test policies/`.
package envoy.authz_test

import rego.v1

import data.envoy.authz

req(method, headers) := {"attributes": {"request": {"http": {
	"method": method,
	"path": "/api/2.1/unity-catalog/catalogs",
	"headers": headers,
}}}}

test_anonymous_is_denied if {
	not authz.allow with input as req("GET", {})
}

test_authenticated_user_can_read if {
	authz.allow with input as req("GET", {"remote-user": "alice"})
}

test_reader_cannot_write if {
	not authz.allow with input as req("POST", {"remote-user": "alice", "remote-groups": "readers"})
}

test_writer_can_write if {
	authz.allow with input as req("POST", {"remote-user": "bob", "remote-groups": "readers, writers"})
}

test_admin_can_write if {
	authz.allow with input as req("DELETE", {"remote-user": "demo", "remote-groups": "admins"})
}
