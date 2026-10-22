//! The functional topology ([`GraphDto`]) as a [Mermaid](https://mermaid.js.org) flowchart, so
//! the diagram can live in Markdown (a README, docs) wherever Mermaid renders.
//!
//! It is a readable summary of what the editor's canvas draws: one node per component with its
//! implementation and role, shaped by [`NodeKind`]; hosted providers grouped by cloud; and each
//! edge labelled with the one detail that explains it (the port reached, the routed prefixes,
//! the protocol and resources used). Pure and deterministic — the same graph always renders the
//! same text, in the graph's own order.

use std::fmt::Write as _;

use super::topology::{
    CLIENTS_NODE_ID, GraphDto, GraphNodeDto, NodeKind, SurfaceKind, TopologyEdgeDto,
};

/// Render `graph` as a Mermaid `flowchart`, left to right like the canvas.
///
/// Node ids are positional (`n0`, `n1`, …): module ids may contain characters, or be words
/// (`end`), that Mermaid gives a meaning. The component's name is in its label instead.
///
/// The output has no Markdown fence; wrap it in ` ```mermaid ` to embed it.
pub fn graph_mermaid(graph: &GraphDto) -> String {
    let id_of = |module: &str| {
        graph
            .nodes
            .iter()
            .position(|n| n.id == module)
            .map(|i| format!("n{i}"))
    };

    let mut out = String::from("flowchart LR\n");

    // The stack's own nodes first, then each cloud's hosted providers in a subgraph, clouds in
    // order of their first node.
    let mut clouds: Vec<&str> = Vec::new();
    for (i, node) in graph.nodes.iter().enumerate() {
        match node.cloud.as_deref() {
            None => {
                let _ = writeln!(out, "    {}", node_line(i, node));
            }
            Some(cloud) if !clouds.contains(&cloud) => clouds.push(cloud),
            Some(_) => {}
        }
    }
    for (c, cloud) in clouds.iter().enumerate() {
        let title = if cloud.is_empty() { "Hosted" } else { cloud };
        let _ = writeln!(out, "    subgraph cloud{c}[\"{}\"]", escape(title));
        for (i, node) in graph.nodes.iter().enumerate() {
            if node.cloud.as_deref() == Some(cloud) {
                let _ = writeln!(out, "        {}", node_line(i, node));
            }
        }
        out.push_str("    end\n");
    }

    for edge in &graph.edges {
        let (from, to, arrow, label) = edge_parts(graph, edge);
        let (Some(from), Some(to)) = (id_of(from), id_of(to)) else {
            continue;
        };
        if label.is_empty() {
            let _ = writeln!(out, "    {from} {arrow} {to}");
        } else {
            let _ = writeln!(out, "    {from} {arrow}|\"{}\"| {to}", escape(&label));
        }
    }

    out.push_str(
        "    classDef clients fill:#f1f5f9,stroke:#64748b\n    \
         classDef gateway fill:#ede9fe,stroke:#7c3aed\n    \
         classDef component fill:#eff6ff,stroke:#2563eb\n    \
         classDef external fill:#fafafa,stroke:#737373,stroke-dasharray: 4 3\n",
    );
    out
}

/// The node's declaration: positional id, shape for its kind, label, and class.
fn node_line(i: usize, node: &GraphNodeDto) -> String {
    let name = if node.id == CLIENTS_NODE_ID {
        "Clients".to_string()
    } else {
        escape(node.display_name.as_deref().unwrap_or(&node.id))
    };
    let label = match node.role.as_deref() {
        Some(role) if node.kind != NodeKind::Clients => {
            format!("<b>{name}</b><br/>{}", escape(&role.replace('_', " ")))
        }
        _ => format!("<b>{name}</b>"),
    };
    let (open, close, class) = match node.kind {
        NodeKind::Clients => ("([", "])", "clients"),
        NodeKind::Gateway => ("{{", "}}", "gateway"),
        NodeKind::Component => ("[", "]", "component"),
        NodeKind::External => ("[", "]", "external"),
    };
    format!("n{i}{open}\"{label}\"{close}:::{class}")
}

/// An edge's endpoints, Mermaid arrow, and (unescaped) label.
fn edge_parts<'a>(
    graph: &GraphDto,
    edge: &'a TopologyEdgeDto,
) -> (&'a str, &'a str, &'static str, String) {
    match edge {
        TopologyEdgeDto::Access {
            from,
            to,
            host_port,
            ..
        } => (from, to, "-->", format!(":{host_port}")),
        TopologyEdgeDto::Route {
            from,
            to,
            host_port,
            gated,
            ..
        } => {
            // A dedicated listener is named by the protocols it speaks (its prefix is `/`),
            // the shared one by the prefixes routed to this backend.
            let surface = graph
                .nodes
                .iter()
                .find(|n| &n.id == from)
                .into_iter()
                .flat_map(|n| &n.surfaces)
                .find(|s| s.host_port == *host_port);
            let mut label = match surface {
                Some(s) if s.kind == SurfaceKind::Service && !s.protocols.is_empty() => {
                    s.protocols.join(", ")
                }
                Some(s) => {
                    let prefixes: Vec<&str> = s
                        .exposes
                        .iter()
                        .filter(|e| &e.module == to)
                        .map(|e| e.prefix.as_str())
                        .collect();
                    prefixes.join(", ")
                }
                None => String::new(),
            };
            if *gated {
                label.push_str(" 🔒");
            }
            (from, to, "-->", label)
        }
        TopologyEdgeDto::Authenticates { from, to, .. } => (from, to, "-.->", "authn".into()),
        TopologyEdgeDto::Authorizes { from, to, .. } => (from, to, "-.->", "authz".into()),
        TopologyEdgeDto::Uses {
            from,
            to,
            protocol,
            resources,
            ..
        } => {
            let label = if resources.is_empty() {
                protocol.clone()
            } else {
                format!("{protocol}: {}", resources.join(", "))
            };
            (from, to, "-->", label)
        }
    }
}

