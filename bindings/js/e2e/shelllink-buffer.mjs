import assert from 'node:assert/strict';
import { DynWinRtValue } from '../dist/index.js';
import { IShellLinkW, IID_IShellLinkW } from './IShellLinkW.js';

const CLSID_SHELL_LINK = '00021401-0000-0000-c000-000000000046';

function wide(text) {
  return Buffer.from(`${text}\0`, 'utf16le');
}

const link = IShellLinkW._fromNative(
  DynWinRtValue.coCreateInstance(CLSID_SHELL_LINK, IID_IShellLinkW),
);

const expectedPath = 'C:\\Windows\\explorer.exe';
link.setPath(wide(expectedPath));
assert.equal(link.getPath(260, 0).toLowerCase(), expectedPath.toLowerCase());

const expectedDescription = 'dynwinrt shelllink buffer';
link.setDescription(wide(expectedDescription));
assert.equal(link.getDescription(), expectedDescription);

console.log('shelllink-buffer ok');
