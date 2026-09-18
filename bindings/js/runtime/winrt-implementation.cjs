// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
'use strict'

const { DynWinRtImplementation, DynWinRtInterfacePlan } = require('./index.js')

class DynWinRtImplementationHandle {
  #owner
  #project
  #releaseView
  #value
  #released = false
  #projecting = false

  constructor(owner, project, releaseView) {
    if (!(owner instanceof DynWinRtImplementation) || typeof project !== 'function' || typeof releaseView !== 'function') {
      throw new TypeError('A WinRT implementation handle requires an owner and typed view projection')
    }
    this.#owner = owner
    this.#project = project
    this.#releaseView = releaseView
  }

  static create(type, handlers, additional, releaseView) {
    let descriptors
    if (additional.length === 1 && additional[0] && Object.hasOwn(additional[0], 'interfaces')) {
      const options = additional[0]
      if (Object.keys(options).some(key => key !== 'interfaces') || !Array.isArray(options.interfaces)) {
        throw new TypeError('WinRT implementation options require an interfaces array')
      }
      descriptors = options.interfaces.map(entry => {
        if (!Array.isArray(entry) || entry.length !== 2 ||
            typeof entry[0]?.implementation !== 'function' || typeof entry[0]?.fromImplementation !== 'function') {
          throw new TypeError('Each interface entry must be [generatedInterface, handlers]')
        }
        return entry[0].implementation(entry[1])
      })
    } else {
      descriptors = additional
    }
    descriptors = [type.implementation(handlers), ...descriptors]
    for (const descriptor of descriptors) {
      if (!(descriptor?.plan instanceof DynWinRtInterfacePlan) || typeof descriptor.dispatch !== 'function') {
        throw new TypeError('Invalid WinRT implementation descriptor')
      }
    }
    const dispatchers = descriptors.map(descriptor => descriptor.dispatch)
    const owner = DynWinRtImplementation.create(descriptors.map(descriptor => descriptor.plan), (index, slot, args) => {
      if (!Number.isInteger(index) || index < 0 || index >= dispatchers.length) {
        throw new RangeError('Unknown implementation interface index')
      }
      return dispatchers[index](slot, args)
    })
    return new DynWinRtImplementationHandle(owner, value => type.fromImplementation(value), releaseView)
  }

  get value() {
    if (this.#released || this.#owner.isClosed) throw new Error('WinRT implementation handle is closed or released')
    if (this.#value === undefined) {
      if (this.#projecting) throw new Error('WinRT implementation primary view creation is reentrant')
      this.#projecting = true
      try {
        const value = this.#project(this.#owner)
        if (this.#released || this.#owner.isClosed) {
          this.#releaseView(value)
          throw new Error('WinRT implementation was released during primary view creation')
        }
        this.#value = value
      } catch (error) {
        this.dispose()
        throw error
      } finally {
        this.#projecting = false
      }
    }
    return this.#value
  }

  toValue() {
    if (this.#released) throw new Error('WinRT implementation handle is released')
    return this.#owner.toValue()
  }

  release() {
    if (this.#released) return
    const value = this.#value
    this.#released = true
    this.#value = undefined
    try {
      if (value !== undefined) this.#releaseView(value)
    } finally {
      this.#owner.release()
    }
  }

  disconnect() { this.#owner.disconnect() }
  dispose() {
    this.disconnect()
    this.release()
  }
  get isClosed() { return this.#owner.isClosed }
  takeError() { return this.#owner.takeError() }
}

module.exports.DynWinRtImplementationHandle = DynWinRtImplementationHandle
