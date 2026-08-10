/**
 * @type {typeof import(".")}
 */
const libvegord = require(".");
const test = require("node:test");
const assert = require("node:assert/strict");

test("getAccentColor should return a number", () => {
    const color = libvegord.getAccentColor();
    assert.strictEqual(typeof color, "number");
});

test("updateUnityLauncherCount should return true (success)", () => {
    assert.strictEqual(libvegord.updateUnityLauncherCount(5), true);
    assert.strictEqual(libvegord.updateUnityLauncherCount(0), true);
    assert.strictEqual(libvegord.updateUnityLauncherCount(10), true);
});

test("requestBackground should return true (success)", () => {
    assert.strictEqual(libvegord.requestBackground(true, ["bash"]), true);
    assert.strictEqual(libvegord.requestBackground(false, []), true);
});
