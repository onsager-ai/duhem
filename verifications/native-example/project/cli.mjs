import { greet } from './greet.mjs';

console.log(greet(process.argv[2] ?? 'world'));
