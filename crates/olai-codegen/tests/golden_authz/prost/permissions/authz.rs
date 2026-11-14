// @generated — do not edit by hand.
//! [`Authorizable`] for the requests of this service.
#![allow(unused_imports)]

use example_common::models::ObjectLabel;
use crate::codegen::authz::{Authorizable, Operation, Securable};
use example_common::models::authz::v1::*;

impl Authorizable for GetTablePermissionsRequest {
    fn operation(&self) -> Operation {
        Operation::GetTablePermissions
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.table_name.split('.').map(str::to_string));
        Securable {
            label: Some(ObjectLabel::Table),
            name,
        }
    }
}

impl Authorizable for CheckTableRequest {
    fn operation(&self) -> Operation {
        Operation::CheckTable
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        if let Some(value) = &self.table_name {
            name.extend(value.split('.').map(str::to_string));
        }
        Securable {
            label: Some(ObjectLabel::Table),
            name,
        }
    }
}