/// Escape text for a quoted Mermaid label: the quote ends it, and `<`/`>` would be read as
/// HTML. Mermaid's entity syntax (`#quot;`) survives both.
fn escape(text: &str) -> String {
    text.replace('"', "#quot;")
        .replace('<', "#lt;")
        .replace('>', "#gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::topology::{ExposedDto, ExposedKind, SurfaceDto};

    fn node(id: &str, kind: NodeKind) -> GraphNodeDto {
        GraphNodeDto {
            id: id.into(),
            model: None,
            kind,
            display_name: None,
            summary: None,
            category: None,
            role: None,
            placement: None,
            cloud: None,
            surfaces: vec![],
            offers: vec![],
            provisions: vec![],
            resource_nouns: vec![],
        }
    }

    fn sample() -> GraphDto {
        let clients = node(CLIENTS_NODE_ID, NodeKind::Clients);
        let mut gateway = node("envoy", NodeKind::Gateway);
        gateway.display_name = Some("Envoy".into());
        gateway.role = Some("gateway".into());
        gateway.surfaces = vec![SurfaceDto {
            host_port: 8080,
            kind: SurfaceKind::Platform,
            protocols: vec![],
            exposes: ["/api", "/ui"]
                .into_iter()
                .map(|prefix| ExposedDto {
                    module: "end".into(),
                    endpoint: "x".into(),
                    kind: ExposedKind::Api,
                    prefix: prefix.into(),
                    gated: true,
                })
                .collect(),
        }];
        let mut app = node("end", NodeKind::Component);
        app.display_name = Some("The \"<App>\"".into());
        app.role = Some("data_catalog".into());
        let mut idp = node("authelia", NodeKind::Component);
        idp.role = Some("auth".into());
        let mut db = node("hosted-postgres", NodeKind::External);
        db.cloud = Some("aws".into());
        let mut blob = node("hosted-blob", NodeKind::External);
        blob.cloud = Some(String::new());
        GraphDto {
            nodes: vec![clients, gateway, app, idp, db, blob],
            edges: vec![
                TopologyEdgeDto::Access {
                    key: "a".into(),
                    from: CLIENTS_NODE_ID.into(),
                    to: "envoy".into(),
                    host_port: 8080,
                },
                TopologyEdgeDto::Route {
                    key: "r".into(),
                    from: "envoy".into(),
                    to: "end".into(),
                    host_port: 8080,
                    gated: true,
                },
                TopologyEdgeDto::Authenticates {
                    key: "n".into(),
                    from: "envoy".into(),
                    to: "authelia".into(),
                    host_port: 8080,
                    portal_prefix: "/auth".into(),
                },
                TopologyEdgeDto::Authorizes {
                    key: "z".into(),
                    from: "envoy".into(),
                    to: "authelia".into(),
                    host_port: 8080,
                },
                TopologyEdgeDto::Uses {
                    key: "u".into(),
                    from: "end".into(),
                    to: "hosted-postgres".into(),
                    role: "relational_db".into(),
                    protocol: "PostgreSQL".into(),
                    resources: vec!["catalog".into(), "mlflow".into()],
                },
                TopologyEdgeDto::Uses {
                    key: "v".into(),
                    from: "end".into(),
                    to: "hosted-blob".into(),
                    role: "object_store".into(),
                    protocol: "Azure Blob".into(),
                    resources: vec![],
                },
            ],
        }
    }

    #[test]
    fn renders_nodes_groups_and_every_edge_kind() {
        let text = graph_mermaid(&sample());
        let expected = r#"flowchart LR
    n0(["<b>Clients</b>"]):::clients
    n1{{"<b>Envoy</b><br/>gateway"}}:::gateway
    n2["<b>The #quot;#lt;App#gt;#quot;</b><br/>data catalog"]:::component
    n3["<b>authelia</b><br/>auth"]:::component
    subgraph cloud0["aws"]
        n4["<b>hosted-postgres</b>"]:::external
    end
    subgraph cloud1["Hosted"]
        n5["<b>hosted-blob</b>"]:::external
    end
    n0 -->|":8080"| n1
    n1 -->|"/api, /ui 🔒"| n2
    n1 -.->|"authn"| n3
    n1 -.->|"authz"| n3
    n2 -->|"PostgreSQL: catalog, mlflow"| n4
    n2 -->|"Azure Blob"| n5
"#;
        assert!(text.starts_with(expected), "got:\n{text}");
        assert!(text.contains("classDef external"));
    }

    #[test]
    fn labels_a_dedicated_listener_route_by_its_protocols() {
        let mut graph = sample();
        graph.nodes[1].surfaces[0].kind = SurfaceKind::Service;
        graph.nodes[1].surfaces[0].protocols = vec!["S3 + STS".into()];
        let text = graph_mermaid(&graph);
        assert!(text.contains("n1 -->|\"S3 + STS 🔒\"| n2"), "got:\n{text}");
    }

    #[test]
    fn skips_edges_to_unknown_nodes() {
        let mut graph = sample();
        graph.nodes.truncate(2);
        let text = graph_mermaid(&graph);
        assert!(text.contains("n0 -->|\":8080\"| n1"));
        assert!(!text.contains("n2"), "got:\n{text}");
    }

    #[test]
    fn empty_graph_is_a_bare_flowchart() {
        let text = graph_mermaid(&GraphDto {
            nodes: vec![],
            edges: vec![],
        });
        assert!(text.starts_with("flowchart LR\n    classDef"));
    }
}
