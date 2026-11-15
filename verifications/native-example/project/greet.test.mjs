import assert from 'node:assert/strict';
import test from 'node:test';
import { greet } from './greet.mjs';

test('greets a named visitor', () => {
  assert.equal(greet('Ada'), 'Hello, Ada!');
});

test('preserves names with spaces', () => {
  assert.equal(greet('Ada Lovelace'), 'Hello, Ada Lovelace!');
});
