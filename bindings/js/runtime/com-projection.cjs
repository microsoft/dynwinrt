// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

'use strict'

const native = require('./index.js')
const projections = new WeakMap()

function registerProjection(type, iid) {
  if (typeof type !== 'function' || !(iid instanceof native.WinGuid)) {
    throw new TypeError('A COM projection requires a generated class and an exact IID')
  }
  if (projections.has(type)) {
    throw new TypeError('The COM projection is already registered')
  }
  projections.set(type, Object.freeze({ iid, prototype: type.prototype }))
}

function descriptor(type) {
  const projection = projections.get(type)
  if (!projection) {
    throw new TypeError('Expected a registered generated safe COM interface type; regenerate the bindings')
  }
  return projection
}

function projectionIid(type) {
  return descriptor(type).iid
}

function projectAs(value, type) {
  const projection = descriptor(type)
  const source = value instanceof native.DynWinRtValue
    ? value
    : value !== null && typeof value === 'object' ? value._obj : undefined
  if (!(source instanceof native.DynWinRtValue)) {
    throw new TypeError('projectAs requires a managed COM value or generated wrapper')
  }
  // cast validates managed COM ownership and thread affinity before querying the IID.
  const cast = source.cast(projection.iid)
  try {
    native.DynCom.bindComObject(cast)
    return Object.assign(Object.create(projection.prototype), { _obj: cast })
  } catch (error) {
    cast.release()
    throw error
  }
}

module.exports = { projectAs, registerProjection, projectionIid }
