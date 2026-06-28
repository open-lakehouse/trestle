//! TypeScript declaration (`.d.ts`) generation for the WASM client.
//!
//! Describes the JS surface that `wasm-bindgen` exposes from [`super::bindings`]: an aggregate
//! class (constructed from a base URL) whose accessors return per-service classes, each carrying
//! `async` methods that take/return plain objects (the serde-JSON representation of the proto
//! request/response messages). Request/response types are referenced by name and imported from
//! `./models`, mirroring the node TypeScript client's `from "./models"` convention.

use std::collections::BTreeSet;

use convert_case::{Case, Casing};

use crate::codegen::ServiceHandler;

/// Generate `client.d.ts`.
pub(crate) fn generate_dts(services: &[ServiceHandler<'_>]) -> String {
    let bindings = services
        .first()
        .and_then(|s| s.config.bindings.as_ref())
        .expect("bindings config required for wasm output");
    let aggregate = &bindings.aggregate_client_name;

    // Collect every request/response model type referenced, so we can import them from `./models`.
    let mut model_types: BTreeSet<String> = BTreeSet::new();
    for service in services {
        for method in service.methods() {
            if let Some(t) = method.input_type() {
                model_types.insert(t.to_string());
            }
            if let Some(t) = method.output_type() {
                model_types.insert(t.to_string());
            }
        }
    }

    let mut out = String::new();
    out.push_str("// @generated — do not edit by hand.\n");
    out.push_str(
        "// TypeScript declarations for the wasm-bindgen browser client.\n\
         // Request/response values are plain objects (the JSON form of the proto messages).\n\n",
    );

    if model_types.is_empty() {
        out.push_str("// (no message types referenced)\n\n");
    } else {
        out.push_str("import type {\n");
        for ty in &model_types {
            out.push_str(&format!("  {ty},\n"));
        }
        out.push_str("} from \"./models\";\n\n");
    }

    // Per-service classes.
    for service in services {
        let class = service
            .low_level_client_type(crate::codegen::ClientProtocol::Rest)
            .to_string();
        out.push_str(&format!(
            "/** WASM/browser binding for the `{}` service. */\n",
            service.plan.base_path
        ));
        out.push_str(&format!("export class {class} {{\n"));
        for method in service.methods() {
            out.push_str(&method_signature(&method));
        }
        out.push_str("}\n\n");
    }

    // Aggregate class.
    out.push_str(
        "/** Browser entry point. Construct from a base URL; the browser manages the session\n\
        \x20  by default, or pass options for bearer-token auth. */\n",
    );
    out.push_str(&format!("export class {aggregate} {{\n"));
    out.push_str(
        "  /**\n\
        \x20  * @param baseUrl Absolute base URL of the API (same-origin for cookie auth).\n\
        \x20  * @param options Optional auth/session settings. Omit to keep the default\n\
        \x20  *   browser-session (cookie) behavior, where `fetch` sends credentials with\n\
        \x20  *   `credentials: \"include\"`.\n\
        \x20  *\n\
        \x20  *   - `authToken` — when set, every request carries\n\
        \x20  *     `Authorization: Bearer <authToken>`. Unless `credentials` is given too,\n\
        \x20  *     this also switches the `fetch` credentials mode to `\"omit\"` so a stale\n\
        \x20  *     cookie can't shadow the header.\n\
        \x20  *   - `credentials` — the `fetch` credentials mode: `\"include\"` (default),\n\
        \x20  *     `\"same-origin\"`, or `\"omit\"`.\n\
        \x20  */\n",
    );
    out.push_str(
        "  constructor(baseUrl: string, options?: { authToken?: string; credentials?: \"include\" | \"same-origin\" | \"omit\" });\n",
    );
    for service in services {
        let accessor = service.plan.base_path.to_case(Case::Camel);
        let class = service
            .low_level_client_type(crate::codegen::ClientProtocol::Rest)
            .to_string();
        out.push_str(&format!(
            "  /** Access the `{}` service. */\n  {accessor}(): {class};\n",
            service.plan.base_path
        ));
    }
    out.push_str("}\n");

    out
}

/// One method signature line: `methodName(request: ReqType): Promise<RespType>;`.
fn method_signature(method: &crate::codegen::MethodHandler<'_>) -> String {
    let name = method.binding_method_name_str().to_case(Case::Camel);
    let param = match method.input_type() {
        Some(ty) => format!("request: {ty}"),
        None => String::new(),
    };
    let ret = match method.output_type() {
        Some(ty) => format!("Promise<{ty}>"),
        None => "Promise<void>".to_string(),
    };
    let mut sig = String::new();
    if let Some(doc) = method.plan.metadata.documentation.as_deref() {
        // Single-line JSDoc; collapse to avoid breaking the declaration.
        let one_line = doc.replace('\n', " ");
        let trimmed = one_line.trim();
        if !trimmed.is_empty() {
            sig.push_str(&format!("  /** {trimmed} */\n"));
        }
    }
    sig.push_str(&format!("  {name}({param}): {ret};\n"));
    sig
}
