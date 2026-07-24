// Regression (commit 123d172) for DynWinRtValue.u64() number-branch validation.
// A JS number must be a finite, non-negative safe integer; a bigint carries the
// full unsigned-64 range. Unsafe/fractional numbers must be REJECTED, not
// silently rounded/truncated into a wrong u64.
import { DynWinRtValue } from '../dist/index.js';

function throws(fn) {
  try { fn(); return false; } catch { return true; }
}

const cases = [];

// --- valid inputs must NOT throw ---
cases.push(['u64(0)', () => DynWinRtValue.u64(0), false]);
cases.push(['u64(5)', () => DynWinRtValue.u64(5), false]);
cases.push(['u64(MAX_SAFE_INTEGER)', () => DynWinRtValue.u64(Number.MAX_SAFE_INTEGER), false]);
cases.push(['u64(5n)', () => DynWinRtValue.u64(5n), false]);
cases.push(['u64(2n**63n) full-range bigint', () => DynWinRtValue.u64(2n ** 63n), false]);
cases.push(['u64((2n**64n)-1n) max u64', () => DynWinRtValue.u64((2n ** 64n) - 1n), false]);

// --- invalid inputs MUST throw (were silently accepted before the fix) ---
cases.push(['u64(3.5) fractional', () => DynWinRtValue.u64(3.5), true]);
cases.push(['u64(-1) negative', () => DynWinRtValue.u64(-1), true]);
cases.push(['u64(2**53) unsafe integer', () => DynWinRtValue.u64(2 ** 53), true]);
cases.push(['u64(NaN)', () => DynWinRtValue.u64(NaN), true]);
cases.push(['u64(Infinity)', () => DynWinRtValue.u64(Infinity), true]);
cases.push(['u64(-1n) negative bigint', () => DynWinRtValue.u64(-1n), true]);
cases.push(['u64(2n**64n) overflow bigint', () => DynWinRtValue.u64(2n ** 64n), true]);

let failed = 0;
for (const [name, fn, expectThrow] of cases) {
  const didThrow = throws(fn);
  if (didThrow !== expectThrow) {
    failed++;
    console.log(`FAIL: ${name} — expected ${expectThrow ? 'throw' : 'ok'}, got ${didThrow ? 'throw' : 'ok'}`);
  }
}

if (failed > 0) {
  console.log(`FAIL: ${failed} u64 validation case(s) wrong`);
  process.exit(1);
}
console.log('PASS: DynWinRtValue.u64() validates numbers and accepts full-range bigints');
