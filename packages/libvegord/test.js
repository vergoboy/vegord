/**
 * @type {typeof import(".")}
 */
const libVegcord = require(".");
const test = require("node:test");
const assert = require("node:assert/strict");

test("getAccentColor should return a number", () => {
    const color = libVegcord.getAccentColor();
    assert.strictEqual(typeof color, "number");
});

test("updateUnityLauncherCount should return true (success)", () => {
    assert.strictEqual(libVegcord.updateUnityLauncherCount(5), true);
    assert.strictEqual(libVegcord.updateUnityLauncherCount(0), true);
    assert.strictEqual(libVegcord.updateUnityLauncherCount(10), true);
});

test("requestBackground should return true (success)", () => {
    assert.strictEqual(libVegcord.requestBackground(true, ["bash"]), true);
    assert.strictEqual(libVegcord.requestBackground(false, []), true);
});
