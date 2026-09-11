// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ReturnKind {
    U32,
    U64,
    Status,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub(super) enum Argument {
    U32,
    Utf16 { nullable: bool },
    BorrowHkey { reject_performance_data: bool },
    OwnHkey { borrowed_from: usize },
    ConsumeHkey,
    ReservedNull,
    OutU32,
    Bytes { count_parameter: usize },
    ByteCount { buffer_parameter: usize },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Plan {
    pub version: u32,
    pub dll: String,
    pub entry_point: String,
    pub calling_convention: &'static str,
    pub architectures: u32,
    pub returns: ReturnKind,
    pub parameters: Vec<Argument>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum InputType {
    U32,
    Utf16 { nullable: bool },
    Hkey,
    Resource,
    OptionalBytes,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum OutputType {
    U32,
    Hkey,
    Bytes,
}

#[derive(Clone, Debug)]
pub(super) struct Input {
    pub name: String,
    pub typ: InputType,
}

#[derive(Clone, Debug)]
pub(super) struct Output {
    pub name: String,
    pub typ: OutputType,
}

#[derive(Clone, Debug)]
pub(super) struct Function {
    pub name: String,
    pub plan: Plan,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
}

pub(super) struct File {
    pub safe_import: String,
    pub unsafe_import: String,
    pub functions: Vec<Function>,
}
