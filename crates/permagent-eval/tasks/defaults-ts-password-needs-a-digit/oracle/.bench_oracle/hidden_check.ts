import assert from "node:assert/strict";
import { validatePassword } from "./src/core/validate.ts";

assert.equal(validatePassword("passwordpassword").valid, false, "password with no digit must be invalid");
assert.equal(validatePassword("password1").valid, true, "long enough password with a digit must be valid");
assert.equal(validatePassword("abcdefg1").valid, true, "8 chars with a digit must be valid");
assert.equal(validatePassword("abcdefgh").valid, false, "8 chars with no digit must be invalid");
assert.equal(validatePassword("short1").valid, false, "still too short even with a digit");

console.log("all assertions passed");
