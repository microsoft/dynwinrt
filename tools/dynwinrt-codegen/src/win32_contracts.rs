// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Independently pinned Win32 semantic evidence. Generic ABI facts do not
//! require function allowlisting; exact contracts only supply missing semantics.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};
use windows_metadata::{AsRow, HasAttributes, reader};

use crate::codegen::win32::ir::{
    AsyncIoKind, CallContract, Cleanup, InputPredicate, OutputAction, ResourceEffect, Subsystem,
};
use crate::win32_metadata::{RawFunction, RawScalar};

pub(crate) const METADATA_SHA256: &str =
    "B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D";

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:X}", Sha256::digest(bytes))
}

fn text_sha256(text: &str) -> String {
    sha256(text.replace("\r\n", "\n").as_bytes())
}

fn string_enum<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let text = String::deserialize(deserializer)?;
    T::deserialize(serde::de::value::StringDeserializer::<D::Error>::new(text))
}

fn optional_string_enum<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|text| T::deserialize(serde::de::value::StringDeserializer::<D::Error>::new(text)))
        .transpose()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FunctionSelector {
    pub namespace: String,
    pub container: String,
    pub name: String,
    pub dll: String,
    pub entry_point: String,
    pub calling_convention: String,
    pub architectures: u8,
    pub source_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TypeSelector {
    pub namespace: String,
    pub name: String,
    pub source_fingerprint: String,
}

fn bytes(mut blob: reader::Blob<'_>) -> Vec<u8> {
    let mut result = Vec::new();
    while !blob.is_empty() {
        result.push(blob.read_u8());
    }
    result
}

fn attributes<'a>(item: &impl HasAttributes<'a>) -> Vec<(String, Vec<u8>)> {
    let mut result = item
        .attributes()
        .map(|attribute| {
            let constructor = attribute.ctor();
            let parent = constructor.parent();
            (
                format!("{}.{}", parent.namespace(), parent.name()),
                bytes(attribute.blob(2)),
            )
        })
        .collect::<Vec<_>>();
    result.sort();
    result
}

pub(crate) fn method_fingerprint(
    namespace: &str,
    container: &str,
    method: &reader::MethodDef,
) -> String {
    let import = method
        .impl_map()
        .expect("only flat imports have Win32 evidence");
    let parameters = method
        .params()
        .map(|p| {
            (
                p.sequence(),
                p.name().to_owned(),
                format!("{:?}", p.flags()),
                attributes(&p),
            )
        })
        .collect::<Vec<_>>();
    sha256(
        &serde_json::to_vec(&(
            namespace,
            container,
            method.name(),
            import.import_scope().name(),
            import.import_name(),
            format!("{:?}", import.flags()),
            format!("{:?}", method.flags()),
            format!("{:?}", method.impl_flags()),
            bytes(method.blob(4)),
            attributes(method),
            parameters,
        ))
        .expect("raw metadata facts serialize"),
    )
}

