// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from "node:assert/strict";
import {
  DynComDispatchParams,
  initializeCom,
} from "../../../../bindings/js/dist/com.js";
import { DynComUnsafe } from "../../../../bindings/js/dist/com-unsafe.js";
import {
  IDispatch,
  IID_IDispatch,
} from "../../e2e_generated/com/automation/com/IDispatch.js";
import { IEnumVARIANT } from "../../e2e_generated/com/automation/com/IEnumVARIANT.js";
import { IStream } from "../../e2e_generated/com/stream/com/IStream.js";

const CLSID_SHELL_APPLICATION = "13709620-c279-11ce-a49e-444553540000";
const DISPATCH_PROPERTYGET = 2;
const IID_NULL = "00000000-0000-0000-0000-000000000000";

initializeCom(1);

const raw = DynComUnsafe.coCreateInstance(
  CLSID_SHELL_APPLICATION,
  IID_IDispatch,
);
const shell = IDispatch._fromNative(raw);
raw.release();

function invokeMember(dispatch, dispid, flags, name) {
  const params = new DynComDispatchParams([]);
  try {
    const result = dispatch.invoke(dispid, IID_NULL, 0, flags, params, {
      result: true,
      excepInfo: true,
      argErr: true,
    }).result;
    assert.ok(result, `${name} did not return a VARIANT`);
    return result;
  } finally {
    params.release();
  }
}

function namedMember(dispatch, name, flags = DISPATCH_PROPERTYGET) {
  const [dispid] = dispatch.getIDsOfNames(IID_NULL, [name], 0);
  return invokeMember(dispatch, dispid, flags, name);
}

try {
  assert.throws(
    () => IStream._fromNative(shell._obj),
    /QueryInterface failed/,
  );
  assert.ok(shell.getTypeInfoCount() >= 1);

  const application = namedMember(shell, "Application");
  try {
    assert.equal(application.kind, "dispatch");
    const object = application.toInterface();
    assert.ok(object && !object.isNull());
    object.release();
  } finally {
    application.release();
  }

  const windows = namedMember(shell, "Windows", 1);
  let windowsDispatch;
  try {
    const object = windows.toInterface();
    assert.ok(object);
    try {
      windowsDispatch = IDispatch._fromNative(object);
    } finally {
      object.release();
    }
  } finally {
    windows.release();
  }
  try {
    const enumeratorValue = invokeMember(windowsDispatch, -4, 3, "_NewEnum");
    let enumerator;
    try {
      const object = enumeratorValue.toInterface();
      assert.ok(object);
      try {
        enumerator = IEnumVARIANT._fromNative(object);
      } finally {
        object.release();
      }
    } finally {
      enumeratorValue.release();
    }
    try {
      const values = enumerator.next(1);
      for (const value of values) value.release();
      enumerator.reset();
    } finally {
      enumerator.release();
    }
  } finally {
    windowsDispatch.release();
  }

  const params = new DynComDispatchParams([]);
  try {
    assert.throws(
      () => shell.invoke(0x7fffffff, IID_NULL, 0, DISPATCH_PROPERTYGET, params),
      (error) => typeof error.hresult === "number" && error.hresult < 0,
    );
  } finally {
    params.release();
  }
} finally {
  shell.release();
}

console.log("automation-dispatch ok");
