// @generated — do not edit by hand.
//! [`Authorizable`] for the requests of this service.
#![allow(unused_imports)]

use example_common::models::ObjectLabel;
use crate::codegen::authz::{Authorizable, Operation, Securable};
use example_common::models::authz::v1::*;

impl Authorizable for ListCatalogsRequest {
    fn operation(&self) -> Operation {
        Operation::ListCatalogs
    }

    fn securable(&self) -> Securable {
        Securable {
            label: None,
            name: Vec::new(),
        }
    }
}

impl Authorizable for CreateCatalogRequest {
    fn operation(&self) -> Operation {
        Operation::CreateCatalog
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.name.split('.').map(str::to_string));
        Securable {
            label: Some(ObjectLabel::Catalog),
            name,
        }
    }
}

impl Authorizable for GetCatalogRequest {
    fn operation(&self) -> Operation {
        Operation::GetCatalog
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.name.split('.').map(str::to_string));
        Securable {
            label: Some(ObjectLabel::Catalog),
            name,
        }
    }
}
