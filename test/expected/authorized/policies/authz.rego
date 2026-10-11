# Gateway authorization policy — evaluated by OPA for every request on the gateway's shared
# listener, *after* Authelia has authenticated it. Written next to `compose.yaml` on first
# render and never overwritten: this file is yours to edit. Delete it and re-render to restore
# the default.
#
# Input is Envoy's ext_authz CheckRequest; Authelia's asserted identity arrives as the
# `remote-user` / `remote-groups` request headers (clients cannot forge them: the gateway
# strips them on ingress). Check your changes with `opa test policies/`.
package envoy.authz

import rego.v1

default allow := false

request := input.attributes.request.http

user := request.headers["remote-user"]

groups := {trim_space(g) | some g in split(object.get(request.headers, "remote-groups", ""), ",")} - {""}

read_methods := {"GET", "HEAD", "OPTIONS"}

# Any authenticated user may read.
allow if {
	user != ""
	request.method in read_methods
}

# Writes need the `writers` or `admins` group (see `users.yml`).
allow if {
	user != ""
	not request.method in read_methods
	some g in {"writers", "admins"}
	g in groups
}
