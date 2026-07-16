// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::metadata_table::MetadataTable;
use crate::signature::{InterfaceSignature, MethodSignature};
use std::sync::Arc;

pub fn uri_vtable(reg: &Arc<MetadataTable>) -> InterfaceSignature {
    let mut vtable = InterfaceSignature::define_from_iinspectable(
        "Windows.Foundation.IUriRuntimeClass",
        Default::default(),
        reg,
    );
    vtable
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 6 get_AbsoluteUri
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 7 get_DisplayUri
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 8 get_Domain
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 9 get_Extension
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 10 get_Fragment
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 11 get_Host
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 12 get_Password
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 13 get_Path
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 14 get_Query
        .add_method(MethodSignature::new(reg)) // 15 get_QueryParsed
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 16 get_RawUri
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 17 get_SchemeName
        .add_method(MethodSignature::new(reg).add_out(reg.hstring())) // 18 get_UserName
        .add_method(MethodSignature::new(reg).add_out(reg.i32_type())) // 19 get_Port
        .add_method(MethodSignature::new(reg)); // 20 get_Suspicious;
    vtable
}
