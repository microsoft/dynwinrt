// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use dynwinrt::{ArrayData, MetadataTable, WinRTValue};
use windows::Foundation::{IStringable, IUriRuntimeClass, Uri};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows_core::{Interface, h};

struct Apartment;

impl Apartment {
    fn enter() -> windows_core::Result<Self> {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

// A separate test executable keeps stock activation inside one apartment.
#[test]
fn stock_uri_array_ownership_and_typed_qi_share_one_apartment() -> windows_core::Result<()> {
    let _apartment = Apartment::enter()?;
    let table = MetadataTable::new();

    {
        let uri = Uri::CreateUri(h!("https://example.com/first"))?;
        let default: IUriRuntimeClass = uri.cast()?;
        let source = WinRTValue::Object(default.cast()?);
        let mislabeled = ArrayData::from_values(table.i32_type(), &[source.clone()]);
        assert!(mislabeled.contains_com_references());
        assert!(WinRTValue::Array(mislabeled).contains_com_references());
        assert!(ArrayData::try_from_values(table.i32_type(), &[source.clone()]).is_err());
        let inner = WinRTValue::Array(ArrayData::from_values(table.object(), &[source]));
        assert!(ArrayData::try_from_values(table.array(&table.object()), &[inner]).is_err());
    }

    {
        let uri = Uri::CreateUri(h!("https://example.com/second"))?;
        let default: IUriRuntimeClass = uri.cast()?;
        let stringable: IStringable = uri.cast()?;
        assert_ne!(default.as_raw(), stringable.as_raw());
        let source = WinRTValue::Object(default.cast()?);
        let checked = ArrayData::try_from_values(
            table.interface(IStringable::IID),
            &[source.clone(), WinRTValue::Null],
        )?;
        assert_eq!(
            checked.get(0).as_object().unwrap().as_raw(),
            stringable.as_raw()
        );
        assert!(checked.get(1).is_null_object());
        assert!(source.as_object().is_some());
    }
    Ok(())
}
