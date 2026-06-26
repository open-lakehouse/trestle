//! Fill unset config fields from the project root name.
//!
//! [`ProjectMeta::name`](super::ProjectMeta::name) is the single source of truth
//! for the project's naming; the various crate / binding identifiers default to
//! case-conversions of it. Derivation only fills fields left unset — any value
//! explicitly present in the file survives untouched.

use convert_case::{Case, Casing};

use super::{Bindings, GenerateConfig, TrestleConfig};

impl TrestleConfig {
    /// Fill unset derived fields (`models.crate_name`, `bindings.*`) from
    /// [`ProjectMeta::name`](super::ProjectMeta::name).
    pub fn derive_defaults(&mut self) {
        let name = self.project.name.clone();
        self.generate.derive_from_name(&name);
    }
}

impl GenerateConfig {
    fn derive_from_name(&mut self, name: &str) {
        let snake = name.to_case(Case::Snake);
        let kebab = name.to_case(Case::Kebab);
        let pascal = name.to_case(Case::Pascal);
        let screaming = name.to_case(Case::Constant);

        self.models
            .crate_name
            .get_or_insert_with(|| format!("{snake}_common"));

        // path_template depends on the (now-derived) crate name.
        if self.models.path_template.is_none() {
            if let Some(crate_name) = &self.models.crate_name {
                self.models.path_template =
                    Some(format!("{crate_name}::models::{{service}}::{{version}}"));
            }
        }
        self.models
            .path_crate_template
            .get_or_insert_with(|| "crate::models::{service}::{version}".to_string());

        // Binding identity is only meaningful when a JS/TS client is emitted, but
        // deriving unconditionally is harmless and keeps the written file complete.
        let b = self.bindings.get_or_insert_with(Bindings::default);
        b.aggregate_client_name
            .get_or_insert_with(|| format!("{pascal}Client"));
        b.client_crate_name
            .get_or_insert_with(|| format!("{kebab}-client"));
        b.error_base_class
            .get_or_insert_with(|| "ApiError".to_string());
        b.error_code_prefix
            .get_or_insert_with(|| format!("{screaming}_"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Clients, Models, ProjectMeta, Server, Servers};

    fn cfg(name: &str) -> TrestleConfig {
        TrestleConfig {
            version: 1,
            project: ProjectMeta {
                name: name.to_string(),
                id: None,
                description: None,
            },
            generate: GenerateConfig {
                proto_lib: Default::default(),
                descriptors: "api.bin".into(),
                servers: Servers::default(),
                clients: Clients::default(),
                bindings: None,
                models: Models {
                    common_output: "x".into(),
                    parent_output: None,
                    subdir: "_gen".into(),
                    crate_name: None,
                    path_template: None,
                    path_crate_template: None,
                },
                server: Server::default(),
            },
            enrich_openapi: None,
        }
    }

    #[test]
    fn derives_all_four_identifiers() {
        let mut c = cfg("golden-path-app");
        c.derive_defaults();
        assert_eq!(
            c.generate.models.crate_name.as_deref(),
            Some("golden_path_app_common")
        );
        let b = c.generate.bindings.unwrap();
        assert_eq!(
            b.aggregate_client_name.as_deref(),
            Some("GoldenPathAppClient")
        );
        assert_eq!(
            b.client_crate_name.as_deref(),
            Some("golden-path-app-client")
        );
        assert_eq!(b.error_code_prefix.as_deref(), Some("GOLDEN_PATH_APP_"));
    }

    #[test]
    fn explicit_overrides_survive() {
        let mut c = cfg("golden-path-app");
        c.generate.models.crate_name = Some("custom_models".into());
        c.derive_defaults();
        assert_eq!(
            c.generate.models.crate_name.as_deref(),
            Some("custom_models")
        );
        // path_template derived from the explicit crate name.
        assert_eq!(
            c.generate.models.path_template.as_deref(),
            Some("custom_models::models::{service}::{version}")
        );
    }
}
