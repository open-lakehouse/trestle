// @generated — do not edit by hand.
//! [`Authorizable`] for the requests of this service.
#![allow(unused_imports)]

use example_common::models::ObjectLabel;
use crate::codegen::authz::{Authorizable, Operation, Securable};
use example_common::models::authz::v1::*;

impl Authorizable for ListSchemasRequest {
    fn operation(&self) -> Operation {
        Operation::ListSchemas
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.catalog_name.split('.').map(str::to_string));
        Securable {
            label: Some(ObjectLabel::Catalog),
            name,
        }
    }
}

impl Authorizable for CreateSchemaRequest {
    fn operation(&self) -> Operation {
        Operation::CreateSchema
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.catalog_name.split('.').map(str::to_string));
        name.extend(self.name.split('.').map(str::to_string));
        Securable {
            label: Some(ObjectLabel::Schema),
            name,
        }
    }
}

impl Authorizable for GetSchemaRequest {
    fn operation(&self) -> Operation {
        Operation::GetSchema
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.full_name.split('.').map(str::to_string));
        Securable {
            label: Some(ObjectLabel::Schema),
            name,
        }
    }
}
