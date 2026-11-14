// @generated — do not edit by hand.
//! [`Authorizable`] for the requests of this service.
#![allow(unused_imports)]

use example_common::models::ObjectLabel;
use crate::codegen::authz::{Authorizable, Operation, Securable};
use example_common::models::authz::v1::*;

impl Authorizable for CreateDraftRequest {
    fn operation(&self) -> Operation {
        Operation::CreateDraft
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.catalog_name.split('.').map(str::to_string));
        name.extend(self.name.split('.').map(str::to_string));
        Securable {
            label: Some(ObjectLabel::Draft),
            name,
        }
    }
}

impl Authorizable for GetDraftRequest {
    fn operation(&self) -> Operation {
        Operation::GetDraft
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.name.split('.').map(str::to_string));
        Securable {
            label: Some(ObjectLabel::Draft),
            name,
        }
    }
}
