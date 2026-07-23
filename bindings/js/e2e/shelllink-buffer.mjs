import assert from 'node:assert/strict';
import { DynWinRtValue } from '../dist/index.js';
import { IShellLinkW, IID_IShellLinkW } from './IShellLinkW.js';
import { SHOW_WINDOW_CMD } from './SHOW_WINDOW_CMD.js';

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

// Proves the u16 arg-wrapper codegen fix: setHotkey takes a [in] u16 (WORD).
// Before the fix, codegen emitted the non-existent DynWinRtValue.u16Value(...)
// and this call threw a TypeError. It must now complete without throwing.
const expectedHotkey = 0x0341; // Ctrl+Alt+'A'
assert.doesNotThrow(() => link.setHotkey(expectedHotkey));
assert.equal(link.getHotkey(), expectedHotkey);

link.setShowCmd(SHOW_WINDOW_CMD.SW_SHOWMAXIMIZED);
assert.equal(link.getShowCmd(), SHOW_WINDOW_CMD.SW_SHOWMAXIMIZED);

console.log('shelllink-buffer ok');
