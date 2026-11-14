// @generated — do not edit by hand.
//! [`Authorizable`] for the requests of this service.
#![allow(unused_imports)]

use example_common::models::ObjectLabel;
use crate::codegen::authz::{Authorizable, Operation, Securable};
use example_common::models::authz::v1::*;

impl Authorizable for ListTablesRequest {
    fn operation(&self) -> Operation {
        Operation::ListTables
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.catalog_name.split('.').map(str::to_string));
        name.extend(self.schema_name.split('.').map(str::to_string));
        Securable {
            label: Some(ObjectLabel::Schema),
            name,
        }
    }
}

impl Authorizable for ListTableSummariesRequest {
    fn operation(&self) -> Operation {
        Operation::ListTableSummaries
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

impl Authorizable for CreateTableRequest {
    fn operation(&self) -> Operation {
        Operation::CreateTable
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.table_info.catalog_name.split('.').map(str::to_string));
        name.extend(self.table_info.schema_name.split('.').map(str::to_string));
        name.extend(self.table_info.name.split('.').map(str::to_string));
        Securable {
            label: Some(ObjectLabel::Table),
            name,
        }
    }
}

impl Authorizable for GetTableRequest {
    fn operation(&self) -> Operation {
        Operation::GetTable
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.full_name.split('.').map(str::to_string));
        Securable {
            label: Some(ObjectLabel::Table),
            name,
        }
    }
}

impl Authorizable for GetTableVersionRequest {
    fn operation(&self) -> Operation {
        Operation::GetTableVersion
    }

    fn securable(&self) -> Securable {
        let mut name = Vec::new();
        name.extend(self.full_name.split('.').map(str::to_string));
        name.push(self.version.to_string());
        Securable {
            label: Some(ObjectLabel::Table),
            name,
        }
    }
}
