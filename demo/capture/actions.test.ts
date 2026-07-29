import { test } from "node:test";
import assert from "node:assert/strict";
import { moveCmd, clickCmd, keyCmd, typeCmd, scrollCmd, shQuote } from "./actions.ts";

test("shQuote wraps in single quotes and escapes embedded single quotes", () => {
  assert.equal(shQuote("hello"), "'hello'");
  assert.equal(shQuote("it's"), "'it'\\''s'");
});

test("command builders emit the expected ydotool strings", () => {
  assert.equal(moveCmd(480, 270), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool mousemove --absolute -- 480 270");
  assert.equal(clickCmd(), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool click 0xC0");
  assert.equal(keyCmd("1:1 1:0"), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool key 1:1 1:0");
  assert.equal(scrollCmd(-15), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool mousemove --wheel -- 0 -15");
});

test("typeCmd single-quotes the payload safely", () => {
  assert.equal(typeCmd("hi there"), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool type -- 'hi there'");
  assert.equal(typeCmd("it's"), "YDOTOOL_SOCKET=/run/ydotoold/socket ydotool type -- 'it'\\''s'");
});
