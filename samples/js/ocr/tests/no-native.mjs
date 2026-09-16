// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

process.dlopen = () => {
    throw new Error('Native addons are forbidden in the OCR pure tests.')
}
