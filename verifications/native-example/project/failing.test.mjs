// Deliberately wrong expectation: this control must fail in the real framework.
import assert from 'node:assert/strict';
import test from 'node:test';
import { greet } from './greet.mjs';

test('negative control expects the wrong greeting', () => {
  assert.equal(greet('Ada'), 'Goodbye, Ada!');
});
