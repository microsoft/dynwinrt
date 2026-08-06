// Regression for memory-safety fix #2: DynCom.pointer() must REJECT
// DynWinRtValue inputs. Borrowing an owned COM object's raw pointer here would
// make it indistinguishable from an owned raw pointer to adoptComPointer(),
// which can double-release the original wrapper's COM object.
import { DynCom, WinGuid } from '../../../bindings/js/dist/com-unsafe.js';

// iidPointer() returns a DynWinRtValue — a representative value input.
const someValue = DynCom.iidPointer(WinGuid.parse('a5caee9b-8708-49d1-8d36-67d25a8da00c'));

let rejected = false;
try {
  DynCom.pointer(someValue);
} catch (e) {
  rejected = String(e).includes('not accepted');
}

if (!rejected) {
  console.log('FAIL: DynCom.pointer() accepted a DynWinRtValue input (double-release hazard)');
  process.exit(1);
}
console.log('PASS: DynCom.pointer() rejects DynWinRtValue inputs');