pub(crate) fn type_fingerprint(index: &reader::Index, namespace: &str, name: &str) -> String {
    let definitions = index
        .get(namespace, name)
        .map(|definition| {
            let fields = definition
                .fields()
                .map(|field| {
                    (
                        field.name().to_owned(),
                        format!("{:?}", field.flags()),
                        bytes(field.blob(2)),
                        field.constant().map(|value| format!("{:?}", value.value())),
                        attributes(&field),
                    )
                })
                .collect::<Vec<_>>();
            (
                definition.namespace().to_owned(),
                definition.name().to_owned(),
                format!("{:?}", definition.flags()),
                definition
                    .extends()
                    .map(|base| (base.namespace().to_string(), base.name().to_string())),
                definition
                    .class_layout()
                    .map(|layout| (layout.packing_size(), layout.class_size())),
                attributes(&definition),
                fields,
            )
        })
        .collect::<Vec<_>>();
    sha256(&serde_json::to_vec(&definitions).expect("raw type metadata serializes"))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum FunctionEffect {
    CountedBuffer {
        parameter: usize,
        count_parameter: usize,
        nullable: bool,
    },
    UnsupportedCountUnit {
        parameter: usize,
    },
    OwnedReturn {
        #[serde(deserialize_with = "string_enum")]
        cleanup: Cleanup,
    },
    BorrowedReturn {},
    OwnedOutput {
        parameter: usize,
        #[serde(deserialize_with = "string_enum")]
        cleanup: Cleanup,
    },
    ConsumedInput {
        parameter: usize,
        #[serde(deserialize_with = "string_enum")]
        cleanup: Cleanup,
    },
    MutableString {
        parameter: usize,
    },
    CallContract {
        contract: CallContract,
    },
    OverlappedIo {
        #[serde(deserialize_with = "string_enum")]
        operation: AsyncIoKind,
        file_parameter: usize,
        buffer_parameter: usize,
        count_parameter: usize,
        transferred_parameter: usize,
        overlapped_parameter: usize,
    },
    Subsystem {
        #[serde(deserialize_with = "string_enum")]
        subsystem: Subsystem,
    },
    SubsystemExempt {},
    ManagedLifecycle {
        #[serde(deserialize_with = "string_enum")]
        subsystem: Subsystem,
    },
}

impl FunctionEffect {
    fn key(&self) -> String {
        match self {
            Self::CountedBuffer { parameter, .. } | Self::UnsupportedCountUnit { parameter } => {
                format!("buffer:{parameter}")
            }
            Self::OwnedReturn { .. } | Self::BorrowedReturn {} => "return".into(),
            Self::OwnedOutput { parameter, .. } | Self::ConsumedInput { parameter, .. } => {
                format!("ownership:{parameter}")
            }
            Self::MutableString { parameter } => format!("mutable-string:{parameter}"),
            Self::CallContract { .. } => "call-contract".into(),
            Self::OverlappedIo { .. } => "async".into(),
            Self::Subsystem { .. } | Self::SubsystemExempt {} | Self::ManagedLifecycle { .. } => {
                "subsystem".into()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Encoding {
    Utf16,
    Ansi,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum TypeMeaning {
    Scalar {
        #[serde(deserialize_with = "string_enum")]
        scalar: RawScalar,
    },
    Handle {},
    DataPointer {},
    StringPointer {
        #[serde(deserialize_with = "string_enum")]
        encoding: Encoding,
        is_const: bool,
    },
    FunctionPointer {},
    Unsupported {},
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StatusMeaning {
    Zero,
    SignedNonnegative,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BuilderKind {
    SecurityAttributes,
    StartupInfoAnsi,
    StartupInfoWide,
    ProcessInformation,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum FieldContract {
    RetainedPointer {
        nullable: bool,
        optional: bool,
    },
    NullPointer {},
    BorrowedHandle {},
    OwnedHandle {
        #[serde(deserialize_with = "string_enum")]
        cleanup: Cleanup,
    },
    BooleanInput {
        optional: bool,
    },
    OutputU32 {},
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FieldPolicy {
    pub name: String,
    pub contract: FieldContract,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RecordKind {
    Struct,
    Union,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum RecipeType {
    Scalar {
        #[serde(deserialize_with = "string_enum")]
        scalar: RawScalar,
    },
    Named {
        namespace: String,
        name: String,
    },
    Record {
        layout: Box<LayoutRecipe>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecipeField {
    pub name: String,
    pub typ: RecipeType,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LayoutRecipe {
    pub name: String,
    #[serde(deserialize_with = "string_enum")]
    pub kind: RecordKind,
    pub fields: Vec<RecipeField>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AggregateContract {
    #[serde(deserialize_with = "optional_string_enum")]
    pub builder: Option<BuilderKind>,
    pub size_field: Option<String>,
    pub fields: Vec<FieldPolicy>,
    pub anonymous_layouts: Vec<LayoutRecipe>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum Citation {
    MicrosoftLearn { url: String },
    SdkHeader { file: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FunctionEntry {
    pub id: String,
    pub selector: FunctionSelector,
    pub contracts: Vec<FunctionEffect>,
    evidence: Vec<Citation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TypeEntry {
    pub id: String,
    pub selector: TypeSelector,
    pub meaning: Option<TypeMeaning>,
    #[serde(deserialize_with = "optional_string_enum")]
    pub status: Option<StatusMeaning>,
    pub aggregate: Option<AggregateContract>,
    evidence: Vec<Citation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum DomainPolicy {
    Subsystem {
        namespace: String,
        #[serde(deserialize_with = "string_enum")]
        subsystem: Subsystem,
    },
    ProviderLifecycle {
        dll: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Group<T> {
    schema_version: u32,
    entries: Vec<T>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePin {
    file: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataPin {
    package: String,
    version: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    metadata: MetadataPin,
    schema: FilePin,
    files: Vec<FilePin>,
}

const MANIFEST: &str = include_str!("../contracts/win32/manifest.json");
const SCHEMA: &str = include_str!("../contracts/win32/schema.json");
const FILES: &[(&str, &str)] = &[
    (
        "function-contracts.json",
        include_str!("../contracts/win32/function-contracts.json"),
    ),
    (
        "subsystem-policies.json",
        include_str!("../contracts/win32/subsystem-policies.json"),
    ),
    (
        "native-types.json",
        include_str!("../contracts/win32/native-types.json"),
    ),
    (
        "aggregate-layouts.json",
        include_str!("../contracts/win32/aggregate-layouts.json"),
    ),
    (
        "policy-domains.json",
        include_str!("../contracts/win32/policy-domains.json"),
    ),
];

pub(crate) struct Registry {
    functions: BTreeMap<(String, String, String), FunctionEntry>,
    types: BTreeMap<(String, String), TypeEntry>,
    domains: Vec<DomainPolicy>,
}

pub(crate) struct FunctionPolicy<'a> {
    pub entry: Option<&'a FunctionEntry>,
    pub subsystem: Option<Subsystem>,
}

impl FunctionPolicy<'_> {
    pub fn effects(&self) -> impl Iterator<Item = &FunctionEffect> {
        self.entry
            .into_iter()
            .flat_map(|entry| entry.contracts.iter())
    }

    pub fn owned_return(&self) -> Option<Cleanup> {
        self.effects().find_map(|effect| match effect {
            FunctionEffect::OwnedReturn { cleanup } => Some(*cleanup),
            _ => None,
        })
    }

    pub fn borrowed_return(&self) -> bool {
        self.effects()
            .any(|effect| matches!(effect, FunctionEffect::BorrowedReturn {}))
    }

    pub fn output_cleanup(&self, index: usize) -> Option<Cleanup> {
        self.effects().find_map(|effect| match effect {
            FunctionEffect::OwnedOutput { parameter, cleanup } if *parameter == index => {
                Some(*cleanup)
            }
            _ => None,
        })
    }

    pub fn consumed_input(&self, index: usize) -> Option<Cleanup> {
        self.effects().find_map(|effect| match effect {
            FunctionEffect::ConsumedInput { parameter, cleanup } if *parameter == index => {
                Some(*cleanup)
            }
            _ => None,
        })
    }

    pub fn mutable_string(&self, index: usize) -> bool {
        self.effects().any(|effect|matches!(effect,FunctionEffect::MutableString { parameter } if *parameter == index))
    }

    pub fn call_contract(&self) -> CallContract {
        self.effects()
            .find_map(|effect| match effect {
                FunctionEffect::CallContract { contract } => Some(contract.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn apply_buffers(&self, raw: &RawFunction) -> Result<RawFunction, String> {
        use crate::win32_metadata::{RawBuffer, RawBufferSize, RawDirection, buffer_element};
        let mut result = raw.clone();
        for effect in self.effects() {
            match effect {
                FunctionEffect::CountedBuffer {
                    parameter,
                    count_parameter,
                    nullable,
                } => {
                    if parameter == count_parameter || *count_parameter >= result.parameters.len() {
                        return Err("win32.contract-conflict: invalid count parameter".into());
                    }
                    let buffer = result
                        .parameters
                        .get_mut(*parameter)
                        .ok_or("win32.contract-conflict: missing buffer parameter")?;
                    if buffer.direction != RawDirection::Out {
                        return Err(
                            "win32.contract-conflict: counted output is not writable".into()
                        );
                    }
                    if let Some(existing) = &buffer.buffer
                        && existing.size != RawBufferSize::ElementCountParam(*count_parameter)
                    {
                        return Err(
                            "win32.contract-conflict: metadata and evidence counts disagree".into(),
                        );
                    }
                    buffer.buffer = Some(RawBuffer {
                        element: buffer_element(&buffer.typ),
                        size: RawBufferSize::ElementCountParam(*count_parameter),
                    });
                    buffer.nullable = *nullable;
                }
                FunctionEffect::UnsupportedCountUnit { .. } => {
                    return Err(
                        "flag-dependent native buffer count units require a dedicated contract"
                            .into(),
                    );
                }
                FunctionEffect::OwnedReturn { .. }
                | FunctionEffect::BorrowedReturn {}
                | FunctionEffect::OwnedOutput { .. }
                | FunctionEffect::ConsumedInput { .. }
                | FunctionEffect::MutableString { .. }
                | FunctionEffect::CallContract { .. }
                | FunctionEffect::OverlappedIo { .. }
                | FunctionEffect::Subsystem { .. }
                | FunctionEffect::SubsystemExempt {}
                | FunctionEffect::ManagedLifecycle { .. } => {}
            }
        }
        Ok(result)
    }
}

fn group<T: serde::de::DeserializeOwned>(text: &str) -> Result<Vec<T>, String> {
    let group: Group<T> =
        serde_json::from_str(text).map_err(|e| format!("win32.contract-json: {e}"))?;
    if group.schema_version != 2 {
        return Err("win32.contract-schema-version".into());
    }
    Ok(group.entries)
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'A'..=b'F').contains(&c))
}

fn native_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .enumerate()
            .all(|(i, c)| c.is_ascii_alphabetic() || c == b'_' || (i > 0 && c.is_ascii_digit()))
}

fn native_namespace(name: &str) -> bool {
    name.starts_with("Windows.Win32.") && name.split('.').all(native_identifier)
}

fn identifier(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
}

fn dll_name(dll: &str) -> bool {
    let lower = dll.to_ascii_lowercase();
    (lower.ends_with(".dll") || lower.ends_with(".drv"))
        && !dll.contains("..")
        && dll
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
}

pub(crate) fn validate_call_contract_structure(contract: &CallContract) -> Result<(), String> {
    if contract.outputs.len() > 1024 || contract.resource_effects.len() > 1024 {
        return Err("win32.contract-conflict: too many native call rules".into());
    }
    let mut outputs = BTreeSet::new();
    for rule in &contract.outputs {
        if rule.parameter > 1023 || !outputs.insert(rule.parameter) || rule.when.inputs.len() > 16 {
            return Err("win32.contract-conflict: output rule parameter or predicates".into());
        }
        for predicate in &rule.when.inputs {
            let valid = match predicate {
                InputPredicate::BitsIn {
                    parameter,
                    mask,
                    values,
                } => {
                    *parameter <= 1023
                        && *mask != 0
                        && !values.is_empty()
                        && values.len() <= 64
                        && values.iter().all(|value| value & !mask == 0)
                        && values.iter().copied().collect::<BTreeSet<_>>().len() == values.len()
                }
                InputPredicate::HandleIn { parameter, values } => {
                    *parameter <= 1023
                        && !values.is_empty()
                        && values.len() <= 64
                        && values.iter().copied().collect::<BTreeSet<_>>().len() == values.len()
                }
                InputPredicate::NullOrEmpty {
                    parameter,
                    element_width,
                } => *parameter <= 1023 && matches!(element_width, 1 | 2),
            };
            if !valid {
                return Err("win32.contract-conflict: invalid native input predicate".into());
            }
        }
        if let OutputAction::AliasInput { parameter } = rule.action
            && (parameter > 1023 || parameter == rule.parameter)
        {
            return Err("win32.contract-conflict: invalid native alias input".into());
        }
    }
    let mut resources = BTreeSet::new();
    for effect in &contract.resource_effects {
        let ResourceEffect::AddFileCompletionModes {
            handle_parameter,
            flags_parameter,
        } = *effect;
        if handle_parameter > 1023
            || flags_parameter > 1023
            || handle_parameter == flags_parameter
            || !resources.insert(handle_parameter)
        {
            return Err("win32.contract-conflict: resource state effect parameters".into());
        }
    }
    Ok(())
}

fn validate_recipe(recipe: &LayoutRecipe, depth: usize) -> Result<(), String> {
    if depth > 16
        || !native_identifier(&recipe.name)
        || recipe.fields.is_empty()
        || recipe.fields.len() > 256
    {
        return Err("win32.contract-evidence: invalid anonymous layout recipe".into());
    }
    let mut names = BTreeSet::new();
    for field in &recipe.fields {
        if !native_identifier(&field.name) || !names.insert(&field.name) {
            return Err("win32.contract-conflict: anonymous layout field".into());
        }
        match &field.typ {
            RecipeType::Record { layout } => validate_recipe(layout, depth + 1)?,
            RecipeType::Named { namespace, name }
                if !native_namespace(namespace) || !native_identifier(name) =>
            {
                return Err("win32.contract-evidence: invalid native type reference".into());
            }
            RecipeType::Scalar { .. } | RecipeType::Named { .. } => {}
        }
    }
    Ok(())
}

fn citations(values: &[Citation]) -> bool {
    values.iter().any(|citation| matches!(citation, Citation::MicrosoftLearn { url }
        if url.starts_with("https://learn.microsoft.com/") && !url.bytes().any(|b|b.is_ascii_whitespace())))
        && values.iter().all(|citation| match citation {
            Citation::MicrosoftLearn { url } => url.starts_with("https://learn.microsoft.com/"),
            Citation::SdkHeader { file } => file.ends_with(".h") && !file.contains(['/', '\\', ':']),
        })
}

impl Registry {
    pub fn builtin() -> Result<&'static Self, String> {
        static REGISTRY: OnceLock<Result<Registry, String>> = OnceLock::new();
        REGISTRY
            .get_or_init(|| Self::load(MANIFEST, SCHEMA, FILES))
            .as_ref()
            .map_err(Clone::clone)
    }

    fn load(manifest: &str, schema: &str, files: &[(&str, &str)]) -> Result<Self, String> {
        let manifest: Manifest =
            serde_json::from_str(manifest).map_err(|e| format!("win32.manifest: {e}"))?;
        if manifest.schema_version != 2
            || manifest.metadata.package != "Microsoft.Windows.SDK.Win32Metadata"
            || manifest.metadata.version != "71.0.14-preview"
            || manifest.metadata.sha256 != METADATA_SHA256
            || manifest.schema.file != "schema.json"
            || manifest.schema.sha256 != text_sha256(schema)
            || manifest.files.len() != FILES.len()
            || files.len() != FILES.len()
        {
            return Err("win32.contract-integrity: schema or metadata pin mismatch".into());
        }
        let mut seen = BTreeSet::new();
        for file in &manifest.files {
            if !seen.insert(file.file.as_str()) || !valid_hash(&file.sha256) {
                return Err("win32.contract-integrity: duplicate file or invalid digest".into());
            }
            let Some((_, bytes)) = files.iter().find(|(name, _)| *name == file.file) else {
                return Err("win32.contract-integrity: unknown data file".into());
            };
            if text_sha256(bytes) != file.sha256 {
                return Err(format!("win32.contract-integrity: {}", file.file));
            }
        }
        let data = |name: &str| {
            files
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| *v)
                .ok_or_else(|| format!("win32.contract-integrity: missing {name}"))
        };
        let mut function_entries: Vec<FunctionEntry> = group(data("function-contracts.json")?)?;
        function_entries.extend(group::<FunctionEntry>(data("subsystem-policies.json")?)?);
        let mut type_entries: Vec<TypeEntry> = group(data("native-types.json")?)?;
        type_entries.extend(group::<TypeEntry>(data("aggregate-layouts.json")?)?);
        Self::validate(
            function_entries,
            type_entries,
            group(data("policy-domains.json")?)?,
        )
    }

    fn validate(
        function_entries: Vec<FunctionEntry>,
        type_entries: Vec<TypeEntry>,
        domains: Vec<DomainPolicy>,
    ) -> Result<Self, String> {
        let mut ids = BTreeSet::new();
        let mut functions = BTreeMap::new();
        let mut types = BTreeMap::new();
        for entry in function_entries {
            let selector = &entry.selector;
            if !identifier(&entry.id)
                || !ids.insert(entry.id.clone())
                || !citations(&entry.evidence)
                || entry.contracts.is_empty()
                || !native_namespace(&selector.namespace)
                || !dll_name(&selector.dll)
                || selector.entry_point.is_empty()
                || selector.entry_point.contains('\0')
                || !native_identifier(&selector.name)
                || !native_identifier(&selector.container)
                || !valid_hash(&selector.source_fingerprint)
                || selector.architectures & !7 != 0
                || !matches!(
                    selector.calling_convention.as_str(),
                    "system" | "cdecl" | "unsupported"
                )
            {
                return Err(format!(
                    "win32.contract-evidence: invalid function {}",
                    entry.id
                ));
            }
            let mut effects = BTreeSet::new();
            for effect in &entry.contracts {
                if !effects.insert(effect.key()) {
                    return Err(format!("win32.contract-conflict: {}", entry.id));
                }
                if let FunctionEffect::CallContract { contract } = effect {
                    validate_call_contract_structure(contract)?;
                    if entry
                        .contracts
                        .iter()
                        .any(|effect| matches!(effect, FunctionEffect::OverlappedIo { .. }))
                    {
                        return Err(format!(
                            "win32.contract-conflict: asynchronous adapter call rules {}",
                            entry.id
                        ));
                    }
                }
                let positions: Vec<usize> = match effect {
                    FunctionEffect::CountedBuffer {
                        parameter,
                        count_parameter,
                        ..
                    } => vec![*parameter, *count_parameter],
                    FunctionEffect::UnsupportedCountUnit { parameter }
                    | FunctionEffect::OwnedOutput { parameter, .. }
                    | FunctionEffect::ConsumedInput { parameter, .. }
                    | FunctionEffect::MutableString { parameter } => vec![*parameter],
                    FunctionEffect::OverlappedIo {
                        file_parameter,
                        buffer_parameter,
                        count_parameter,
                        transferred_parameter,
                        overlapped_parameter,
                        ..
                    } => vec![
                        *file_parameter,
                        *buffer_parameter,
                        *count_parameter,
                        *transferred_parameter,
                        *overlapped_parameter,
                    ],
                    _ => Vec::new(),
                };
                if positions.iter().any(|p| *p > 1023)
                    || positions.iter().copied().collect::<BTreeSet<_>>().len() != positions.len()
                {
                    return Err(format!(
                        "win32.contract-conflict: parameter roles {}",
                        entry.id
                    ));
                }
                if matches!(
                    effect,
                    FunctionEffect::OwnedReturn {
                        cleanup: Cleanup::None
                    } | FunctionEffect::OwnedOutput {
                        cleanup: Cleanup::None,
                        ..
                    } | FunctionEffect::ConsumedInput {
                        cleanup: Cleanup::None,
                        ..
                    }
                ) {
                    return Err(format!(
                        "win32.contract-evidence: missing cleanup {}",
                        entry.id
                    ));
                }
            }
            let key = (
                selector.namespace.clone(),
                selector.container.clone(),
                selector.name.clone(),
            );
            if functions.insert(key, entry).is_some() {
                return Err("win32.contract-conflict: duplicate function selector".into());
            }
        }
        for entry in type_entries {
            if !identifier(&entry.id)
                || !ids.insert(entry.id.clone())
                || !citations(&entry.evidence)
                || !native_namespace(&entry.selector.namespace)
                || !valid_hash(&entry.selector.source_fingerprint)
                || (!native_identifier(&entry.selector.name))
                || (entry.meaning.is_none() && entry.status.is_none() && entry.aggregate.is_none())
            {
                return Err(format!(
                    "win32.contract-evidence: invalid type {}",
                    entry.id
                ));
            }
            if let Some(aggregate) = &entry.aggregate {
                if entry.meaning.is_some()
                    || entry.status.is_some()
                    || aggregate
                        .size_field
                        .as_deref()
                        .is_some_and(|s| !native_identifier(s))
                {
                    return Err(format!(
                        "win32.contract-conflict: aggregate meaning {}",
                        entry.id
                    ));
                }
                let mut fields = BTreeSet::new();
                for field in &aggregate.fields {
                    if !native_identifier(&field.name)
                        || !fields.insert(&field.name)
                        || matches!(
                            field.contract,
                            FieldContract::OwnedHandle {
                                cleanup: Cleanup::None
                            }
                        )
                    {
                        return Err(format!(
                            "win32.contract-conflict: aggregate fields {}",
                            entry.id
                        ));
                    }
                }
                let mut names = BTreeSet::new();
                for recipe in &aggregate.anonymous_layouts {
                    validate_recipe(recipe, 0)?;
                    if !names.insert(&recipe.name) {
                        return Err(format!(
                            "win32.contract-conflict: duplicate layout {}",
                            entry.id
                        ));
                    }
                }
            }
            if types
                .insert(
                    (
                        entry.selector.namespace.clone(),
                        entry.selector.name.clone(),
                    ),
                    entry,
                )
                .is_some()
            {
                return Err("win32.contract-conflict: duplicate type selector".into());
            }
        }
        let mut seen = BTreeSet::new();
        for domain in &domains {
            let key = match domain {
                DomainPolicy::Subsystem { namespace, .. } if native_namespace(namespace) => {
                    format!("namespace:{namespace}")
                }
                DomainPolicy::ProviderLifecycle { dll } if dll_name(dll) => {
                    format!("dll:{}", dll.to_ascii_lowercase())
                }
                _ => return Err("win32.contract-evidence: invalid policy domain".into()),
            };
            if !seen.insert(key) {
                return Err("win32.contract-conflict: duplicate policy domain".into());
            }
        }
        Ok(Self {
            functions,
            types,
            domains,
        })
    }

    pub fn type_entry(&self, namespace: &str, name: &str) -> Option<&TypeEntry> {
        self.types.get(&(namespace.to_string(), name.to_string()))
    }

    pub fn function_entry(&self, raw: &RawFunction) -> Option<&FunctionEntry> {
        self.functions.get(&(
            raw.namespace.clone(),
            raw.container.clone(),
            raw.name.clone(),
        ))
    }

    pub fn function_policy(&self, raw: &RawFunction) -> Result<FunctionPolicy<'_>, String> {
        if let Some(evidence) = &raw.evidence
            && evidence.shape_fingerprint != crate::win32_metadata::function_shape_fingerprint(raw)
        {
            return Err("win32.signature-drift: raw facts changed after parsing".into());
        }
        let entry = self.function_entry(raw);
        if let Some(entry) = entry {
            let proof = raw.evidence.as_ref().ok_or("win32.missing-raw-evidence")?;
            if proof.metadata_sha256 != METADATA_SHA256 {
                return Err("win32.metadata-hash-mismatch".into());
            }
            if proof.selector != entry.selector {
                return Err(format!("win32.signature-drift: {}", entry.id));
            }
            use crate::win32_metadata::{RawBaseType, RawDirection, RawNamedKind};
            for effect in &entry.contracts {
                let valid = match effect {
                    FunctionEffect::OwnedReturn { .. } | FunctionEffect::BorrowedReturn {} => {
                        raw.return_type.pointer_depth == 0
                            && matches!(
                                raw.return_type.base,
                                RawBaseType::Named {
                                    kind: RawNamedKind::Handle { .. },
                                    ..
                                }
                            )
                    }
                    FunctionEffect::OwnedOutput { parameter, .. } => {
                        raw.parameters.get(*parameter).is_some_and(|p| {
                            p.direction == RawDirection::Out
                                && p.typ.pointer_depth == 1
                                && matches!(
                                    p.typ.base,
                                    RawBaseType::Named {
                                        kind: RawNamedKind::Handle { .. },
                                        ..
                                    }
                                )
                        })
                    }
                    FunctionEffect::ConsumedInput { parameter, .. } => {
                        raw.parameters.get(*parameter).is_some_and(|p| {
                            p.direction == RawDirection::In
                                && p.typ.pointer_depth == 0
                                && matches!(
                                    p.typ.base,
                                    RawBaseType::Named {
                                        kind: RawNamedKind::Handle { .. },
                                        ..
                                    }
                                )
                        })
                    }
                    FunctionEffect::MutableString { parameter } => {
                        raw.parameters.get(*parameter).is_some_and(|p| {
                            p.direction != RawDirection::Out
                                && p.typ.pointer_depth == 0
                                && matches!(
                                    p.typ.base,
                                    RawBaseType::Named {
                                        kind: RawNamedKind::StringPointer { .. },
                                        ..
                                    }
                                )
                        })
                    }
                    FunctionEffect::CountedBuffer {
                        parameter,
                        count_parameter,
                        ..
                    } => {
                        *parameter < raw.parameters.len() && *count_parameter < raw.parameters.len()
                    }
                    FunctionEffect::UnsupportedCountUnit { parameter } => {
                        *parameter < raw.parameters.len()
                    }
                    FunctionEffect::CallContract { contract } => {
                        validate_call_contract_structure(contract)?;
                        contract.outputs.iter().all(|rule| {
                            raw.parameters.get(rule.parameter).is_some_and(|parameter| {
                                parameter.direction != RawDirection::In
                                    && parameter.typ.pointer_depth > 0
                                    && parameter.buffer.is_none()
                            })
                        })
                    }
                    FunctionEffect::OverlappedIo {
                        file_parameter,
                        buffer_parameter,
                        count_parameter,
                        transferred_parameter,
                        overlapped_parameter,
                        ..
                    } => [
                        file_parameter,
                        buffer_parameter,
                        count_parameter,
                        transferred_parameter,
                        overlapped_parameter,
                    ]
                    .into_iter()
                    .all(|p| *p < raw.parameters.len()),
                    FunctionEffect::Subsystem { .. }
                    | FunctionEffect::SubsystemExempt {}
                    | FunctionEffect::ManagedLifecycle { .. } => true,
                };
                if !valid {
                    return Err(format!("win32.contract-shape-conflict: {}", entry.id));
                }
            }
        } else if self.functions.values().any(|entry| {
            entry.selector.dll.eq_ignore_ascii_case(&raw.dll)
                && entry.selector.entry_point == raw.entry_point
        }) {
            return Err(
                "win32.signature-drift: known export has a different metadata identity".into(),
            );
        }

        let mut subsystem = None;
        for effect in entry.into_iter().flat_map(|entry| entry.contracts.iter()) {
            match effect {
                FunctionEffect::Subsystem { subsystem: value } => subsystem = Some(*value),
                FunctionEffect::ManagedLifecycle { subsystem } => {
                    return Err(format!(
                        "{subsystem:?} lifecycle is managed by the generated initialization adapter"
                    ));
                }
                _ => {}
            }
        }
        for domain in &self.domains {
            match domain {
                DomainPolicy::Subsystem {
                    namespace,
                    subsystem: required,
                } if *namespace == raw.namespace => {
                    let exempt = entry.is_some_and(|entry| {
                        entry
                            .contracts
                            .iter()
                            .any(|e| matches!(e, FunctionEffect::SubsystemExempt {}))
                    });
                    if !exempt && subsystem != Some(*required) {
                        return Err("win32.missing-subsystem-evidence".into());
                    }
                }
                DomainPolicy::ProviderLifecycle { dll } if dll.eq_ignore_ascii_case(&raw.dll) => {
                    return Err(
                        "MAPI exports require a separately classified provider lifecycle contract"
                            .into(),
                    );
                }
                DomainPolicy::Subsystem { .. } | DomainPolicy::ProviderLifecycle { .. } => {}
            }
        }
        Ok(FunctionPolicy { entry, subsystem })
    }

    pub fn layout_contract<'a>(
        &'a self,
        raw: &crate::win32_metadata::RawNativeLayoutSet,
    ) -> Result<Option<&'a AggregateContract>, String> {
        let Some(proof) = &raw.evidence else {
            return Ok(None);
        };
        if proof.metadata_sha256 != METADATA_SHA256
            || proof.shape_fingerprint != crate::win32_metadata::layout_shape_fingerprint(raw)
        {
            return Err("win32.type-signature-drift: native layout facts changed".into());
        }
        let entry = self
            .type_entry(&proof.selector.namespace, &proof.selector.name)
            .ok_or("win32.missing-type-evidence")?;
        if entry.selector != proof.selector {
            return Err("win32.type-signature-drift".into());
        }
        Ok(entry.aggregate.as_ref())
    }
}

#[cfg(test)]
#[path = "codegen/win32/contract_tests.rs"]
mod tests;
