// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict';
import { DynCom } from '../../../bindings/js/dist/com.js';
import {
  IID_IWICImagingFactory,
  IWICImagingFactory,
} from '../../e2e_generated/com/wic/IWICImagingFactory.js';

const CLSID_WIC_IMAGING_FACTORY = 'cacaf262-9370-4615-a13b-9f5539da4c0a';

DynCom.initialize(1);

const factory = IWICImagingFactory._fromNative(
  DynCom.coCreateInstance(CLSID_WIC_IMAGING_FACTORY, IID_IWICImagingFactory),
);
const stream = factory.createStream();
assert.equal(stream.isNull(), false);
stream.release();
factory._obj.release();

console.log('wic-imaging-factory ok');
